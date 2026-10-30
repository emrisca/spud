//! Launch at sign-in.
//!
//! Windows' per-user `Run` key is the whole mechanism: one string value naming
//! the executable, written with `--tray` so the console starts in the
//! notification area instead of landing on top of whatever is on screen.
//!
//! No service, no scheduled task, and turning it off is deleting that value —
//! which is what the switch in Settings does. Nothing is written outside
//! `HKEY_CURRENT_USER`, so it works without administrator rights.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "Spud";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn ok(status: WIN32_ERROR) -> bool {
    status == ERROR_SUCCESS
}

/// The executable this process is running from.
pub fn running() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|exe| exe.display().to_string())
}

/// The command Windows should run at sign-in, quoted because a path may have
/// spaces in it, plus the flag that keeps the window hidden.
fn command_for(exe: &str) -> String {
    format!("\"{exe}\" --tray")
}

/// The executable a stored command names.
///
/// This build writes it quoted, but another tool or a hand-edit may have left
/// it bare, so both are read: the trailing `--tray` is dropped and the quotes
/// come off whatever is left.
fn exe_of(command: &str) -> Option<String> {
    let text = command.trim();
    let text = text.strip_suffix("--tray").unwrap_or(text);
    let text = text.trim().trim_matches('"').trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn open(access: REG_SAM_FLAGS) -> Option<HKEY> {
    let path = wide(RUN_KEY);
    let mut handle = HKEY::default();
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, access, &mut handle) };
    if ok(status) {
        Some(handle)
    } else {
        None
    }
}

/// Whatever the `Run` value holds right now, if anything.
fn stored() -> Option<String> {
    let handle = open(KEY_QUERY_VALUE)?;
    let name = wide(VALUE);
    let mut bytes = vec![0u8; 4096];
    let mut size = bytes.len() as u32;
    let status = unsafe {
        RegQueryValueExW(
            handle,
            PCWSTR(name.as_ptr()),
            None,
            None,
            Some(bytes.as_mut_ptr()),
            Some(&mut size),
        )
    };
    unsafe {
        let _ = RegCloseKey(handle);
    }
    if !ok(status) {
        return None;
    }

    let units: Vec<u16> = bytes[..size as usize]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// Whether Windows has anything to launch at sign-in.
///
/// Which executable it names is a separate question, answered by [`entry`]: an
/// entry left behind by a move or an older install is still an entry, and the
/// window shows the path it holds rather than pretending the switch is off.
pub fn enabled() -> bool {
    stored().is_some_and(|value| !value.trim().is_empty())
}

/// The executable the sign-in entry names, if there is an entry.
pub fn entry() -> Option<String> {
    stored().as_deref().and_then(exe_of)
}

fn write_value(value: &str) -> bool {
    let Some(handle) = open(KEY_SET_VALUE) else {
        return false;
    };
    let name = wide(VALUE);
    let data: Vec<u8> = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let status =
        unsafe { RegSetValueExW(handle, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(&data)) };
    unsafe {
        let _ = RegCloseKey(handle);
    }
    ok(status)
}

fn remove_value() -> bool {
    let Some(handle) = open(KEY_SET_VALUE) else {
        return false;
    };
    let name = wide(VALUE);
    // Deleting a value that is not there is the state we want anyway.
    let _ = unsafe { RegDeleteValueW(handle, PCWSTR(name.as_ptr())) };
    unsafe {
        let _ = RegCloseKey(handle);
    }
    true
}

/// Write or delete the value, then report what the registry actually says.
pub fn set(enabled: bool) -> bool {
    let touched = if enabled {
        match running() {
            Some(exe) => write_value(&command_for(&exe)),
            None => false,
        }
    } else {
        remove_value()
    };
    touched && self::enabled() == enabled
}

/// Aim the entry at a chosen executable, creating the entry if it was off.
///
/// This is what the window's "choose another copy" button is for: pointing
/// sign-in at an installed Spud while running a build, or the other way round.
pub fn point_at(path: &str) -> bool {
    let path = path.trim();
    if path.is_empty() {
        return false;
    }
    write_value(&command_for(path))
        && entry().is_some_and(|value| value.eq_ignore_ascii_case(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value this build writes is a quoted path plus the tray flag, and it
    /// reads back as the path it names.
    #[test]
    fn the_command_quotes_the_exe_and_asks_for_the_tray() {
        let exe = r"C:\Program Files\Spud\spud.exe";
        let command = command_for(exe);
        assert_eq!(command, "\"C:\\Program Files\\Spud\\spud.exe\" --tray");
        assert_eq!(exe_of(&command).as_deref(), Some(exe));
    }

    /// A value somebody else wrote — bare, or with the flag missing — is still
    /// read as the executable it names.
    #[test]
    fn a_bare_path_reads_back_too() {
        assert_eq!(exe_of(r"C:\Apps\spud.exe --tray").as_deref(), Some(r"C:\Apps\spud.exe"));
        assert_eq!(exe_of("\"C:\\Apps\\spud.exe\"").as_deref(), Some(r"C:\Apps\spud.exe"));
        assert_eq!(exe_of("   "), None);
    }

    /// Round-trips the real key, then puts back whatever was there so running
    /// the suite does not change this machine's sign-in behaviour.
    #[test]
    fn the_run_value_can_be_written_read_and_removed() {
        let before = stored();

        assert!(set(true), "writing the value should succeed for this user");
        assert!(enabled(), "a written value should read back as enabled");
        assert_eq!(entry().as_deref(), running().as_deref());

        // Pointed at another copy, the entry follows and still reads as an entry.
        let elsewhere = r"C:\Spud elsewhere\spud.exe";
        assert!(point_at(elsewhere), "re-pointing should succeed");
        assert!(enabled());
        assert_eq!(entry().as_deref(), Some(elsewhere));

        assert!(set(false), "deleting the value should succeed");
        assert!(!enabled(), "a deleted value should read back as disabled");
        assert!(stored().is_none());
        assert_eq!(entry(), None);

        if let Some(previous) = before {
            write_value(&previous);
        }
    }
}
