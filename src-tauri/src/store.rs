//! What the console remembers between launches.
//!
//! The mixer is atomics in RAM, so every fader, mute, route, EQ band, label and
//! device choice would be gone on exit without this. The file is small plain
//! JSON in the app's config directory, written by a watcher thread a moment
//! after the last change — dragging a fader produces dozens of updates and none
//! of them deserves its own disk write.
//!
//! Restoring is deliberately literal: an empty device string is a real choice
//! (the user pressed **None**), so it is restored as empty rather than being
//! quietly replaced by the automatic wiring.

use crate::audio::Config;
use crate::mixer::{Mixer, BUSES, COMP_FIELDS, GATE_FIELDS, STRIPS};
use crate::state::Labels;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const FILE: &str = "settings.json";
const VERSION: u32 = 1;
/// Quiet time after the last change before the file is written.
const SETTLE: Duration = Duration::from_millis(600);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StripState {
    pub gain: f32,
    /// Level into the strip in dB. Not behind the `touched` guard below: the
    /// engine seeds no trim, so whatever is saved here is always the person's.
    #[serde(default)]
    pub trim: f32,
    pub mute: bool,
    pub solo: bool,
    pub mono: bool,
    /// One bit per bus, in `A1..B3` order.
    pub routes: u32,
    /// Front / Rear / Center / LFE.
    pub position: u32,
    pub eq_on: bool,
    /// Treble, mid, bass, body. Read as a list so a file written by an older
    /// build — three bands — still loads, with the body band at its default.
    #[serde(default)]
    pub eq: Vec<f32>,
    /// Low-cut corner in Hz, 0 when off.
    #[serde(default)]
    pub cut: f32,
    /// False on a strip nobody has tuned, which is how a microphone gets the
    /// voice defaults back after an upgrade instead of a flat, saved EQ.
    #[serde(default)]
    pub eq_touched: bool,
    #[serde(default)]
    pub gate_on: bool,
    #[serde(default)]
    pub comp_on: bool,
    /// Threshold, attack, release, hold and damping. A list for the same reason
    /// the EQ bands are: an older file must still load.
    #[serde(default)]
    pub gate: Vec<f32>,
    /// Threshold, ratio, attack, release, knee, input trim and makeup.
    #[serde(default)]
    pub comp: Vec<f32>,
    /// Same guard as `eq_touched`, for the gate and the compressor.
    #[serde(default)]
    pub dynamics_touched: bool,
    pub label: String,
    pub device: String,
}

