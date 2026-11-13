//! Spud — a mixing console with its own audio engine.
//!
//! Eight inputs in, eight buses out, mixed here rather than in somebody else's
//! product. `mixer` holds the maths, `audio` owns the devices, and this module is
//! the narrow waist between them and the window: parse a parameter name, apply
//! it, hand back a snapshot.

mod audio;
mod mixer;
mod sessions;
mod startup;
mod state;
mod store;
mod update;

use mixer::{Mixer, BUSES, STRIPS};
use state::ROUTES;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use serde::Serialize;
use tauri::{Emitter, Manager, State, WindowEvent};

struct App {
    engine: audio::Engine,
    labels: Arc<Mutex<state::Labels>>,
    /// Where the console is remembered, once setup has resolved it.
    settings: Mutex<Option<PathBuf>>,
    /// Set by anything that changes the mixer; the writer clears it.
    dirty: Arc<AtomicBool>,
}

/// Note that the console has changed, so it should be written out shortly.
fn touch(app: &App) {
    app.dirty.store(true, Ordering::Relaxed);
}

/// Write the console out right now — used when quitting, where the debounced
/// writer would not get another chance.
fn persist_now(app: &App) {
    let dir = app.settings.lock().ok().and_then(|slot| slot.clone());
    let Some(dir) = dir else {
        return;
    };
    let labels = app.labels.lock().map(|l| l.clone()).unwrap_or_default();
    let saved = store::Saved::capture(&app.engine.mix, &app.engine.config(), &labels);
    store::save(&dir, &saved);
}

/// Parameter names the console is allowed to write.
fn writable(name: &str) -> bool {
    ["Strip[", "Bus[", "Command."].iter().any(|p| name.starts_with(p))
}

enum Target {
    Strip(usize),
    Bus(usize),
}

/// `Strip[3].EQGain2` -> (Strip(3), "EQGain2").
fn split(name: &str) -> Option<(Target, &str)> {
    let (strip, rest) = if let Some(rest) = name.strip_prefix("Strip[") {
        (true, rest)
    } else if let Some(rest) = name.strip_prefix("Bus[") {
        (false, rest)
    } else {
        return None;
    };
    let (index, field) = rest.split_once("].")?;
    let index: usize = index.parse().ok()?;
    Some((
        if strip {
            Target::Strip(index)
        } else {
            Target::Bus(index)
        },
        field,
    ))
}

/// Clear any route that would put a strip's own cable back into its input.
///
/// A virtual strip captures a cable; the matching B bus renders into that same
/// cable. Routing one to the other closes a loop through Windows and howls, so
/// the wiring is checked whenever a device moves and any such route is dropped.
fn break_loops(mix: &Mixer, config: &audio::Config) {
    for strip in 0..STRIPS {
        for bus in 0..BUSES {
            if mix.strips[strip].routed_to(bus)
                && audio::same_cable(&config.strip_device[strip], &config.bus_device[bus])
            {
                mix.strips[strip].set_routed(bus, false);
            }
        }
    }
}

/// Note which part of a strip a person has set, so the engine's microphone
/// defaults are not put back over their choices on the next launch. Called for
/// every write, whether it came one at a time or in a script.
fn note_touched(mix: &Mixer, name: &str) {
    let Some((Target::Strip(index), field)) = split(name) else {
        return;
    };
    let Some(strip) = mix.strips.get(index) else {
        return;
    };
    if field.starts_with("EQ") {
        strip.mark_eq_touched();
    } else if field.starts_with("Gate") || field.starts_with("Comp") {
        strip.mark_dynamics_touched();
    }
}

