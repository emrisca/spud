//! Which programs are playing into each virtual input.
//!
//! A virtual input is the capture side of a cable; programs reach it by picking
//! the cable's *render* side in Windows sound settings. So the question "what is
//! playing into strip 5" is answered by asking Windows which audio sessions live
//! on the render endpoint that pairs with it.
//!
//! This is not a parameter of the mixer and cannot be: it is Windows' own
//! bookkeeping. Every program that renders to an endpoint owns a session there,
//! and WASAPI exposes the process, its volume and its mute. A row exists exactly
//! as long as its session does — quit the program and it is gone on the next
//! poll, with nothing to clean up.

use crate::audio::cable_key;
use crate::mixer::STRIPS;
use crate::state::FIRST_VIRTUAL;
use serde::Serialize;
use windows::core::{Interface, PROPVARIANT, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{
    eRender, AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceCollection,
    IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

#[derive(Serialize, Clone)]
pub struct AppSession {
    pub pid: u32,
    pub name: String,
    pub volume: f32,
    pub mute: bool,
    /// True while the program is actually producing sound.
    pub playing: bool,
}

#[derive(Serialize, Clone)]
pub struct VirtualInput {
    /// Strip index this endpoint feeds: 5, 6 or 7.
    pub strip: usize,
    pub endpoint: String,
    pub apps: Vec<AppSession>,
}

/// COM is per-thread and Tauri calls commands from a pool, so each call states
/// its apartment. `S_FALSE` (already initialised) is success, and a thread left
/// in another mode is not a reason to fail.
fn init_com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn wide(value: &[u16]) -> String {
    let end = value.iter().position(|&c| c == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}

fn device_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store: IPropertyStore = device.OpenPropertyStore(STGM_READ).ok()?;
        let value: PROPVARIANT = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let text: PWSTR = PropVariantToStringAlloc(&value).ok()?;
        let name = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0 as *const _));
        Some(name)
    }
}

/// The exe stem for a process id, which is what a person recognises in the list.
fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = vec![0u16; 1024];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size);
        let _ = CloseHandle(handle);
        result.ok()?;
        let full = wide(&buffer);
        let file = full.rsplit(['\\', '/']).next()?.to_string();
        Some(file.trim_end_matches(".exe").to_string())
    }
}

/// Which strip a render endpoint feeds.
///
/// The wiring decides, not a fixed table: a program playing into `CABLE-A Input`
/// is heard by whichever strip captures `CABLE-A Output`. That keeps the app
/// list honest when a cable has been moved to a different strip by hand.
fn strip_of(endpoint: &str, strips: &[String; STRIPS]) -> Option<usize> {
    let key = cable_key(endpoint)?;
    (FIRST_VIRTUAL..STRIPS)
        .find(|&strip| cable_key(&strips[strip]).as_deref() == Some(key.as_str()))
}

fn sessions_on(device: &IMMDevice) -> Vec<AppSession> {
    let mut out = Vec::new();
    unsafe {
        let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
            return out;
        };
        let Ok(enumerator) = manager.GetSessionEnumerator() else {
            return out;
        };
        let Ok(count) = enumerator.GetCount() else {
            return out;
        };

        for index in 0..count {
            let Ok(control) = enumerator.GetSession(index) else {
                continue;
            };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            let state = control2.GetState().unwrap_or(AudioSessionStateExpired);
            if state == AudioSessionStateExpired {
                continue;
            }
            let pid = control2.GetProcessId().unwrap_or(0);
            let volume = control
                .cast::<ISimpleAudioVolume>()
                .ok()
                .and_then(|simple| simple.GetMasterVolume().ok())
                .unwrap_or(0.0);
            let mute = control
                .cast::<ISimpleAudioVolume>()
                .ok()
                .and_then(|simple| simple.GetMute().ok())
                .map(|flag| flag.as_bool())
                .unwrap_or(false);

            if pid == 0 {
                // The system-sounds session has no process behind it, and the
                // console lists programs, so it is not a row.
                continue;
            }
            let name = {
                match process_name(pid) {
                    Some(name) if !name.is_empty() => name,
                    _ => control
                        .GetDisplayName()
                        .ok()
                        .and_then(|text| text.to_string().ok())
                        .filter(|text| !text.is_empty())
                        .unwrap_or_else(|| format!("pid {pid}")),
                }
            };

            out.push(AppSession {
                pid,
                name,
                volume,
                mute,
                playing: state == AudioSessionStateActive,
            });
        }
    }

    out.sort_by(|a, b| b.playing.cmp(&a.playing).then_with(|| a.name.cmp(&b.name)));
    out.dedup_by(|a, b| a.pid == b.pid && a.name == b.name);
    out
}