/// A bus nobody has saved yet arrives with its ceiling on, which is the state a
/// first launch runs in: a file written before the limiter existed must not be
/// the one console that still clips its cable.
fn limit_by_default() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BusState {
    pub gain: f32,
    pub mute: bool,
    pub mono: bool,
    #[serde(default = "limit_by_default")]
    pub limit: bool,
    pub label: String,
    pub device: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Saved {
    pub version: u32,
    pub strips: Vec<StripState>,
    pub buses: Vec<BusState>,
}

impl Saved {
    /// Read the live mixer into something serialisable.
    pub fn capture(mix: &Mixer, config: &Config, labels: &Labels) -> Saved {
        Saved {
            version: VERSION,
            strips: (0..STRIPS)
                .map(|index| {
                    let strip = &mix.strips[index];
                    StripState {
                        gain: strip.gain(),
                        trim: strip.trim(),
                        mute: strip.mute(),
                        solo: strip.solo(),
                        mono: strip.mono(),
                        routes: strip.routes(),
                        position: strip.position(),
                        eq_on: strip.eq_on(),
                        eq: (0..4).map(|band| strip.eq(band)).collect(),
                        cut: strip.cut(),
                        eq_touched: strip.eq_touched(),
                        gate_on: strip.gate_on(),
                        comp_on: strip.comp_on(),
                        gate: (0..GATE_FIELDS.len()).map(|field| strip.gate(field)).collect(),
                        comp: (0..COMP_FIELDS.len()).map(|field| strip.comp(field)).collect(),
                        dynamics_touched: strip.dynamics_touched(),
                        label: labels.strip[index].clone(),
                        device: config.strip_device[index].clone(),
                    }
                })
                .collect(),
            buses: (0..BUSES)
                .map(|index| {
                    let bus = &mix.buses[index];
                    BusState {
                        gain: bus.gain(),
                        mute: bus.mute(),
                        mono: bus.mono(),
                        limit: bus.limit_on(),
                        label: labels.bus[index].clone(),
                        device: config.bus_device[index].clone(),
                    }
                })
                .collect(),
        }
    }

    /// Write a saved console back onto a live one. Returns whether anything was
    /// taken — an unknown version or an empty file changes nothing.
    pub fn apply(&self, mix: &Mixer, config: &mut Config, labels: &mut Labels) -> bool {
        if self.version != VERSION {
            return false;
        }
        let mut applied = false;

        for (index, saved) in self.strips.iter().take(STRIPS).enumerate() {
            let strip = &mix.strips[index];
            strip.set_gain(saved.gain);
            strip.set_trim(saved.trim);
            strip.set_mute(saved.mute);
            strip.set_solo(saved.solo);
            strip.set_mono(saved.mono);
            for bus in 0..BUSES {
                strip.set_routed(bus, saved.routes & (1 << bus) != 0);
            }
            for axis in 0..4 {
                strip.set_position(axis, saved.position & (1 << axis) != 0);
            }
            // A strip nobody has equalised keeps whatever defaults the engine
            // seeded, rather than being flattened back to 0 dB.
            if saved.eq_touched {
                strip.set_eq_on(saved.eq_on);
                for band in 0..4 {
                    strip.set_eq(band, saved.eq.get(band).copied().unwrap_or(0.0));
                }
                strip.set_cut(saved.cut);
                strip.mark_eq_touched();
            }
            // Same for the gate and the compressor: a strip nobody has tuned
            // keeps the voice chain the engine seeded on this launch.
            if saved.dynamics_touched {
                strip.set_gate_on(saved.gate_on);
                strip.set_comp_on(saved.comp_on);
                for field in 0..GATE_FIELDS.len() {
                    if let Some(value) = saved.gate.get(field) {
                        strip.set_gate(field, *value);
                    }
                }
                for field in 0..COMP_FIELDS.len() {
                    if let Some(value) = saved.comp.get(field) {
                        strip.set_comp(field, *value);
                    }
                }
                strip.mark_dynamics_touched();
            }
            labels.strip[index] = saved.label.clone();
            config.strip_device[index] = saved.device.clone();
            applied = true;
        }

        for (index, saved) in self.buses.iter().take(BUSES).enumerate() {
            let bus = &mix.buses[index];
            bus.set_gain(saved.gain);
            bus.set_mute(saved.mute);
            bus.set_mono(saved.mono);
            bus.set_limit(saved.limit);
            labels.bus[index] = saved.label.clone();
            config.bus_device[index] = saved.device.clone();
            applied = true;
        }

        applied
    }
}

pub fn file(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Read the file, if it is there and readable. A damaged file is not an error
/// worth reporting: a fresh console is better than refusing to start.
pub fn load(dir: &Path) -> Option<Saved> {
    let text = std::fs::read_to_string(file(dir)).ok()?;
    let saved: Saved = serde_json::from_str(&text).ok()?;
    if saved.version == VERSION {
        Some(saved)
    } else {
        None
    }
}

/// Write it out. The replacement is staged as `settings.json.tmp` first so a
/// half-written file is never left where a restart would read it.
pub fn save(dir: &Path, saved: &Saved) -> bool {
    let Ok(text) = serde_json::to_string_pretty(saved) else {
        return false;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let target = file(dir);
    let staging = target.with_extension("json.tmp");
    if std::fs::write(&staging, text).is_err() {
        return false;
    }
    // Windows will not rename over an existing file.
    let _ = std::fs::remove_file(&target);
    std::fs::rename(&staging, &target).is_ok()
}

/// Watch for changes and write them once things go quiet.
///
/// The `dirty` flag is set by every command that touches the mixer; the thread
/// coalesces a burst of them into one write and stops when the process exits.
pub fn watch(
    dir: PathBuf,
    mix: Arc<Mixer>,
    labels: Arc<Mutex<Labels>>,
    config: Arc<Mutex<Config>>,
    dirty: Arc<AtomicBool>,
) {
    let _ = std::thread::Builder::new()
        .name("spud-settings".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            if !dirty.swap(false, Ordering::Relaxed) {
                continue;
            }
            // Wait out the rest of a burst before committing it.
            std::thread::sleep(SETTLE);
            dirty.store(false, Ordering::Relaxed);

            let Ok(labels) = labels.lock() else { continue };
            let Ok(config) = config.lock() else { continue };
            let saved = Saved::capture(&mix, &config, &labels);
            save(&dir, &saved);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_round_trip_restores_the_whole_console() {
        let mix = Mixer::default();
        let mut config = Config::default();
        let mut labels = Labels::default();

        mix.strips[2].set_gain(-7.5);
        mix.strips[2].set_trim(7.5);
        mix.strips[2].set_mute(true);
        mix.strips[2].set_mono(true);
        mix.strips[2].set_routed(0, true);
        mix.strips[2].set_routed(6, true);
        mix.strips[2].set_position(1, true);
        mix.strips[2].set_eq_on(true);
        mix.strips[2].set_eq(2, 4.5);
        mix.strips[2].set_eq(3, -2.5);
        mix.strips[2].set_cut(90.0);
        mix.strips[2].mark_eq_touched();
        mix.strips[2].set_gate_on(true);
        mix.strips[2].set_gate_named("Threshold", -44.0);
        mix.strips[2].set_gate_named("Hold", 250.0);
        mix.strips[2].set_comp_on(true);
        mix.strips[2].set_comp_named("Ratio", 3.5);
        mix.strips[2].set_comp_named("GainOut", 4.0);
        mix.strips[2].mark_dynamics_touched();
        mix.strips[6].set_gain(-12.0);
        labels.strip[2] = "Guitar".into();
        config.strip_device[2] = "Microphone (USB Audio)".into();
        // Explicitly unassigned: the *None* row, which must survive a restart.
        config.bus_device[3] = String::new();
        mix.buses[5].set_gain(3.0);
        mix.buses[5].set_mute(true);
        labels.bus[5] = "Stream".into();
        config.bus_device[5] = "CABLE Input (VB-Audio Virtual Cable)".into();

        let saved = Saved::capture(&mix, &config, &labels);

        let fresh = Mixer::default();
        let mut fresh_config = Config::default();
        let mut fresh_labels = Labels::default();
        assert!(saved.apply(&fresh, &mut fresh_config, &mut fresh_labels));

        assert_eq!(fresh.strips[2].gain(), -7.5);
        assert_eq!(fresh.strips[2].trim(), 7.5);
        assert!(fresh.strips[2].mute());
        assert!(fresh.strips[2].mono());
        assert!(fresh.strips[2].routed_to(0));
        assert!(fresh.strips[2].routed_to(6));
        assert!(!fresh.strips[2].routed_to(1));
        assert!(fresh.strips[2].position_at(1));
        assert!(fresh.strips[2].eq_on());
        assert_eq!(fresh.strips[2].eq(2), 4.5);
        assert_eq!(fresh.strips[2].eq(3), -2.5);
        assert_eq!(fresh.strips[2].cut(), 90.0);
        assert!(fresh.strips[2].gate_on());
        assert_eq!(fresh.strips[2].gate(0), -44.0);
        assert_eq!(fresh.strips[2].gate(3), 250.0);
        assert!(fresh.strips[2].comp_on());
        assert_eq!(fresh.strips[2].comp(1), 3.5);
        assert_eq!(fresh.strips[2].comp(6), 4.0);
        assert!(!fresh.strips[6].gate_on(), "a strip nobody touched keeps its chain off");
        assert!(!fresh.strips[6].comp_on());
        assert_eq!(fresh.strips[6].gain(), -12.0);
        assert_eq!(fresh_labels.strip[2], "Guitar");
        assert_eq!(fresh_config.strip_device[2], "Microphone (USB Audio)");
        assert_eq!(fresh_config.bus_device[3], "");
        assert_eq!(fresh.buses[5].gain(), 3.0);
        assert!(fresh.buses[5].mute());
        assert_eq!(fresh_labels.bus[5], "Stream");
        assert_eq!(
            fresh_config.bus_device[5],
            "CABLE Input (VB-Audio Virtual Cable)"
        );
    }

    /// A file written before anyone touched a microphone's EQ must not flatten
    /// the voice preset the engine seeded on this launch.
    #[test]
    fn an_untouched_strip_keeps_its_defaults() {
        let mix = Mixer::default();
        let saved = Saved::capture(&mix, &Config::default(), &Labels::default());

        let fresh = Mixer::default();
        // What the engine seeds for a microphone.
        fresh.strips[0].set_eq_on(true);
        fresh.strips[0].set_eq(3, -3.0);
        fresh.strips[0].set_cut(90.0);
        fresh.strips[0].set_gate_on(true);
        fresh.strips[0].set_gate_named("Threshold", -42.0);
        fresh.strips[0].set_comp_on(true);
        fresh.strips[0].set_comp_named("Threshold", -18.0);

        let mut config = Config::default();
        let mut labels = Labels::default();
        assert!(saved.apply(&fresh, &mut config, &mut labels));

        assert!(fresh.strips[0].eq_on(), "the defaults survive an untouched save");
        assert_eq!(fresh.strips[0].cut(), 90.0);
        assert_eq!(fresh.strips[0].eq(3), -3.0);
        assert!(fresh.strips[0].gate_on());
        assert_eq!(fresh.strips[0].gate(0), -42.0);
        assert!(fresh.strips[0].comp_on());
        assert_eq!(fresh.strips[0].comp(0), -18.0);
    }

    #[test]
    fn a_file_round_trip_survives_the_disk() {
        let dir = std::env::temp_dir().join(format!("spud-settings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mix = Mixer::default();
        mix.strips[0].set_gain(-3.0);
        let config = Config::default();
        let saved = Saved::capture(&mix, &config, &Labels::default());

        assert!(save(&dir, &saved), "the file should be written");
        let read = load(&dir).expect("the file should read back");
        assert_eq!(read, saved);
        assert!(!file(&dir).with_extension("json.tmp").exists(), "no staging file is left behind");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The watcher is what actually writes during normal use: a change sets the
    /// flag, the thread coalesces the burst and commits it once quiet.
    #[test]
    fn the_watcher_writes_after_a_change() {
        let dir = std::env::temp_dir().join(format!("spud-settings-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mix = Arc::new(Mixer::default());
        let labels = Arc::new(Mutex::new(Labels::default()));
        let config = Arc::new(Mutex::new(Config::default()));
        let dirty = Arc::new(AtomicBool::new(false));
        watch(
            dir.clone(),
            Arc::clone(&mix),
            Arc::clone(&labels),
            Arc::clone(&config),
            Arc::clone(&dirty),
        );

        // Nothing has happened yet, so nothing should be written.
        std::thread::sleep(Duration::from_millis(700));
        assert!(!file(&dir).exists(), "an untouched console writes no file");

        mix.strips[1].set_gain(-5.0);
        labels.lock().unwrap().strip[1] = "Browser".into();
        dirty.store(true, Ordering::Relaxed);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline && !file(&dir).exists() {
            std::thread::sleep(Duration::from_millis(100));
        }

        let saved = load(&dir).expect("the watcher should have written the console out");
        assert_eq!(saved.strips[1].gain, -5.0);
        assert_eq!(saved.strips[1].label, "Browser");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_or_future_file_is_ignored() {
        let dir = std::env::temp_dir().join(format!("spud-settings-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(file(&dir), "{ this is not json").unwrap();
        assert!(load(&dir).is_none());

        std::fs::write(file(&dir), r#"{"version":99,"strips":[],"buses":[]}"#).unwrap();
        assert!(load(&dir).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