/// Apply one assignment to the mixer. Returns whether the name was one it knows.
fn apply(mix: &Mixer, config: &audio::Config, name: &str, value: f32) -> bool {
    let on = value >= 0.5;
    let Some((target, field)) = split(name) else {
        return false;
    };

    match target {
        Target::Strip(index) => {
            let Some(strip) = mix.strips.get(index) else {
                return false;
            };
            match field {
                "Gain" => strip.set_gain(value),
                "Trim" => strip.set_trim(value),
                "Denoiser" => strip.set_denoiser(value),
                "Mute" => strip.set_mute(on),
                "Solo" => strip.set_solo(on),
                "Mono" => strip.set_mono(on),
                "EQ.on" => strip.set_eq_on(on),
                "EQGain1" => strip.set_eq(0, value),
                "EQGain2" => strip.set_eq(1, value),
                "EQGain3" => strip.set_eq(2, value),
                "EQGain4" => strip.set_eq(3, value),
                "EQCut" => strip.set_cut(value),
                "Gate" => strip.set_gate_on(on),
                "Comp" => strip.set_comp_on(on),
                other => {
                    // `Gate.Hold`, `Comp.Threshold` — the controls behind the two
                    // chips, named as the console writes them.
                    if let Some(name) = other.strip_prefix("Gate.") {
                        if !strip.set_gate_named(name, value) {
                            return false;
                        }
                    } else if let Some(name) = other.strip_prefix("Comp.") {
                        if !strip.set_comp_named(name, value) {
                            return false;
                        }
                    } else if let Some(bus) = ROUTES.iter().position(|route| *route == other) {
                        // Refuse the loop rather than take the answer and mute
                        // it later: returning false leaves the button off.
                        if on && audio::same_cable(
                            &config.strip_device[index],
                            &config.bus_device[bus],
                        ) {
                            return false;
                        }
                        strip.set_routed(bus, on);
                    } else if let Some(axis) = other.strip_prefix("mode.") {
                        match ["Front", "Rear", "Center", "LFE"].iter().position(|a| *a == axis) {
                            Some(axis) => strip.set_position(axis, on),
                            None => return false,
                        }
                    } else {
                        return false;
                    }
                }
            }
        }
        Target::Bus(index) => {
            let Some(bus) = mix.buses.get(index) else {
                return false;
            };
            match field {
                "Gain" => bus.set_gain(value),
                "Mute" => bus.set_mute(on),
                "Mono" => bus.set_mono(on),
                "Limit" => bus.set_limit(on),
                _ => return false,
            }
        }
    }
    true
}

#[tauri::command]
fn engine_status(app: State<'_, App>) -> state::EngineStatus {
    let status = app.engine.status();
    state::EngineStatus {
        found: true,
        dll: None,
        kind: 3,
        edition: "Spud".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        strips: STRIPS,
        buses: BUSES,
        running: status.running,
        inputs: status.inputs.len(),
        outputs: status.outputs.len(),
        errors: status.errors,
    }
}

#[tauri::command]
fn snapshot(app: State<'_, App>) -> Option<state::Snapshot> {
    let labels = app.labels.lock().map(|l| l.clone()).unwrap_or_default();
    Some(state::snapshot(
        &app.engine.mix,
        &app.engine.config(),
        &labels,
        &app.engine.status(),
    ))
}

#[tauri::command]
fn levels(app: State<'_, App>) -> state::Levels {
    state::levels(&app.engine.mix)
}

#[tauri::command]
fn set_param(app: State<'_, App>, name: String, value: f32) -> bool {
    if !writable(&name) {
        return false;
    }
    if name.starts_with("Command.") {
        // The one command the console has: re-open every stream.
        app.engine.assign(app.engine.config());
        return true;
    }
    let applied = apply(&app.engine.mix, &app.engine.config(), &name, value);
    if applied {
        // Any move on a strip's own controls is the person's own: the
        // microphone defaults must not come back over it on the next launch.
        note_touched(&app.engine.mix, &name);
        touch(&app);
    }
    applied
}

fn apply_text(labels: &Mutex<state::Labels>, name: &str, value: &str) -> bool {
    if !writable(name) || value.len() > 64 {
        return false;
    }
    let Some((target, field)) = split(name) else {
        return false;
    };
    if field != "Label" {
        return false;
    }
    let Ok(mut labels) = labels.lock() else {
        return false;
    };
    match target {
        Target::Strip(index) => match labels.strip.get_mut(index) {
            Some(slot) => {
                *slot = value.to_string();
                true
            }
            None => false,
        },
        Target::Bus(index) => match labels.bus.get_mut(index) {
            Some(slot) => {
                *slot = value.to_string();
                true
            }
            None => false,
        },
    }
}

#[tauri::command]
fn set_text(app: State<'_, App>, name: String, value: String) -> bool {
    let applied = apply_text(&app.labels, &name, &value);
    if applied {
        touch(&app);
    }
    applied
}

