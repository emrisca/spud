//! The console's view of the mixer.
//!
//! The window speaks one vocabulary — `Strip[3].Gain`, `Bus[1].Mute`, `A1..B3` —
//! and this module is where those names bind to the mixer's own parameters. It
//! also builds the snapshot: one struct per strip and bus, in the shapes the
//! console draws.

use crate::audio::{Config, Status};
use crate::mixer::{Mixer, BUSES, STRIPS};

/// Routing targets, in the order the console lays them out. A1..A5 are the
/// hardware outputs; B1..B3 are the three virtual cables.
pub const ROUTES: [&str; 8] = ["A1", "A2", "A3", "A4", "A5", "B1", "B2", "B3"];
/// Strips from here on are virtual inputs: they capture a cable, not a mic.
pub const FIRST_VIRTUAL: usize = 5;

/// Names the user typed. Kept apart from the mixer so a rename does not touch
/// the audio thread.
#[derive(Clone)]
pub struct Labels {
    pub strip: [String; STRIPS],
    pub bus: [String; BUSES],
}

impl Default for Labels {
    fn default() -> Self {
        Labels {
            strip: [
                "Hardware 1".into(),
                "Hardware 2".into(),
                "Hardware 3".into(),
                "Hardware 4".into(),
                "Hardware 5".into(),
                "Virtual 1".into(),
                "Virtual 2".into(),
                "Virtual 3".into(),
            ],
            bus: Default::default(),
        }
    }
}