fn render_endpoints() -> Vec<(IMMDevice, String)> {
    let mut found = Vec::new();
    unsafe {
        let Ok(enumerator): Result<IMMDeviceEnumerator, _> =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return found;
        };
        let Ok(collection): Result<IMMDeviceCollection, _> =
            enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
        else {
            return found;
        };
        let Ok(count) = collection.GetCount() else {
            return found;
        };
        for index in 0..count {
            if let Ok(device) = collection.Item(index) {
                if let Some(name) = device_name(&device) {
                    found.push((device, name));
                }
            }
        }
    }
    found
}

/// One entry per virtual input that is currently wired to a cable.
pub fn virtual_inputs(strips: &[String; STRIPS]) -> Vec<VirtualInput> {
    init_com();
    let mut out: Vec<VirtualInput> = render_endpoints()
        .into_iter()
        .filter_map(|(device, name)| {
            let strip = strip_of(&name, strips)?;
            Some(VirtualInput {
                strip,
                endpoint: name,
                apps: sessions_on(&device),
            })
        })
        .collect();
    out.sort_by_key(|input| input.strip);
    out.dedup_by_key(|input| input.strip);
    out
}

/// Set volume and/or mute for every session belonging to `pid`.
pub fn set_app(pid: u32, volume: Option<f32>, mute: Option<bool>) -> bool {
    init_com();
    let mut touched = false;
    for (device, name) in render_endpoints() {
        if cable_key(&name).is_none() {
            continue;
        }
        unsafe {
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(enumerator) = manager.GetSessionEnumerator() else {
                continue;
            };
            let Ok(count) = enumerator.GetCount() else {
                continue;
            };
            for index in 0..count {
                let Ok(control) = enumerator.GetSession(index) else {
                    continue;
                };
                let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                if control2.GetProcessId().unwrap_or(u32::MAX) != pid {
                    continue;
                }
                let Ok(simple) = control.cast::<ISimpleAudioVolume>() else {
                    continue;
                };
                if let Some(level) = volume {
                    if simple.SetMasterVolume(level.clamp(0.0, 1.0), std::ptr::null()).is_ok() {
                        touched = true;
                    }
                }
                if let Some(flag) = mute {
                    if simple.SetMute(flag, std::ptr::null()).is_ok() {
                        touched = true;
                    }
                }
            }
        }
    }
    touched
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prints what Windows reports. On a machine with programs playing into a
    /// cable this is the list the console will draw; a machine without VB-CABLE
    /// installed reports nothing, which is the honest answer.
    #[test]
    fn finds_the_virtual_endpoints_the_wiring_names() {
        let config = crate::audio::auto_config();
        let inputs = virtual_inputs(&config.strip_device);
        for input in &inputs {
            println!("strip {} -> {} ({} app(s))", input.strip, input.endpoint, input.apps.len());
            for app in &input.apps {
                println!("   {} pid={} vol={:.2} mute={} playing={}", app.name, app.pid, app.volume, app.mute, app.playing);
            }
        }
        for input in &inputs {
            // A row exists only because a virtual strip captures that cable's
            // capture side, so the two names must belong to one cable.
            assert!(input.strip >= FIRST_VIRTUAL);
            assert!(crate::audio::same_cable(&input.endpoint, &config.strip_device[input.strip]));
        }
        if inputs.is_empty() {
            eprintln!("no virtual cable wired here — virtual inputs stay unassigned");
        }
    }
}