/// `Strip[0].Gain = -3.0; Strip[0].Mute = 1;` — one call, no interleaving.
#[tauri::command]
fn apply_script(app: State<'_, App>, script: String) -> bool {
    let mut applied = false;
    for statement in script.split(';') {
        let Some((name, value)) = statement.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if !writable(name) {
            continue;
        }
        if name.starts_with("Command.") {
            app.engine.assign(app.engine.config());
            applied = true;
        } else if let Ok(number) = value.parse::<f32>() {
            let landed = apply(&app.engine.mix, &app.engine.config(), name, number);
            if landed {
                note_touched(&app.engine.mix, name);
            }
            applied |= landed;
        } else if let Some(text) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            applied |= apply_text(&app.labels, name, text);
        }
    }
    if applied {
        touch(&app);
    }
    applied
}

/// Assign an output device to a bus and reopen the streams.
#[tauri::command]
fn set_bus_device(app: State<'_, App>, bus: usize, _kind: String, name: String) -> bool {
    if bus >= BUSES {
        return false;
    }
    let mut config = app.engine.config();
    config.bus_device[bus] = name;
    // The new device may be the cable this strip captures; drop that route
    // before it can loop.
    break_loops(&app.engine.mix, &config);
    app.engine.assign(config);
    touch(&app);
    true
}

/// Assign a capture device to a strip and reopen the streams.
#[tauri::command]
fn set_strip_device(app: State<'_, App>, strip: usize, name: String) -> bool {
    if strip >= STRIPS {
        return false;
    }
    let mut config = app.engine.config();
    config.strip_device[strip] = name;
    break_loops(&app.engine.mix, &config);
    app.engine.assign(config);
    touch(&app);
    true
}

#[tauri::command]
fn devices() -> (Vec<audio::DeviceDesc>, Vec<audio::DeviceDesc>) {
    audio::devices()
}

/// What Windows would start at sign-in, and which copy is running now.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupView {
    /// Whether there is an entry at all.
    enabled: bool,
    /// The executable it names; `null` when there is no entry.
    entry: Option<String>,
    /// This process's own executable, which is what "use this copy" means.
    running: Option<String>,
}

fn startup_view() -> StartupView {
    StartupView {
        enabled: startup::enabled(),
        entry: startup::entry(),
        running: startup::running(),
    }
}

/// The sign-in entry as it stands: whether there is one, and which copy it aims at.
#[tauri::command]
fn startup_state() -> StartupView {
    startup_view()
}

/// Turn the sign-in entry on or off, aimed at the copy that is running.
/// Answers with the registry as it ended up, not with what was asked for.
#[tauri::command]
fn set_startup(enabled: bool) -> StartupView {
    startup::set(enabled);
    startup_view()
}

/// Aim the sign-in entry at a chosen executable, creating the entry if it was off.
#[tauri::command]
fn point_startup_at(path: String) -> StartupView {
    startup::point_at(&path);
    startup_view()
}

/// Ask Windows for an executable rather than making the user type a path.
///
/// The answer arrives as an event rather than as the command's return value: the
/// picker is modal to the desktop and answers long after the click that opened
/// it, so the window stays free to keep drawing while it is open.
#[tauri::command]
fn choose_startup_exe(app: tauri::AppHandle) {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("Which copy of Spud should start with Windows?")
        .add_filter("Programs", &["exe"])
        .pick_file(move |picked| {
            let Some(path) = picked else { return };
            let _ = app.emit_to("main", "startup-exe", path.to_string());
        });
}

/// The programs playing into each virtual cable, straight from Windows.
#[tauri::command]
fn app_sessions(app: State<'_, App>) -> Vec<sessions::VirtualInput> {
    sessions::virtual_inputs(&app.engine.config().strip_device)
}

#[tauri::command]
fn set_app_audio(pid: u32, volume: Option<f32>, mute: Option<bool>) -> bool {
    sessions::set_app(pid, volume, mute)
}