#[derive(serde::Serialize)]
pub struct EngineStatus {
    /// Always true: the engine is this process, not something installed.
    pub found: bool,
    pub dll: Option<String>,
    pub kind: i32,
    pub edition: String,
    pub version: String,
    pub strips: usize,
    pub buses: usize,
    /// True while at least one stream is open.
    pub running: bool,
    /// Inputs and outputs with a live stream.
    pub inputs: usize,
    pub outputs: usize,
    /// Assignments that could not be opened.
    pub errors: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct Strip {
    pub index: usize,
    pub label: String,
    pub device: String,
    /// Strip 5..7 capture a virtual cable; 0..4 capture a microphone or line.
    pub virtual_input: bool,
    pub gain: f32,
    /// Level into the strip in dB, ahead of the gate, the equaliser and the
    /// compressor.
    pub trim: f32,
    pub mute: bool,
    pub solo: bool,
    pub mono: bool,
    pub routes: Vec<bool>,
    pub position: Vec<bool>,
    pub comp: f32,
    pub gate: f32,
    pub eq: bool,
    /// Treble, mid, bass, body — in the order the knobs read.
    pub eq_gain: Vec<f32>,
    /// Low-cut corner in Hz, 0 when it is off.
    pub eq_cut: f32,
    pub pan: f32,
    pub limit: f32,
    pub denoiser: f32,
    pub fx1: f32,
    pub fx2: f32,
    pub comp_detail: Comp,
    pub gate_detail: Gate,
}

#[derive(serde::Serialize)]
pub struct Comp {
    pub threshold: f32,
    pub ratio: f32,
    pub attack: f32,
    pub release: f32,
    pub knee: f32,
    pub gain_in: f32,
    pub gain_out: f32,
}

#[derive(serde::Serialize)]
pub struct Gate {
    pub threshold: f32,
    pub attack: f32,
    pub release: f32,
    pub hold: f32,
    pub damping: f32,
}

#[derive(serde::Serialize)]
pub struct Bus {
    pub index: usize,
    pub key: String,
    pub physical: bool,
    pub label: String,
    pub device: String,
    pub gain: f32,
    pub mute: bool,
    pub mono: bool,
    /// The ceiling this bus is held under, on or off.
    pub limit: bool,
    pub eq: bool,
    pub sel: bool,
}

#[derive(serde::Serialize)]
pub struct Snapshot {
    pub strips: Vec<Strip>,
    pub buses: Vec<Bus>,
    pub sample_rate: f32,
    /// Frames per capture callback, as the devices deliver them.
    pub buffer: f32,
    /// The buffer the readers are holding behind the devices, in milliseconds.
    pub latency: f32,
}

#[derive(serde::Serialize, Default)]
pub struct Levels {
    pub strips: Vec<f32>,
    pub buses: Vec<f32>,
    /// The mix entering each bus's ceiling, before the limiter pulls it down.
    pub buses_post: Vec<f32>,
    /// The ceiling itself, so the console compares against the engine's number
    /// instead of keeping a copy of it.
    pub ceiling: f32,
}


pub fn snapshot(mix: &Mixer, config: &Config, labels: &Labels, status: &Status) -> Snapshot {
    let strips = (0..STRIPS)
        .map(|index| {
            let params = &mix.strips[index];
            Strip {
                index,
                label: labels.strip[index].clone(),
                device: config.strip_device[index].clone(),
                virtual_input: index >= FIRST_VIRTUAL,
                gain: params.gain(),
                trim: params.trim(),
                mute: params.mute(),
                solo: params.solo(),
                mono: params.mono(),
                routes: (0..BUSES).map(|bus| params.routed_to(bus)).collect(),
                position: (0..4).map(|axis| params.position_at(axis)).collect(),
                // The engine mixes gain, routing, mute, solo, the gate, the four
                // bands, the low cut and the compressor. These read out as 0/1
                // so the console can light a chip from the same field. The FX
                // sends and the denoiser are not wired to audio yet, so they
                // report their resting values and the console offers no control.
                comp: if params.comp_on() { 1.0 } else { 0.0 },
                gate: if params.gate_on() { 1.0 } else { 0.0 },
                eq: params.eq_on(),
                eq_gain: (0..4).map(|band| params.eq(band)).collect(),
                eq_cut: params.cut(),
                pan: 0.0,
                limit: 0.0,
                denoiser: params.denoiser(),
                fx1: 0.0,
                fx2: 0.0,
                // The order of these two is `mixer::COMP_FIELDS` and
                // `mixer::GATE_FIELDS`, which is where the names come from.
                comp_detail: Comp {
                    threshold: params.comp(0),
                    ratio: params.comp(1),
                    attack: params.comp(2),
                    release: params.comp(3),
                    knee: params.comp(4),
                    gain_in: params.comp(5),
                    gain_out: params.comp(6),
                },
                gate_detail: Gate {
                    threshold: params.gate(0),
                    attack: params.gate(1),
                    release: params.gate(2),
                    hold: params.gate(3),
                    damping: params.gate(4),
                },
            }
        })
        .collect();

    let buses = (0..BUSES)
        .map(|index| {
            let params = &mix.buses[index];
            Bus {
                index,
                key: ROUTES[index].to_string(),
                physical: index < FIRST_VIRTUAL,
                label: labels.bus[index].clone(),
                device: config.bus_device[index].clone(),
                gain: params.gain(),
                mute: params.mute(),
                mono: params.mono(),
                limit: params.limit_on(),
                eq: false,
                sel: false,
            }
        })
        .collect();

    let _ = status;
    Snapshot {
        strips,
        buses,
        sample_rate: crate::mixer::RATE as f32,
        // Both of these are what the devices are doing right now, not what the
        // engine hoped for when it opened them.
        buffer: if mix.block() == 0 {
            0.0
        } else {
            mix.block() as f32
        },
        latency: crate::mixer::latency_ms() as f32,
    }
}

pub fn levels(mix: &Mixer) -> Levels {
    Levels {
        strips: (0..STRIPS).map(|index| mix.strip_peak(index)).collect(),
        buses: (0..BUSES).map(|index| mix.bus_peak(index)).collect(),
        buses_post: (0..BUSES).map(|index| mix.bus_post(index)).collect(),
        ceiling: crate::mixer::ceiling(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixer::{ceiling, Mixer, BUSES};

    /// The lamp compares a bus's pre-ceiling level with `ceiling`, so the number
    /// the console is handed has to be the one the limiter holds. A second copy
    /// of it is how a bus ends up held down with no lamp to say so.
    #[test]
    fn the_levels_publish_the_ceiling_the_limiter_uses() {
        let levels = levels(&Mixer::default());
        assert_eq!(levels.ceiling, ceiling());
        assert!(levels.ceiling < 1.0, "the ceiling leaves headroom");
        assert_eq!(levels.buses_post.len(), BUSES);
        assert_eq!(levels.buses.len(), BUSES);
    }

    #[test]
    fn the_snapshot_reflects_the_mixer() {
        let mix = Mixer::default();
        mix.strips[0].set_gain(-6.0);
        mix.strips[0].set_trim(4.0);
        mix.strips[0].set_mute(true);
        mix.strips[2].set_routed(5, true);
        mix.buses[1].set_gain(3.0);

        let snapshot = snapshot(
            &mix,
            &Config::default(),
            &Labels::default(),
            &Status::default(),
        );

        assert_eq!(snapshot.strips.len(), STRIPS);
        assert_eq!(snapshot.buses.len(), BUSES);
        assert_eq!(snapshot.strips[0].gain, -6.0);
        assert_eq!(snapshot.strips[0].trim, 4.0);
        assert!(snapshot.strips[0].mute);
        assert!(snapshot.strips[2].routes[5]);
        assert!(!snapshot.strips[2].routes[0]);
        assert_eq!(snapshot.buses[1].gain, 3.0);
        assert_eq!(snapshot.buses[1].key, "A2");
        assert!(snapshot.strips[6].virtual_input);
        assert!(!snapshot.strips[4].virtual_input);
        assert_eq!(snapshot.sample_rate, 48_000.0);
    }

    /// The gate and the compressor are the console's chips, so an untouched
    /// strip must report them off and a seeded one must report its settings.
    #[test]
    fn the_snapshot_reflects_the_dynamics() {
        let mix = Mixer::default();
        mix.strips[0].set_gate_on(true);
        mix.strips[0].set_gate_named("Threshold", -42.0);
        mix.strips[0].set_comp_on(true);
        mix.strips[0].set_comp_named("Ratio", 3.0);
        mix.strips[0].set_comp_named("GainOut", 3.0);

        let snapshot = snapshot(
            &mix,
            &Config::default(),
            &Labels::default(),
            &Status::default(),
        );

        assert_eq!(snapshot.strips[0].gate, 1.0);
        assert_eq!(snapshot.strips[0].comp, 1.0);
        assert_eq!(snapshot.strips[0].gate_detail.threshold, -42.0);
        assert_eq!(snapshot.strips[0].comp_detail.ratio, 3.0);
        assert_eq!(snapshot.strips[0].comp_detail.gain_out, 3.0);
        assert_eq!(snapshot.strips[1].gate, 0.0, "off is off on the next strip");
    }

    #[test]
    fn meters_are_reported_per_channel() {
        let mix = Mixer::default();
        mix.set_strip_peak(3, 0.5);
        mix.set_bus_peak(2, 0.7, 0.6);
        let levels = levels(&mix);
        assert!((levels.strips[3] - 0.5).abs() < 1e-6);
        assert!((levels.buses[2] - 0.7).abs() < 1e-6);
        assert!((levels.buses_post[2] - 0.6).abs() < 1e-6);
    }
}