/// Reopen every stream, filling in anything that was unassigned.
///
/// This is the button that makes a freshly plugged device — or a newly installed
/// virtual cable — appear without reopening the window: the wiring is recomputed
/// and only the empty slots are touched, so the user's own choices survive.
#[tauri::command]
fn run_engine(app: State<'_, App>) -> bool {
    let mut config = app.engine.config();
    let suggested = audio::auto_config();
    for index in 0..STRIPS {
        if config.strip_device[index].trim().is_empty() {
            config.strip_device[index] = suggested.strip_device[index].clone();
        }
    }
    for index in 0..BUSES {
        if config.bus_device[index].trim().is_empty() {
            config.bus_device[index] = suggested.bus_device[index].clone();
        }
    }
    // A B bus left pointing at a cable another B bus already feeds is silent;
    // this is the button that puts it back on its own cable.
    audio::repair_renders(&mut config);
    break_loops(&app.engine.mix, &config);
    app.engine.assign(config);
    touch(&app);
    true
}

/// Load the console remembered on disk into a live one.
///
/// The saved routes go through the same loop check as a click would: a device
/// may have moved since the file was written, and a stale route must not howl
/// the moment the window opens.
fn restore(dir: &std::path::Path, mix: &Mixer, config: &mut audio::Config, labels: &mut state::Labels) -> bool {
    let Some(saved) = store::load(dir) else {
        return false;
    };
    if !saved.apply(mix, config, labels) {
        return false;
    }
    break_loops(mix, config);
    true
}

/// Bring the console back from the notification area.
fn reveal(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two virtual strips, each on its own cable, plus one hardware output.
    fn wired() -> audio::Config {
        let mut config = audio::Config::default();
        config.strip_device[5] = "CABLE Output (VB-Audio Virtual Cable)".into();
        config.bus_device[5] = "CABLE Input (VB-Audio Virtual Cable)".into();
        config.strip_device[6] = "CABLE-A Output (VB-Audio Virtual Cable A)".into();
        config.bus_device[6] = "CABLE-A Input (VB-Audio Virtual Cable A)".into();
        config.bus_device[0] = "Headphones (2- HyperX Cloud III Wireless)".into();
        config
    }

    /// A cable's capture side cannot also feed its render side: the loop would
    /// arrive back at its own input through Windows.
    #[test]
    fn a_cable_cannot_be_routed_back_into_itself() {
        let mix = Mixer::default();
        let config = wired();

        assert!(!apply(&mix, &config, "Strip[5].B1", 1.0));
        assert!(!mix.strips[5].routed_to(5), "the loop was refused");

        // A different cable is a legitimate destination — that is how one app's
        // audio is folded into another cable's mix.
        assert!(apply(&mix, &config, "Strip[5].B2", 1.0));
        assert!(mix.strips[5].routed_to(6));

        // A microphone has no cable of its own, so every virtual bus is open.
        assert!(apply(&mix, &config, "Strip[0].B1", 1.0));
        assert!(mix.strips[0].routed_to(5));
    }

    /// A bus name is stored where the console writes it, and only a label gets
    /// through that path.
    #[test]
    fn a_bus_name_is_stored_by_the_label_command() {
        let labels = Mutex::new(state::Labels::default());
        assert!(apply_text(&labels, "Bus[5].Label", "Stream"));
        assert_eq!(labels.lock().unwrap().bus[5], "Stream");
        assert!(apply_text(&labels, "Bus[5].Label", ""));
        assert_eq!(labels.lock().unwrap().bus[5], "", "clearing is a rename too");
        assert!(!apply_text(&labels, "Bus[5].Gain", "3"));
        assert!(!apply_text(&labels, "Strip[0].Label", &"x".repeat(65)));
    }

    /// The whole point of the store: a console saved to disk comes back as it
    /// was, and a route that has become a loop is dropped on the way in.
    #[test]
    fn the_saved_console_is_restored_from_disk() {
        let dir = std::env::temp_dir().join(format!("spud-restore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mix = Mixer::default();
        let mut config = audio::Config::default();
        let mut labels = state::Labels::default();
        mix.strips[4].set_gain(-9.0);
        mix.strips[4].set_mute(true);
        labels.strip[4] = "Aux".into();
        config.strip_device[4] = "Line In (USB Audio)".into();
        // Strip 5 captures the plain cable and B1 renders into it: a route that
        // could only exist in the file if a device moved after it was written.
        config.strip_device[5] = "CABLE Output (VB-Audio Virtual Cable)".into();
        config.bus_device[5] = "CABLE Input (VB-Audio Virtual Cable)".into();
        mix.strips[5].set_routed(5, true);
        assert!(store::save(&dir, &store::Saved::capture(&mix, &config, &labels)));

        let fresh = Mixer::default();
        let mut fresh_config = audio::Config::default();
        let mut fresh_labels = state::Labels::default();
        assert!(restore(&dir, &fresh, &mut fresh_config, &mut fresh_labels));

        assert_eq!(fresh.strips[4].gain(), -9.0);
        assert!(fresh.strips[4].mute());
        assert_eq!(fresh_labels.strip[4], "Aux");
        assert_eq!(fresh_config.strip_device[4], "Line In (USB Audio)");
        assert_eq!(fresh_config.bus_device[5], "CABLE Input (VB-Audio Virtual Cable)");
        assert!(
            !fresh.strips[5].routed_to(5),
            "a restored loop must be dropped, not reopened"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Microphones start on the voice preset; anything already tuned — or a
    /// virtual input, which is carrying someone else's audio — is left alone.
    #[test]
    fn a_microphone_starts_on_the_voice_preset() {
        let mix = Mixer::default();
        mix.strips[1].set_eq(2, 6.0);
        mix.strips[1].mark_eq_touched();

        seed_mic_eq(&mix);

        assert!(mix.strips[0].eq_on());
        assert_eq!(mix.strips[0].cut(), VOICE_CUT);
        assert_eq!(mix.strips[0].eq(3), -3.0, "the body band takes the boxiness out");
        assert_eq!(mix.strips[0].eq(0), 2.0, "a little air");

        assert_eq!(mix.strips[1].eq(2), 6.0, "a tuned strip keeps its own settings");
        assert_eq!(mix.strips[1].cut(), 0.0);
        assert!(!mix.strips[1].eq_on());

        assert!(!mix.strips[6].eq_on(), "a virtual input is not a microphone");
        assert_eq!(mix.strips[6].cut(), 0.0);
    }

    /// A microphone also starts with the gate and the compressor doing their
    /// job, and a strip somebody has already set keeps what they set.
    #[test]
    fn a_microphone_starts_on_the_voice_chain() {
        let mix = Mixer::default();
        mix.strips[2].set_gate_on(false);
        mix.strips[2].set_gate_named("Threshold", -55.0);
        mix.strips[2].set_comp_on(false);
        mix.strips[2].mark_dynamics_touched();

        seed_mic_dynamics(&mix);

        assert!(mix.strips[0].gate_on(), "a microphone is gated by default");
        assert_eq!(mix.strips[0].gate(0), -42.0);
        assert_eq!(mix.strips[0].gate(3), 220.0, "the hold is what keeps speech whole");
        assert!(mix.strips[0].comp_on(), "and compressed");
        assert_eq!(mix.strips[0].comp(0), -18.0);
        assert_eq!(mix.strips[0].comp(1), 3.0);
        assert_eq!(mix.strips[0].comp(6), 3.0, "makeup gain hands back what the ratio took");

        assert!(!mix.strips[2].gate_on(), "a tuned strip keeps its own gate");
        assert_eq!(mix.strips[2].gate(0), -55.0);
        assert!(!mix.strips[2].comp_on());

        assert!(!mix.strips[6].gate_on(), "a virtual input is left alone");
        assert!(!mix.strips[6].comp_on());
    }

    /// The console writes the controls by name, so a name the engine does not
    /// know must be refused rather than quietly clamped into the wrong slot.
    #[test]
    fn the_dynamics_controls_are_written_by_name() {
        let mix = Mixer::default();
        let config = audio::Config::default();

        assert!(apply(&mix, &config, "Strip[0].Gate", 1.0));
        assert!(apply(&mix, &config, "Strip[0].Gate.Hold", 175.0));
        assert!(apply(&mix, &config, "Strip[0].Comp", 1.0));
        assert!(apply(&mix, &config, "Strip[0].Comp.Ratio", 2.5));
        assert!(apply(&mix, &config, "Strip[0].Trim", 6.0));
        assert!(apply(&mix, &config, "Strip[0].Denoiser", 0.5));
        assert!(!apply(&mix, &config, "Strip[0].Gate.Nonsense", 1.0));
        assert!(!apply(&mix, &config, "Strip[0].Comp.Nonsense", 1.0));

        assert!(mix.strips[0].gate_on());
        assert_eq!(mix.strips[0].gate(3), 175.0);
        assert!(mix.strips[0].comp_on());
        assert_eq!(mix.strips[0].comp(1), 2.5);
        assert_eq!(mix.strips[0].trim(), 6.0, "the input level is written by name too");
        assert_eq!(mix.strips[0].denoiser(), 0.5, "and the denoiser amount");

        // Writing a control is also what marks the strip as the person's own.
        assert!(!mix.strips[0].dynamics_touched());
        note_touched(&mix, "Strip[0].Comp.Ratio");
        assert!(mix.strips[0].dynamics_touched());
        assert!(!mix.strips[0].eq_touched());

        // The input level is not part of the voice chain the engine seeds, so
        // setting it must not stop that seed being applied.
        let untouched = Mixer::default();
        note_touched(&untouched, "Strip[0].Trim");
        assert!(!untouched.strips[0].dynamics_touched());
        assert!(!untouched.strips[0].eq_touched());
    }

    /// Moving a cable onto a bus that an input already feeds back to drops the
    /// route, because the pair became a loop without anything else being touched.
    #[test]
    fn moving_a_device_clears_the_route_it_would_loop() {
        let mix = Mixer::default();
        let mut config = wired();
        mix.strips[5].set_routed(5, true);
        mix.strips[5].set_routed(6, true);
        mix.strips[0].set_routed(2, true);

        // B2 is now the same cable strip 5 captures, as B1 already was.
        config.bus_device[6] = "CABLE Input (VB-Audio Virtual Cable)".into();
        break_loops(&mix, &config);

        assert!(!mix.strips[5].routed_to(5));
        assert!(!mix.strips[5].routed_to(6));
        assert!(mix.strips[0].routed_to(2), "unrelated routing is left alone");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// How a microphone starts out: a cut below the rumble, a little taken out of
/// the boxy region a close mic has, a lift where consonants live, and a touch of
/// air. Seeded only for strips nobody has tuned, so it never overwrites a choice.
pub const VOICE_CUT: f32 = 90.0;
pub const VOICE_EQ: [f32; 4] = [2.0, 1.5, -2.0, -3.0];

/// A microphone's gate, by control name.
///
/// -42 dB is under a voice at a normal distance and over most room tone; three
/// milliseconds to open is fast enough not to clip a consonant; and the hold is
/// what keeps the gate from closing in the gaps between words so it does not
/// sound like a radio tuned between stations. -50 dB of damping is silence to
/// anyone listening, short of a hard zero.
///
/// The low cut runs in front of it, so desk rumble does not hold it open.
pub const VOICE_GATE: [(&str, f32); 5] = [
    ("Threshold", -42.0),
    ("Attack", 3.0),
    ("Release", 200.0),
    ("Hold", 220.0),
    ("Damping", -50.0),
];

/// A microphone's compressor, by control name.
///
/// A voice at a normal distance peaks around -18 dB, so that is where the
/// threshold goes, and 3:1 takes the top off without flattening the delivery. A
/// six-decibel knee means the compression eases in rather than switching. The
/// ten-millisecond attack lets a shout through for an instant before it is
/// caught, which is what keeps it from sounding squeezed, and the makeup gain
/// hands back what the ratio took.
pub const VOICE_COMP: [(&str, f32); 7] = [
    ("Threshold", -18.0),
    ("Ratio", 3.0),
    ("Attack", 10.0),
    ("Release", 150.0),
    ("Knee", 6.0),
    ("GainIn", 0.0),
    ("GainOut", 3.0),
];

fn seed_mic_eq(mix: &Mixer) {
    for index in 0..state::FIRST_VIRTUAL {
        let strip = &mix.strips[index];
        if strip.eq_touched() {
            continue;
        }
        strip.set_eq_on(true);
        for (band, db) in VOICE_EQ.iter().enumerate() {
            strip.set_eq(band, *db);
        }
        strip.set_cut(VOICE_CUT);
    }
}

/// The gate and the compressor a microphone starts on, seeded for the same
/// reason and with the same guard as the equaliser: a strip somebody has tuned
/// keeps their settings, and a virtual input is left alone because it is
/// carrying another program's audio rather than a voice.
fn seed_mic_dynamics(mix: &Mixer) {
    for index in 0..state::FIRST_VIRTUAL {
        let strip = &mix.strips[index];
        if strip.dynamics_touched() {
            continue;
        }
        strip.set_gate_on(true);
        for (name, value) in VOICE_GATE {
            strip.set_gate_named(name, value);
        }
        strip.set_comp_on(true);
        for (name, value) in VOICE_COMP {
            strip.set_comp_named(name, value);
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        // Registered first, as the plugin requires. A second launch hands its
        // window to this callback and exits, and it does so before any of the
        // work in `setup` below: two consoles would each open the same cables
        // and output devices, so the second one must not get that far.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            reveal(app);
        }))
        // Only used from Rust, to let the sign-in entry be pointed at a copy of
        // the app rather than typed in by hand.
        .plugin(tauri_plugin_dialog::init())
        // The release endpoint, checked after launch and every six hours. The
        // state the banner reads lives in `update`.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // The engine is started here rather than before the builder so the
            // single-instance check above has already turned a second copy away.
            let engine = audio::Engine::start();
            seed_mic_eq(&engine.mix);
            seed_mic_dynamics(&engine.mix);
            // Every strip starts on A1, so the console is audible before
            // anything is touched. A virtual strip whose cable is also its own
            // bus is skipped.
            for strip in &engine.mix.strips {
                strip.set_routed(0, true);
            }
            break_loops(&engine.mix, &engine.config());

            app.manage(App {
                engine,
                labels: Arc::new(Mutex::new(state::Labels::default())),
                settings: Mutex::new(None),
                dirty: Arc::new(AtomicBool::new(false)),
            });

            // Restore first: the console should come up exactly as it was left,
            // device choices included, rather than showing the automatic wiring
            // and then jumping to the saved one.
            let state = app.state::<App>();
            let directory = app.path().app_config_dir().ok();
            if let Some(dir) = directory.as_ref() {
                let mut config = state.engine.config();
                let mut labels = state.labels.lock().map(|l| l.clone()).unwrap_or_default();
                if restore(dir, &state.engine.mix, &mut config, &mut labels) {
                    if let Ok(mut slot) = state.labels.lock() {
                        *slot = labels;
                    }
                    // The file may hold a pairing that was right under the old
                    // device list: a bus on a cable another bus already renders
                    // into, which plays silence rather than a mistake.
                    audio::repair_renders(&mut config);
                    break_loops(&state.engine.mix, &config);
                    state.engine.assign(config);
                    // Write the repaired pairing back, so the next launch starts
                    // from what is actually running.
                    state.dirty.store(true, Ordering::Relaxed);
                }
            }
            if let Some(dir) = directory {
                if let Ok(mut slot) = state.settings.lock() {
                    *slot = Some(dir.clone());
                }
                store::watch(
                    dir,
                    Arc::clone(&state.engine.mix),
                    Arc::clone(&state.labels),
                    state.engine.config_handle(),
                    Arc::clone(&state.dirty),
                );
            }

            // Closing the window hides it instead of quitting: the audio thread
            // is this process, so the mix keeps running while Spud sits in the
            // notification area. Quit lives on the tray menu.
            if let Some(window) = app.get_webview_window("main") {
                let handle = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = handle.hide();
                    }
                });
            }

            let open = MenuItem::with_id(app, "open", "Open Spud", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            TrayIconBuilder::with_id("spud")
                .icon(app.default_window_icon().cloned().expect("the window icon"))
                .tooltip("Spud — mixing console")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => reveal(app),
                    "quit" => {
                        if let Some(state) = app.try_state::<App>() {
                            // The debounced writer will not run again.
                            persist_now(&state);
                            state.engine.stop();
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        reveal(tray.app_handle());
                    }
                })
                .build(app)?;

            // Started by the sign-in entry: stay in the notification area until
            // the tray icon is clicked, rather than opening over the desktop.
            if std::env::args().any(|arg| arg == "--tray") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            // Registered last, because the first check waits twenty seconds for
            // the audio thread to settle and never blocks the window.
            app.manage(update::UpdateHolder::new());
            update::watch(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine_status,
            snapshot,
            levels,
            set_param,
            set_text,
            apply_script,
            set_bus_device,
            set_strip_device,
            devices,
            app_sessions,
            set_app_audio,
            run_engine,
            startup_state,
            set_startup,
            point_startup_at,
            choose_startup_exe,
            update::update_state,
            update::update_check,
            update::update_install
        ])
        .run(tauri::generate_context!())
        .expect("error while running the console");
}
