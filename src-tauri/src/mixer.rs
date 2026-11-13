//! The mixing core: eight inputs, eight outputs, and the dials between them.
//!
//! This module owns the *maths*, not the audio hardware. The window writes
//! parameters into atomics, the audio callbacks read them, and neither side
//! waits for the other. That split is deliberate: a lock taken in a render
//! callback is how mixers get clicks, so nothing here has one.
//!
//! Three things live here:
//!
//! * [`Mixer`] — the parameters and the meters, all atomics.
//! * [`Ring`] / [`Reader`] — a one-writer, many-reader sample store. Each input
//!   device pushes captured frames once; every output stream pulls from it at
//!   its own rate, so eight buses can read the same mic without copying it.
//! * [`StripEq`], [`Gate`] and [`Compressor`], tied together by [`StripChain`] —
//!   the channel strip run per output stream.
//! * [`Limiter`] — the ceiling on every bus, so a strip somebody boosted can
//!   never reach the devices as a squared-off waveform.
//!
//! Everything runs at [`RATE`]. Devices that disagree are resampled on the way
//! in or on the way out, by the readers' fractional step, never by changing the
//! mixing maths.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

pub const STRIPS: usize = 8;
pub const BUSES: usize = 8;
/// The rate everything is mixed at. 48 kHz is what the virtual cables use.
pub const RATE: u32 = 48_000;

pub const GAIN_MIN: f32 = -60.0;
pub const GAIN_MAX: f32 = 12.0;

/// How far a strip's input trim can lift or pull down the signal. Wider than the
/// fader's top because it is there to meet a quiet device: a microphone that
/// arrives 20 dB low is the case this control exists for.
pub const TRIM_MIN: f32 = -24.0;
pub const TRIM_MAX: f32 = 24.0;

const RELAXED: Ordering = Ordering::Relaxed;
const ACQUIRE: Ordering = Ordering::Acquire;
const RELEASE: Ordering = Ordering::Release;

#[inline]
fn bits(value: f32) -> u32 {
    value.to_bits()
}

#[inline]
fn float(value: u32) -> f32 {
    f32::from_bits(value)
}

/// dB to a linear multiplier. The bottom of the fader is silence, not -60 dB.
#[inline]
pub fn to_linear(db: f32) -> f32 {
    if db <= GAIN_MIN + 0.05 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Peak level of a block, linear.
#[inline]
pub fn peak(frames: &[[f32; 2]]) -> f32 {
    frames
        .iter()
        .fold(0.0f32, |acc, f| acc.max(f[0].abs()).max(f[1].abs()))
}

/// One input strip's settings.
pub struct StripParams {
    gain: AtomicU32,
    /// Level into the strip in dB, ahead of the cut, the gate, the bands and the
    /// compressor. The fader cannot serve this purpose: it sits after the chain,
    /// so it changes what a listener hears and not what the gate hears.
    trim: AtomicU32,
    /// How much noise the suppressor takes out: 0 is off, 1 is the full depth.
    denoiser: AtomicU32,
    mute: AtomicBool,
    solo: AtomicBool,
    mono: AtomicBool,
    /// One bit per bus, in `A1..B3` order.
    routes: AtomicU32,
    /// Front / Rear / Center / LFE, for the virtual inputs' channel spread.
    position: AtomicU32,
    eq_on: AtomicBool,
    /// True once a person has set this strip's EQ, so the mic defaults are not
    /// put back over their choices on the next launch.
    eq_touched: AtomicBool,
    /// Low, mid, high and body band gain in dB.
    eq: [AtomicU32; 4],
    /// Low-cut corner in Hz; 0 means no cut.
    cut: AtomicU32,
    gate_on: AtomicBool,
    comp_on: AtomicBool,
    /// True once a person has set this strip's gate or compressor, so the mic
    /// defaults are not put back over their choices on the next launch.
    dynamics_touched: AtomicBool,
    /// Threshold in dB, attack, release and hold in ms, and how far the gate
    /// pulls the signal down when it is shut. See [`GATE_FIELDS`].
    gate: [AtomicU32; GATE_FIELDS.len()],
    /// Threshold in dB, ratio, attack and release in ms, knee in dB, input trim
    /// and makeup gain. See [`COMP_FIELDS`].
    comp: [AtomicU32; COMP_FIELDS.len()],
}

/// The gate's controls, in the order they are stored.
///
/// `Damping` is how far the gate pulls the signal down while it is shut: -60 dB
/// is silence, -10 dB is a gentle expander that only takes the edge off. The
/// name is the one the console has always used for this control.
pub const GATE_FIELDS: [&str; 5] = ["Threshold", "Attack", "Release", "Hold", "Damping"];
/// The compressor's controls, in the order they are stored. `GainIn` trims the
/// signal going in, `GainOut` is the makeup gain coming out.
pub const COMP_FIELDS: [&str; 7] = [
    "Threshold",
    "Ratio",
    "Attack",
    "Release",
    "Knee",
    "GainIn",
    "GainOut",
];

/// What each control accepts, so a slider cannot ask for a ratio of zero or a
/// release longer than a coffee break.
const GATE_RANGE: [(f32, f32); 5] = [
    (-60.0, 0.0),
    (0.0, 200.0),
    (0.0, 2_000.0),
    (0.0, 2_000.0),
    (-60.0, 0.0),
];
const COMP_RANGE: [(f32, f32); 7] = [
    (-60.0, 0.0),
    (1.0, 20.0),
    (0.0, 200.0),
    (0.0, 2_000.0),
    (0.0, 24.0),
    (-12.0, 12.0),
    (-12.0, 24.0),
];

impl Default for StripParams {
    fn default() -> Self {
        StripParams {
            gain: AtomicU32::new(bits(0.0)),
            trim: AtomicU32::new(bits(0.0)),
            denoiser: AtomicU32::new(bits(0.0)),
            mute: AtomicBool::new(false),
            solo: AtomicBool::new(false),
            mono: AtomicBool::new(false),
            routes: AtomicU32::new(0),
            position: AtomicU32::new(0),
            eq_on: AtomicBool::new(false),
            eq_touched: AtomicBool::new(false),
            eq: [
                AtomicU32::new(bits(0.0)),
                AtomicU32::new(bits(0.0)),
                AtomicU32::new(bits(0.0)),
                AtomicU32::new(bits(0.0)),
            ],
            cut: AtomicU32::new(bits(0.0)),
            gate_on: AtomicBool::new(false),
            comp_on: AtomicBool::new(false),
            dynamics_touched: AtomicBool::new(false),
            // Resting values: a gate wide open, a compressor that would barely
            // move anything. A microphone is seeded with the voice chain in
            // `lib.rs`; these are what an unassigned or virtual strip reports.
            gate: [
                AtomicU32::new(bits(-60.0)),
                AtomicU32::new(bits(0.0)),
                AtomicU32::new(bits(220.0)),
                AtomicU32::new(bits(160.0)),
                AtomicU32::new(bits(0.0)),
            ],
            comp: [
                AtomicU32::new(bits(-16.0)),
                AtomicU32::new(bits(2.5)),
                AtomicU32::new(bits(11.0)),
                AtomicU32::new(bits(180.0)),
                AtomicU32::new(bits(6.0)),
                AtomicU32::new(bits(0.0)),
                AtomicU32::new(bits(0.0)),
            ],
        }
    }
}

impl StripParams {
    pub fn gain(&self) -> f32 {
        float(self.gain.load(RELAXED))
    }

    pub fn set_gain(&self, db: f32) {
        self.gain.store(bits(db.clamp(GAIN_MIN, GAIN_MAX)), RELAXED);
    }

    /// Level into the strip in dB, before anything else in the chain.
    pub fn trim(&self) -> f32 {
        float(self.trim.load(RELAXED))
    }

    pub fn set_trim(&self, db: f32) {
        self.trim.store(bits(db.clamp(TRIM_MIN, TRIM_MAX)), RELAXED);
    }

    /// How much noise the suppressor takes out: 0 is off, 1 is the full depth.
    pub fn denoiser(&self) -> f32 {
        float(self.denoiser.load(RELAXED)).clamp(0.0, 1.0)
    }

    pub fn set_denoiser(&self, amount: f32) {
        self.denoiser.store(bits(amount.clamp(0.0, 1.0)), RELAXED);
    }

    pub fn mute(&self) -> bool {
        self.mute.load(RELAXED)
    }

    pub fn set_mute(&self, on: bool) {
        self.mute.store(on, RELAXED);
    }

    pub fn solo(&self) -> bool {
        self.solo.load(RELAXED)
    }

    pub fn set_solo(&self, on: bool) {
        self.solo.store(on, RELAXED);
    }

    pub fn mono(&self) -> bool {
        self.mono.load(RELAXED)
    }

    pub fn set_mono(&self, on: bool) {
        self.mono.store(on, RELAXED);
    }

    pub fn routes(&self) -> u32 {
        self.routes.load(RELAXED)
    }

    pub fn routed_to(&self, bus: usize) -> bool {
        self.routes() & (1 << bus) != 0
    }

    pub fn set_routed(&self, bus: usize, on: bool) {
        let mask = 1u32 << bus;
        let mut current = self.routes();
        loop {
            let next = if on { current | mask } else { current & !mask };
            match self
                .routes
                .compare_exchange_weak(current, next, RELAXED, RELAXED)
            {
                Ok(_) => return,
                Err(seen) => current = seen,
            }
        }
    }

    pub fn position(&self) -> u32 {
        self.position.load(RELAXED)
    }

    pub fn position_at(&self, axis: usize) -> bool {
        self.position() & (1 << axis) != 0
    }

    pub fn set_position(&self, axis: usize, on: bool) {
        let mask = 1u32 << axis;
        let mut current = self.position();
        loop {
            let next = if on { current | mask } else { current & !mask };
            match self
                .position
                .compare_exchange_weak(current, next, RELAXED, RELAXED)
            {
                Ok(_) => return,
                Err(seen) => current = seen,
            }
        }
    }

    pub fn eq_on(&self) -> bool {
        self.eq_on.load(RELAXED)
    }

    pub fn set_eq_on(&self, on: bool) {
        self.eq_on.store(on, RELAXED);
    }

    pub fn eq_touched(&self) -> bool {
        self.eq_touched.load(RELAXED)
    }

    pub fn mark_eq_touched(&self) {
        self.eq_touched.store(true, RELAXED);
    }

    /// Band gain in dB: 0 treble, 1 mid, 2 bass, 3 body.
    pub fn eq(&self, band: usize) -> f32 {
        float(self.eq[band.min(3)].load(RELAXED))
    }

    pub fn set_eq(&self, band: usize, db: f32) {
        self.eq[band.min(3)].store(bits(db.clamp(-12.0, 12.0)), RELAXED);
    }

    /// Low-cut corner in Hz, 0 when off.
    pub fn cut(&self) -> f32 {
        float(self.cut.load(RELAXED))
    }

    pub fn set_cut(&self, hz: f32) {
        let hz = if hz < 20.0 { 0.0 } else { hz.clamp(20.0, 400.0) };
        self.cut.store(bits(hz), RELAXED);
    }

    pub fn gate_on(&self) -> bool {
        self.gate_on.load(RELAXED)
    }

    pub fn set_gate_on(&self, on: bool) {
        self.gate_on.store(on, RELAXED);
    }

    pub fn comp_on(&self) -> bool {
        self.comp_on.load(RELAXED)
    }

    pub fn set_comp_on(&self, on: bool) {
        self.comp_on.store(on, RELAXED);
    }

    pub fn dynamics_touched(&self) -> bool {
        self.dynamics_touched.load(RELAXED)
    }

    pub fn mark_dynamics_touched(&self) {
        self.dynamics_touched.store(true, RELAXED);
    }

    /// One gate control by position in [`GATE_FIELDS`].
    pub fn gate(&self, field: usize) -> f32 {
        let index = field.min(GATE_FIELDS.len() - 1);
        float(self.gate[index].load(RELAXED))
    }

    pub fn set_gate(&self, field: usize, value: f32) {
        let index = field.min(GATE_FIELDS.len() - 1);
        let (low, high) = GATE_RANGE[index];
        self.gate[index].store(bits(value.clamp(low, high)), RELAXED);
    }

    /// One compressor control by position in [`COMP_FIELDS`].
    pub fn comp(&self, field: usize) -> f32 {
        let index = field.min(COMP_FIELDS.len() - 1);
        float(self.comp[index].load(RELAXED))
    }

    pub fn set_comp(&self, field: usize, value: f32) {
        let index = field.min(COMP_FIELDS.len() - 1);
        let (low, high) = COMP_RANGE[index];
        self.comp[index].store(bits(value.clamp(low, high)), RELAXED);
    }

    /// One gate control by the name the console writes, `Hold` or `Damping`.
    /// Returns whether the name was one it knows.
    pub fn set_gate_named(&self, name: &str, value: f32) -> bool {
        match GATE_FIELDS.iter().position(|field| *field == name) {
            Some(index) => {
                self.set_gate(index, value);
                true
            }
            None => false,
        }
    }

    /// One compressor control by name, `Threshold` or `GainOut`.
    pub fn set_comp_named(&self, name: &str, value: f32) -> bool {
        match COMP_FIELDS.iter().position(|field| *field == name) {
            Some(index) => {
                self.set_comp(index, value);
                true
            }
            None => false,
        }
    }
}

/// One output bus's settings.
pub struct BusParams {
    gain: AtomicU32,
    mute: AtomicBool,
    mono: AtomicBool,
    limit: AtomicBool,
}

impl Default for BusParams {
    fn default() -> Self {
        BusParams {
            gain: AtomicU32::new(bits(0.0)),
            mute: AtomicBool::new(false),
            mono: AtomicBool::new(false),
            // On from the first launch: it does nothing below the ceiling, and
            // above it the alternative is a hard clip on somebody's cable.
            limit: AtomicBool::new(true),
        }
    }
}

impl BusParams {
    pub fn gain(&self) -> f32 {
        float(self.gain.load(RELAXED))
    }

    pub fn set_gain(&self, db: f32) {
        self.gain.store(bits(db.clamp(GAIN_MIN, GAIN_MAX)), RELAXED);
    }

    pub fn mute(&self) -> bool {
        self.mute.load(RELAXED)
    }

    pub fn set_mute(&self, on: bool) {
        self.mute.store(on, RELAXED);
    }

    pub fn mono(&self) -> bool {
        self.mono.load(RELAXED)
    }

    pub fn set_mono(&self, on: bool) {
        self.mono.store(on, RELAXED);
    }

    pub fn limit_on(&self) -> bool {
        self.limit.load(RELAXED)
    }

    pub fn set_limit(&self, on: bool) {
        self.limit.store(on, RELAXED);
    }
}

/// A decaying peak, held as an `f32` in an atomic.
fn publish_peak(slot: &AtomicU32, block_peak: f32) {
    let decayed = float(slot.load(RELAXED)) * 0.82;
    let next = block_peak.max(decayed).clamp(0.0, 4.0);
    slot.store(bits(next), RELAXED);
}

/// Everything the audio callbacks and the window share.
pub struct Mixer {
    pub strips: [StripParams; STRIPS],
    pub buses: [BusParams; BUSES],
    strip_peak: [AtomicU32; STRIPS],
    bus_peak: [AtomicU32; BUSES],
    bus_post: [AtomicU32; BUSES],
    /// Frames per capture callback, as the devices actually deliver them.
    block: AtomicU32,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer {
            strips: Default::default(),
            buses: Default::default(),
            strip_peak: Default::default(),
            bus_peak: Default::default(),
            bus_post: Default::default(),
            block: AtomicU32::new(0),
        }
    }
}

impl Mixer {
    /// True while any strip is soloed. A render callback asks this once a block.
    pub fn soloing(&self) -> bool {
        self.strips.iter().any(|s| s.solo())
    }

    pub fn set_strip_peak(&self, strip: usize, value: f32) {
        if let Some(slot) = self.strip_peak.get(strip) {
            publish_peak(slot, value);
        }
    }

    /// `value` is the level leaving the bus fader, which is what its meter
    /// reads. It used to be the sum before the fader, so pulling a bus down left
    /// its meter where it was and the meter could not confirm silence.
    ///
    /// `pre_ceiling` is the same level while the ceiling is holding the bus
    /// down, so comparing it with [`ceiling`] answers "was this bus limited",
    /// which is what the lamp means. It is zero when the limiter is off, because
    /// then nothing is holding the bus down. The accessor that reads it is still
    /// called `bus_post` because the console's levels message is named that way;
    /// the value itself is not a post-limiter one.
    pub fn set_bus_peak(&self, bus: usize, value: f32, pre_ceiling: f32) {
        if let Some(slot) = self.bus_peak.get(bus) {
            publish_peak(slot, value);
        }
        if let Some(slot) = self.bus_post.get(bus) {
            publish_peak(slot, pre_ceiling);
        }
    }

    pub fn strip_peak(&self, strip: usize) -> f32 {
        self.strip_peak
            .get(strip)
            .map(|slot| float(slot.load(RELAXED)))
            .unwrap_or(0.0)
    }

    pub fn bus_peak(&self, bus: usize) -> f32 {
        self.bus_peak
            .get(bus)
            .map(|slot| float(slot.load(RELAXED)))
            .unwrap_or(0.0)
    }

    pub fn bus_post(&self, bus: usize) -> f32 {
        self.bus_post
            .get(bus)
            .map(|slot| float(slot.load(RELAXED)))
            .unwrap_or(0.0)
    }

    /// Reported by a capture callback, so the window can say how much of the
    /// delay is the device's own period rather than a made-up number.
    pub fn set_block(&self, frames: usize) {
        self.block.store(frames as u32, RELAXED);
    }

    pub fn block(&self) -> u32 {
        self.block.load(RELAXED)
    }
}

/// How much slack a reader keeps behind the writer, in milliseconds.
///
/// This is the number that decides how long a sound takes to get from a cable
/// to the speakers, so it starts small — 20 ms is two WASAPI periods, enough to
/// absorb ordinary jitter. A reader that actually runs dry raises it by 5 ms (up
/// to a ceiling) instead of clicking, so a slower machine tunes itself up rather
/// than starting every user at the safe, laggy end.
pub const LATENCY_MIN_MS: u64 = 20;
pub const LATENCY_MAX_MS: u64 = 60;

/// The live target, shared by every reader in every stream.
static TARGET_MS: AtomicU64 = AtomicU64::new(LATENCY_MIN_MS);

#[inline]
fn frames_for(ms: u64) -> u64 {
    RATE as u64 * ms / 1000
}

/// The buffer the readers are shooting for right now, in milliseconds.
pub fn latency_ms() -> u64 {
    TARGET_MS.load(RELAXED)
}

/// Frames of slack the readers are shooting for right now.
pub fn target_frames() -> u64 {
    frames_for(latency_ms())
}

/// Called by a reader that could not fill a block: there was not enough audio
/// waiting, so it gives itself another 5 ms rather than underrun again.
fn grow_latency() {
    let now = latency_ms();
    if now < LATENCY_MAX_MS {
        TARGET_MS.store((now + 5).min(LATENCY_MAX_MS), Ordering::Relaxed);
    }
}

/// Back to the low end. Only the tests need this; a running session only ever
/// tunes up, because shrinking under load is how you get clicking.
#[cfg(test)]
pub fn reset_latency() {
    TARGET_MS.store(LATENCY_MIN_MS, RELAXED);
}

/// Samples as atomics: a writer publishes an index, readers chase it.
pub struct Ring {
    cells: Box<[[AtomicU32; 2]]>,
    write: AtomicU64,
    mask: u64,
}

impl Ring {
    /// `seconds` of stereo audio, rounded up to a power of two.
    pub fn new(seconds: u32) -> Ring {
        let want = (RATE * seconds) as usize;
        let capacity = want.next_power_of_two();
        let cells = (0..capacity)
            .map(|_| [AtomicU32::new(0), AtomicU32::new(0)])
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ring {
            cells,
            write: AtomicU64::new(0),
            mask: (capacity - 1) as u64,
        }
    }

    pub fn capacity(&self) -> u64 {
        self.mask + 1
    }

    /// Publish a captured block. One writer, as the name implies.
    pub fn push(&self, frames: &[[f32; 2]]) {
        let mut index = self.write.load(RELAXED);
        for frame in frames {
            let slot = &self.cells[(index & self.mask) as usize];
            slot[0].store(bits(frame[0]), RELAXED);
            slot[1].store(bits(frame[1]), RELAXED);
            index += 1;
        }
        self.write.store(index, RELEASE);
    }

    fn written(&self) -> u64 {
        self.write.load(ACQUIRE)
    }

    fn at(&self, index: u64) -> [f32; 2] {
        let slot = &self.cells[(index & self.mask) as usize];
        [float(slot[0].load(RELAXED)), float(slot[1].load(RELAXED))]
    }

    #[cfg(test)]
    pub fn reader(&self) -> Reader {
        Reader::idle()
    }
}

/// One consumer's place in a [`Ring`], with a fractional step so a device at any
/// rate can read a 48 kHz ring (and back the other way).
pub struct Reader {
    position: f64,
    /// Set once a pull has been served in full, so the empty ring at startup is
    /// not mistaken for a machine that cannot keep up.
    started: bool,
}

impl Reader {
    /// A reader that has not started yet: its first pull lands on the live edge.
    pub fn idle() -> Reader {
        Reader {
            position: -1.0,
            started: false,
        }
    }

    /// Fill `out` from `ring`, advancing by `step` ring frames per output frame.
    ///
    /// `step` is `RATE / device_rate`. A reader that falls behind the window,
    /// or catches up with the writer, is nudged back to the latency target
    /// instead of being left to click. A reader that runs dry raises the shared
    /// target, which is how the whole console settles on the smallest buffer its
    /// machine can sustain.
    pub fn pull(&mut self, ring: &Ring, frames: usize, step: f64, out: &mut [[f32; 2]]) {
        out.fill([0.0, 0.0]);
        let written = ring.written();
        let capacity = ring.capacity();
        let latency = target_frames();

        if self.position < 0.0 {
            self.position = written as f64 - latency as f64;
        }
        if self.position < written as f64 - capacity as f64 + 2.0 {
            self.position = written as f64 - latency as f64;
        }
        if self.position > written as f64 - 1.0 {
            self.position = written as f64 - 1.0;
        }
        if self.position < 0.0 {
            self.position = 0.0;
        }

        let mut served = 0;
        for slot in out.iter_mut().take(frames) {
            let base = self.position.floor();
            if base + 1.0 > written as f64 {
                // The writer has not reached us yet: hold, and do not advance,
                // so the next callback picks up exactly where this one stopped.
                break;
            }
            let index = base as u64;
            let fraction = (self.position - base) as f32;
            let a = ring.at(index);
            let b = ring.at(index + 1);
            *slot = [
                a[0] + (b[0] - a[0]) * fraction,
                a[1] + (b[1] - a[1]) * fraction,
            ];
            self.position += step;
            served += 1;
        }

        if served == frames {
            self.started = true;
        } else if self.started && written > latency {
            // Not the startup silence: the writer is behind where we needed it.
            grow_latency();
        }
    }
}

/// A direct-form II transposed biquad, stereo state.
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: [f32; 2],
    z2: [f32; 2],
}

impl Biquad {
    /// A 12 dB/octave Butterworth high-pass, for the low cut.
    ///
    /// Separate from [`Biquad::design`] because a cut has no gain: it is the
    /// corner frequency that is the setting, and `0 dB` would mean "passthrough"
    /// to the other filters.
    fn design_cut(&mut self, frequency: f32) {
        if frequency <= 0.0 {
            self.reset();
            return;
        }
        let w = 2.0 * std::f32::consts::PI * frequency / RATE as f32;
        let cos = w.cos();
        let sin = w.sin();
        let alpha = sin / (2.0 * 0.707f32);
        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 + cos) * 0.5) / a0;
        self.b1 = -(1.0 + cos) / a0;
        self.b2 = ((1.0 + cos) * 0.5) / a0;
        self.a1 = (-2.0 * cos) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    /// Back to passing the signal through untouched.
    fn reset(&mut self) {
        self.b0 = 1.0;
        self.b1 = 0.0;
        self.b2 = 0.0;
        self.a1 = 0.0;
        self.a2 = 0.0;
    }

    /// `kind`: 0 low shelf, 1 peak, 2 high shelf. `gain` in dB.
    fn design(&mut self, kind: u8, frequency: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * std::f32::consts::PI * frequency / RATE as f32;
        let cos = w.cos();
        let sin = w.sin();
        let q = 0.707f32;
        let alpha = sin / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = if gain_db.abs() < 0.01 {
            (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
        } else {
            match kind {
                0 => {
                    let sqrt_a = a.sqrt();
                    (
                        a * ((a + 1.0) - (a - 1.0) * cos + 2.0 * sqrt_a * alpha),
                        2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                        a * ((a + 1.0) - (a - 1.0) * cos - 2.0 * sqrt_a * alpha),
                        (a + 1.0) + (a - 1.0) * cos + 2.0 * sqrt_a * alpha,
                        -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                        (a + 1.0) + (a - 1.0) * cos - 2.0 * sqrt_a * alpha,
                    )
                }
                2 => {
                    let sqrt_a = a.sqrt();
                    (
                        a * ((a + 1.0) + (a - 1.0) * cos + 2.0 * sqrt_a * alpha),
                        -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                        a * ((a + 1.0) + (a - 1.0) * cos - 2.0 * sqrt_a * alpha),
                        (a + 1.0) - (a - 1.0) * cos + 2.0 * sqrt_a * alpha,
                        2.0 * ((a - 1.0) - (a + 1.0) * cos),
                        (a + 1.0) - (a - 1.0) * cos - 2.0 * sqrt_a * alpha,
                    )
                }
                _ => (
                    1.0 + alpha * a,
                    -2.0 * cos,
                    1.0 - alpha * a,
                    1.0 + alpha / a,
                    -2.0 * cos,
                    1.0 - alpha / a,
                ),
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    pub fn process(&mut self, frame: [f32; 2]) -> [f32; 2] {
        let mut out = [0.0f32; 2];
        for channel in 0..2 {
            let x = frame[channel];
            let y = self.b0 * x + self.z1[channel];
            self.z1[channel] = self.b1 * x - self.a1 * y + self.z2[channel];
            self.z2[channel] = self.b2 * x - self.a2 * y;
            out[channel] = y;
        }
        out
    }
}

/// dB to a linear multiplier, without the fader's "the bottom of the throw is
/// silence" rule. A threshold of -60 dB is a very small number, not zero.
#[inline]
fn from_db(value: f32) -> f32 {
    10f32.powf(value / 20.0)
}

/// Linear to dB, floored far below anything a device can deliver.
#[inline]
fn to_db(linear: f32) -> f32 {
    20.0 * linear.max(1e-7).log10()
}

/// The level a chain adopts when it is switched on: the loudest of the first
/// frames of the block, so a gate or a compressor coming on mid-sentence starts
/// from where the audio actually is instead of fading in from silence.
fn head(frames: &[[f32; 2]]) -> f32 {
    peak(&frames[..frames.len().min(64)])
}

/// A one-pole's per-frame coefficient for a time constant in milliseconds.
///
/// Zero milliseconds is instant: the coefficient is one, so a control wound to
/// its fastest copies the level straight through instead of dividing by zero.
#[inline]
fn glide(ms: f32) -> f32 {
    if ms <= 0.0 {
        1.0
    } else {
        let seconds = ms / 1000.0;
        (1.0 - (-1.0 / (seconds * RATE as f32)).exp()).clamp(0.0, 1.0)
    }
}

/// How far above the learned floor a signal is left alone, in dB.
const NOISE_MARGIN_DB: f32 = 6.0;

/// A broadband noise suppressor.
///
/// The gate shuts an input between phrases. This works while somebody is
/// talking: it learns the quietest thing the input hears, treats that as the
/// room, and pulls the signal down as it approaches that floor. Hiss, a fan and
/// a noisy preamp come down. A voice sits well above the floor and is left
/// alone.
///
/// It is broadband rather than spectral, which is what keeps it cheap enough to
/// run on every strip of every output stream. The reduction is smooth rather
/// than switched, so the room does not pump between words, and the control
/// scales the whole thing, so at 0 it is a multiply by exactly one.
#[derive(Clone, Copy, Default)]
pub struct Denoiser {
    /// The quietest level seen, in dBFS. It falls quickly and rises slowly, so
    /// a room is learned in a second and a word never becomes the floor.
    floor: f32,
    /// Smoothed signal level in dBFS.
    level: f32,
    /// The reduction being applied, in dB, zero or negative.
    gain_db: f32,
    /// False until a frame has been seen, so the first block adopts the level it
    /// finds instead of starting from an invented floor.
    live: bool,
}

impl Denoiser {
    pub fn process(&mut self, strip: &StripParams, frames: &mut [[f32; 2]]) {
        let amount = strip.denoiser();
        if amount <= 0.0 {
            self.live = false;
            return;
        }
        // The most that is taken out at the floor, scaled by the control.
        let depth = 30.0 * amount;
        let rise = glide(4.0);
        let fall = glide(120.0);
        let learn_quickly = glide(30.0);
        let learn_slowly = glide(4_000.0);
        let smooth = glide(40.0);

        if !self.live {
            let first = to_db(head(frames));
            self.floor = first;
            self.level = first;
            self.gain_db = 0.0;
            self.live = true;
        }

        for frame in frames.iter_mut() {
            let key = to_db(frame[0].abs().max(frame[1].abs()));
            let coeff = if key > self.level { rise } else { fall };
            self.level += (key - self.level) * coeff;

            // Follow anything quieter at once, so the room settles, and rise
            // slowly, so speech is never mistaken for the noise around it.
            let coeff = if self.level < self.floor {
                learn_quickly
            } else {
                learn_slowly
            };
            self.floor += (self.level - self.floor) * coeff;

            let over = self.level - self.floor;
            let target = if over >= NOISE_MARGIN_DB {
                0.0
            } else if over <= 0.0 {
                -depth
            } else {
                -depth * (1.0 - over / NOISE_MARGIN_DB)
            };
            self.gain_db += (target - self.gain_db) * smooth;
            let gain = from_db(self.gain_db);
            *frame = [frame[0] * gain, frame[1] * gain];
        }
    }
}

/// A noise gate: shut while nobody is talking, open while somebody is.
///
/// It follows the loudest of the two channels, so the two never disagree about
/// whether the gate is open, and it holds before it starts to close — that is
/// what stops it chattering in the gaps between words. `Damping` is how far it
/// pulls the signal down when shut: -60 dB is silence, -10 dB is a gentle
/// expander that only takes the edge off the room.
#[derive(Clone, Copy, Default)]
pub struct Gate {
    /// Level estimate, linear.
    detector: f32,
    /// The multiplier being applied, moved towards its target by the times.
    openness: f32,
    /// Milliseconds of hold left before the gate may close.
    holding: f32,
    /// False until the gate has seen a frame, so switching it on adopts
    /// whatever the level is rather than fading in from where it left off.
    live: bool,
}

impl Gate {
    pub fn process(&mut self, strip: &StripParams, frames: &mut [[f32; 2]]) {
        if !strip.gate_on() {
            self.live = false;
            return;
        }
        let threshold = from_db(strip.gate(0));
        let open = glide(strip.gate(1));
        let close = glide(strip.gate(2));
        let hold = strip.gate(3) * RATE as f32 / 1000.0;
        let shut = from_db(strip.gate(4));
        // Fast enough to catch a consonant, slow enough not to follow the
        // waveform itself and let the gate open on every cycle of a hum.
        let rise = glide(1.0);
        let fall = glide(30.0);

        if !self.live {
            self.detector = head(frames);
            self.openness = if self.detector > threshold { 1.0 } else { shut };
            self.holding = 0.0;
            self.live = true;
        }

        for frame in frames.iter_mut() {
            let key = frame[0].abs().max(frame[1].abs());
            let coeff = if key > self.detector { rise } else { fall };
            self.detector += (key - self.detector) * coeff;

            let speaking = self.detector > threshold;
            if speaking {
                self.holding = hold;
            } else if self.holding > 0.0 {
                self.holding -= 1.0;
            }
            let target = if speaking || self.holding > 0.0 { 1.0 } else { shut };
            let coeff = if target > self.openness { open } else { close };
            self.openness += (target - self.openness) * coeff;

            *frame = [frame[0] * self.openness, frame[1] * self.openness];
        }
    }
}

/// A feed-forward compressor with a soft knee.
///
/// The level is followed in decibels with the attack and release times, and
/// above the threshold the curve takes `1 - 1/ratio` of every decibel over it.
/// The knee bends that corner over its own width, which is what keeps a voice
/// that sits across the threshold from sounding switched rather than ridden.
#[derive(Clone, Copy, Default)]
pub struct Compressor {
    /// Level estimate in dB.
    detector: f32,
    /// False until the compressor has seen a frame, so switching it on adopts
    /// the live level instead of starting out at full scale.
    live: bool,
}

impl Compressor {
    pub fn process(&mut self, strip: &StripParams, frames: &mut [[f32; 2]]) {
        if !strip.comp_on() {
            self.live = false;
            return;
        }
        let threshold = strip.comp(0);
        let slope = 1.0 - 1.0 / strip.comp(1).max(1.0);
        let attack = glide(strip.comp(2));
        let release = glide(strip.comp(3));
        let knee = strip.comp(4).max(0.0);
        let trim = from_db(strip.comp(5));
        let makeup = strip.comp(6);
        let half = knee * 0.5;

        if !self.live {
            self.detector = to_db(head(frames) * trim);
            self.live = true;
        }

        for frame in frames.iter_mut() {
            // The input trim is a pre-gain: the detector listens after it, which
            // is what makes it worth having a trim at all.
            let sample = [frame[0] * trim, frame[1] * trim];
            let key = to_db(sample[0].abs().max(sample[1].abs()));
            let coeff = if key > self.detector { attack } else { release };
            self.detector += (key - self.detector) * coeff;

            let over = self.detector - threshold;
            let reduction = if over <= -half {
                0.0
            } else if over >= half {
                over * slope
            } else {
                slope * (over + half).powi(2) / (2.0 * knee.max(1e-6))
            };
            let gain = from_db(makeup - reduction);
            *frame = [sample[0] * gain, sample[1] * gain];
        }
    }
}

/// The ceiling every bus is held under, in dBFS.
///
/// Just under full scale rather than at it: a cable carries 24-bit samples, and
/// the programs on the other side treat 0 dBFS as the edge, so the last third of
/// a decibel is not worth having. What matters is that nothing arrives squared
/// off, which is what the output clamp used to do.
pub const LIMIT_CEILING_DB: f32 = -0.3;

/// How far ahead the limiter looks, at 48 kHz. Five milliseconds is enough to
/// have the gain down before a transient arrives, which is why the waveform is
/// turned down instead of shaved.
pub const LIMIT_LOOKAHEAD: usize = 240;

/// How long the gain takes to come back after a peak, in milliseconds.
pub const LIMIT_RELEASE_MS: f32 = 80.0;

/// The ceiling as a sample value, which is what the limiter compares against and
/// what the meter's lamp is handed.
///
/// One function, because there is one ceiling: the lamp deciding "the limiter is
/// working" must ask the same number the limiter does, or a bus can be held down
/// with nothing on screen saying so.
pub fn ceiling() -> f32 {
    from_db(LIMIT_CEILING_DB)
}

/// A look-ahead peak limiter, one per output bus.
///
/// The last thing between the mix and a device. Strips can be equalised up and
/// compressed with makeup gain, and a program that leaves a cable above full
/// scale is clipped by every program that records it — so the bus holds a
/// ceiling instead.
///
/// The audio is delayed by [`LIMIT_LOOKAHEAD`] frames while the gain for it is
/// worked out, and the gain applied to a frame is the *minimum* asked for by
/// every frame still in that window. That minimum is what makes this a limiter
/// rather than a clipper: a sample can never leave louder than its own `need`
/// allows, and the gain is already down when the transient lands.
pub struct Limiter {
    /// The look-ahead line, in frames.
    delay: Vec<[f32; 2]>,
    cursor: usize,
    /// `(frame number, gain needed)`, kept increasing in gain: the front is the
    /// smallest gain anything still in the delay line requires.
    window: VecDeque<(u64, f32)>,
    /// Frames seen so far, which names the frame leaving the delay line.
    frame: u64,
    /// The gain being applied, smoothed on the way back up only.
    gain: f32,
    /// Per-frame coefficient for that release.
    release: f32,
    ceiling: f32,
}

impl Default for Limiter {
    fn default() -> Self {
        Limiter::new()
    }
}

impl Limiter {
    pub fn new() -> Limiter {
        Limiter {
            delay: vec![[0.0; 2]; LIMIT_LOOKAHEAD.max(1)],
            cursor: 0,
            window: VecDeque::with_capacity(LIMIT_LOOKAHEAD + 2),
            frame: 0,
            gain: 1.0,
            release: glide(LIMIT_RELEASE_MS),
            ceiling: ceiling(),
        }
    }

    /// The gain being applied. Only the tests ask, so it is not compiled into
    /// the release binary where nothing would read it.
    #[cfg(test)]
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Forget the peak in flight. Called when the limiter is switched off, so
    /// switching it back on does not duck the first block.
    pub fn reset(&mut self) {
        self.window.clear();
        self.delay.fill([0.0; 2]);
        self.cursor = 0;
        self.frame = 0;
        self.gain = 1.0;
    }

    pub fn process(&mut self, frames: &mut [[f32; 2]]) {
        let ahead = self.delay.len() as u64;
        for frame in frames.iter_mut() {
            let peak = frame[0].abs().max(frame[1].abs());
            let need = if peak > self.ceiling {
                self.ceiling / peak
            } else {
                1.0
            };

            // Publish what the frame `ahead` frames from now will need. Anything
            // that needs less than this one can never be the minimum again.
            let arrives = self.frame + ahead;
            while self.window.back().is_some_and(|last| last.1 >= need) {
                self.window.pop_back();
            }
            self.window.push_back((arrives, need));

            // Everything already written out is no longer this frame's problem.
            while self.window.front().is_some_and(|first| first.0 < self.frame) {
                self.window.pop_front();
            }
            let required = self.window.front().map_or(1.0, |first| first.1);

            // Up slowly, down at once: the release is what keeps a passage from
            // pumping, and `required` is what keeps the ceiling.
            self.gain += (1.0 - self.gain) * self.release;
            if self.gain > required {
                self.gain = required;
            }

            let delayed = self.delay[self.cursor];
            self.delay[self.cursor] = *frame;
            self.cursor = (self.cursor + 1) % self.delay.len();
            *frame = [delayed[0] * self.gain, delayed[1] * self.gain];
            self.frame += 1;
        }
    }
}

/// One strip's equaliser, for one output stream.
///
/// Four bands plus a low cut: a shelf under the voice, a narrow cut where a
/// close microphone sounds boxy, a peak for presence and a shelf for air. The
/// corner frequencies are fixed — the settings that matter for speech are the
/// `Body` and `Low cut` controls, and fixed corners keep the numbers on the
/// dialog honest about what is happening.
///
/// The cut is kept apart from the bands so the gate can sit between them: a gate
/// that cannot hear the rumble is a gate that does not open on it.
#[derive(Clone, Copy, Default)]
pub struct StripEq {
    cut: Biquad,
    low: Biquad,
    body: Biquad,
    mid: Biquad,
    high: Biquad,
    shaped_for: [f32; 5],
    designed: bool,
}

impl StripEq {
    /// Design the coefficients if a control has moved, and say whether the
    /// equaliser is on at all.
    ///
    /// Coefficients only change when a control moves, so they are designed on
    /// the block that follows a change and left alone otherwise. The split
    /// between the cut and the bands means both halves share this one design.
    fn ready(&mut self, strip: &StripParams) -> bool {
        if !strip.eq_on() {
            self.designed = false;
            return false;
        }
        let shape = [
            strip.eq(0),
            strip.eq(1),
            strip.eq(2),
            strip.eq(3),
            strip.cut(),
        ];
        if !self.designed || shape != self.shaped_for {
            self.cut.design_cut(shape[4]);
            self.low.design(0, 120.0, shape[2]);
            self.body.design(1, 350.0, shape[3]);
            self.mid.design(1, 1_000.0, shape[1]);
            self.high.design(2, 6_000.0, shape[0]);
            self.shaped_for = shape;
            self.designed = true;
        }
        true
    }

    /// The low cut alone.
    pub fn process_cut(&mut self, strip: &StripParams, frames: &mut [[f32; 2]]) {
        if !self.ready(strip) {
            return;
        }
        for frame in frames.iter_mut() {
            *frame = self.cut.process(*frame);
        }
    }

    /// The four bands.
    pub fn process_bands(&mut self, strip: &StripParams, frames: &mut [[f32; 2]]) {
        if !self.ready(strip) {
            return;
        }
        for frame in frames.iter_mut() {
            *frame = self.low.process(*frame);
            *frame = self.body.process(*frame);
            *frame = self.mid.process(*frame);
            *frame = self.high.process(*frame);
        }
    }

    /// Both halves, in the order the strip runs them when nothing sits between.
    ///
    /// The channel strip calls the two halves separately so the gate can sit
    /// between them; this is how a test measures the equaliser on its own.
    #[cfg(test)]
    pub fn process(&mut self, strip: &StripParams, frames: &mut [[f32; 2]]) {
        self.process_cut(strip, frames);
        self.process_bands(strip, frames);
    }
}

/// One strip's channel strip, for one output stream.
///
/// The order is the one a channel strip uses, and each step is there for the
/// next one: the input trim sets the level everything else works at, the
/// denoiser takes the room down, the low cut removes the rumble the denoiser
/// should not be learning from, the gate shuts what is left between phrases, the
/// bands shape the voice, and the compressor finishes the job on the level the
/// bands produced.
#[derive(Clone, Copy, Default)]
pub struct StripChain {
    eq: StripEq,
    gate: Gate,
    comp: Compressor,
    denoise: Denoiser,
}

impl StripChain {
    pub fn process(&mut self, strip: &StripParams, frames: &mut [[f32; 2]]) {
        // The trim first, so the cut, the gate and the compressor all hear the
        // level the person set rather than the one the device happened to
        // deliver. At 0 dB this is a multiply by exactly one, so a strip nobody
        // has trimmed stays bit-for-bit untouched.
        let trim = from_db(strip.trim());
        for frame in frames.iter_mut() {
            *frame = [frame[0] * trim, frame[1] * trim];
        }
        self.denoise.process(strip, frames);
        self.eq.process_cut(strip, frames);
        self.gate.process(strip, frames);
        self.eq.process_bands(strip, frames);
        self.comp.process(strip, frames);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_a_bitmask_per_strip() {
        let strip = StripParams::default();
        strip.set_routed(0, true);
        strip.set_routed(7, true);
        assert!(strip.routed_to(0));
        assert!(strip.routed_to(7));
        assert!(!strip.routed_to(1));
        strip.set_routed(0, false);
        assert_eq!(strip.routes(), 1 << 7);
    }

    #[test]
    fn gain_bottom_is_silence_and_top_is_clamped() {
        assert_eq!(to_linear(GAIN_MIN), 0.0);
        assert!((to_linear(0.0) - 1.0).abs() < 1e-6);
        let strip = StripParams::default();
        strip.set_gain(40.0);
        assert_eq!(strip.gain(), GAIN_MAX);
    }

    /// At the learned floor the input is pulled down. A voice far above it is
    /// not, which is the whole point of learning the floor instead of pinning a
    /// threshold.
    #[test]
    fn the_denoiser_pulls_the_floor_down_and_leaves_a_voice() {
        let strip = StripParams::default();
        strip.set_denoiser(1.0);
        let mut denoise = Denoiser::default();

        // Two seconds of a steady, quiet room.
        let room = vec![[0.002f32, 0.002]; 960];
        let mut block = room.clone();
        let mut tail = 0.0;
        for _ in 0..100 {
            block.copy_from_slice(&room);
            denoise.process(&strip, &mut block);
            tail = peak(&block);
        }
        assert!(
            tail < 0.001,
            "the room should be at least 6 dB down after two seconds, got {tail}"
        );

        // A voice sits 40 dB above that floor and is left where it is. The gain
        // takes a moment to come back, so the last block is the one measured.
        let voice = vec![[0.2f32, 0.2]; 960];
        for _ in 0..10 {
            block.copy_from_slice(&voice);
            denoise.process(&strip, &mut block);
        }
        let out = peak(&block);
        assert!((out - 0.2).abs() < 0.02, "a voice should pass, got {out}");
    }

    #[test]
    fn the_denoiser_at_zero_is_passthrough_and_the_control_is_clamped() {
        let strip = StripParams::default();
        let mut denoise = Denoiser::default();
        let mut frames = vec![[0.123f32, -0.456]; 64];
        let before = frames.clone();
        denoise.process(&strip, &mut frames);
        assert_eq!(frames, before, "off is a passthrough, not a quiet version");

        strip.set_denoiser(4.0);
        assert_eq!(strip.denoiser(), 1.0);
        strip.set_denoiser(-1.0);
        assert_eq!(strip.denoiser(), 0.0);
    }

    #[test]
    fn a_reader_gets_what_was_pushed_after_the_latency_target() {
        reset_latency();
        // Two seconds of audio: the target can be anywhere in its range, and the
        // reader must still find real samples rather than the start of the ring.
        let ring = Ring::new(2);
        let frames: Vec<[f32; 2]> = (0..RATE as usize * 2)
            .map(|i| [i as f32 / 1000.0, 0.5])
            .collect();
        ring.push(&frames);

        let mut reader = ring.reader();
        let mut out = vec![[0.0f32; 2]; 128];
        reader.pull(&ring, 128, 1.0, &mut out);
        // The reader starts one latency target behind the writer, so it has a
        // full block of real audio to hand and nothing invented.
        assert!(out.iter().all(|f| f[1] == 0.5));
        assert!(out.iter().any(|f| f[0] > 0.0));
    }

    /// A console that cannot keep up gives itself more room, and stops asking
    /// for more than the ceiling.
    #[test]
    fn a_starved_reader_raises_the_target_up_to_the_ceiling() {
        reset_latency();
        assert_eq!(latency_ms(), LATENCY_MIN_MS);

        let ring = Ring::new(2);
        ring.push(&vec![[0.4f32, 0.4]; 128]);
        let mut reader = Reader::idle();
        let mut big = vec![[0.0f32; 2]; 8_192];
        let mut block = vec![[0.0f32; 2]; 1_024];

        // The first pull is the startup case: the ring is filling, so a short
        // read must not be blamed on the machine.
        reader.pull(&ring, 1_024, 1.0, &mut block);
        assert_eq!(latency_ms(), LATENCY_MIN_MS, "startup is not starvation");

        // A pull that is served in full means the reader is running.
        ring.push(&vec![[0.4f32, 0.4]; 8_192]);
        reader.pull(&ring, 1_024, 1.0, &mut block);
        assert!(block.iter().all(|f| f[0] == 0.4), "real audio, not silence");

        // Now ask for far more than is waiting: that is a buffer too small.
        reader.pull(&ring, 8_192, 1.0, &mut big);
        assert!(
            latency_ms() > LATENCY_MIN_MS,
            "a dry pull should grow the target"
        );

        // Keep asking for more than the ring holds: every pull is a shortfall,
        // and the target walks up to its ceiling instead of clicking forever.
        for _ in 0..40 {
            reader.pull(&ring, 8_192, 1.0, &mut big);
        }
        assert_eq!(latency_ms(), LATENCY_MAX_MS, "the target has a ceiling");
        reset_latency();
    }

    /// The reason the limiter exists: a program boosted past full scale leaves
    /// the bus at the ceiling, not squared off at it.
    #[test]
    fn a_boosted_bus_cannot_exceed_the_ceiling() {
        let mut limiter = Limiter::new();
        let ceiling = from_db(LIMIT_CEILING_DB);
        // 1.5× full scale: a mastered program with the +4 dB mid on DESKTOP.
        let mut block: Vec<[f32; 2]> = (0..960)
            .map(|i| {
                let phase = i as f32 / RATE as f32 * 1_000.0 * std::f32::consts::TAU;
                let sample = phase.sin() * 1.5;
                [sample, sample]
            })
            .collect();

        limiter.process(&mut block);
        let loudest = peak(&block);
        assert!(
            loudest <= ceiling + 1e-6,
            "the bus left at {loudest}, ceiling is {ceiling}"
        );
        assert!(limiter.gain() < 1.0, "the limiter never engaged");
    }

    /// The other half of the deal: once the peak has gone, the gain comes back
    /// rather than leaving the bus ducked for the rest of the session.
    #[test]
    fn the_limiter_lets_its_gain_back_after_the_peak() {
        let mut limiter = Limiter::new();
        let mut loud = vec![[1.5f32; 2]; 480];
        limiter.process(&mut loud);
        assert!(limiter.gain() < 0.9, "a 3 dB overshoot should duck the bus");

        // An 80 ms release needs about 3 840 frames; a second is plenty.
        let mut quiet = vec![[0.0f32; 2]; RATE as usize];
        limiter.process(&mut quiet);
        assert!(
            limiter.gain() > 0.99,
            "the gain should be back, it is {}",
            limiter.gain()
        );
    }

    /// Below the ceiling the limiter is a delay and nothing else: a quiet mix
    /// must come out sample for sample as it went in.
    #[test]
    fn a_quiet_bus_passes_through_untouched() {
        let mut limiter = Limiter::new();
        let block: Vec<[f32; 2]> = (0..960)
            .map(|i| {
                let phase = i as f32 / RATE as f32 * 440.0 * std::f32::consts::TAU;
                let sample = phase.sin() * 0.5;
                [sample, sample * 0.5]
            })
            .collect();
        let mut passed = block.clone();
        limiter.process(&mut passed);

        // The output is the input, `LIMIT_LOOKAHEAD` frames late.
        let ahead = LIMIT_LOOKAHEAD;
        for (before, after) in block[..block.len() - ahead].iter().zip(passed[ahead..].iter()) {
            assert_eq!(before, after, "a quiet bus must not be touched");
        }
        assert_eq!(limiter.gain(), 1.0, "nothing to pull down");
    }

    /// The low cut is a cut, not a tone control: it takes the rumble out of a
    /// close microphone and leaves speech where it is.
    #[test]
    fn the_low_cut_removes_rumble_and_keeps_the_voice() {
        let strip = StripParams::default();
        strip.set_eq_on(true);
        strip.set_cut(90.0);

        let rumble = measure(&strip, 40.0);
        let voice = measure(&strip, 1_000.0);
        let flat_voice = measure(&StripParams::default(), 1_000.0);
        assert!(
            rumble < 0.08,
            "40 Hz should be well below the corner, got {rumble}"
        );
        assert!(
            (voice - flat_voice).abs() / flat_voice < 0.05,
            "1 kHz must pass untouched: {voice} vs {flat_voice}"
        );
    }

    /// The body band is where a close microphone sounds boxy.
    #[test]
    fn the_body_band_cuts_the_boxiness_and_leaves_the_rest() {
        let strip = StripParams::default();
        strip.set_eq_on(true);
        strip.set_eq(3, -6.0);

        let body = measure(&strip, 350.0);
        let flat_body = measure(&StripParams::default(), 350.0);
        assert!(
            body < flat_body * 0.6,
            "6 dB down at 350 Hz should be plain: {body} vs {flat_body}"
        );

        let treble = measure(&strip, 10_000.0);
        let flat_treble = measure(&StripParams::default(), 10_000.0);
        assert!(
            (treble - flat_treble).abs() / flat_treble < 0.05,
            "air is not the body band's business: {treble} vs {flat_treble}"
        );
    }

    #[test]
    fn an_empty_ring_reads_as_silence_and_does_not_run_ahead() {
        let ring = Ring::new(1);
        let mut reader = ring.reader();
        let mut out = vec![[9.0f32; 2]; 64];
        reader.pull(&ring, 64, 1.0, &mut out);
        assert!(out.iter().all(|f| *f == [0.0, 0.0]));
        assert_eq!(reader.position, 0.0);
    }

    #[test]
    fn a_resampled_reader_consumes_more_input_than_it_outputs() {
        // Long enough that the reader starts well clear of the live edge even
        // at the largest latency target.
        let ring = Ring::new(4);
        let ramp: Vec<[f32; 2]> = (0..RATE as usize * 3)
            .map(|i| [i as f32, 0.0])
            .collect();
        ring.push(&ramp);

        let mut reader = ring.reader();
        // A 96 kHz device pulls a 48 kHz ring at two frames per frame. 256 is a
        // realistic period there — 512 would ask for more runway than the
        // smallest latency target holds.
        let mut out = vec![[0.0f32; 2]; 256];
        reader.pull(&ring, 256, 2.0, &mut out);
        let span = out.last().unwrap()[0] - out[0][0];
        assert!(
            (span - 510.0).abs() < 8.0,
            "256 frames at step 2 should span ~512 ring frames, got {span}"
        );
    }

    #[test]
    fn peaks_decay_after_the_signal_stops() {
        let mixer = Mixer::default();
        mixer.set_strip_peak(0, 0.8);
        assert!((mixer.strip_peak(0) - 0.8).abs() < 1e-6);
        mixer.set_strip_peak(0, 0.0);
        assert!(mixer.strip_peak(0) < 0.7);
        mixer.set_strip_peak(0, 0.0);
        assert!(mixer.strip_peak(0) < 0.6);
    }

    /// A tone through a strip's equaliser, measured after it settles.
    fn measure(strip: &StripParams, frequency: f32) -> f32 {
        let mut eq = StripEq::default();
        let frames: Vec<[f32; 2]> = (0..RATE as usize / 2)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let s = (2.0 * std::f32::consts::PI * frequency * t).sin() * 0.25;
                [s, s]
            })
            .collect();
        let mut buf = frames.clone();
        eq.process(strip, &mut buf);
        peak(&buf[RATE as usize / 4..])
    }

    #[test]
    fn the_high_shelf_lifts_treble_and_leaves_bass_alone() {
        let flat = StripParams::default();
        flat.set_eq_on(true);

        let boosted = StripParams::default();
        boosted.set_eq_on(true);
        boosted.set_eq(0, 12.0); // treble

        let treble = measure(&boosted, 10_000.0);
        let treble_flat = measure(&flat, 10_000.0);
        assert!(
            treble > treble_flat * 1.6,
            "12 dB of treble boost should be audible: {treble} vs {treble_flat}"
        );

        let bass = measure(&boosted, 80.0);
        let bass_flat = measure(&flat, 80.0);
        assert!(
            (bass - bass_flat).abs() / bass_flat < 0.12,
            "the treble knob must not move 80 Hz: {bass} vs {bass_flat}"
        );
    }

    /// Why the trim exists: a microphone that arrives below the gate's threshold
    /// is lifted into the window rather than the threshold being moved down to
    /// meet it.
    #[test]
    fn the_trim_brings_a_quiet_microphone_into_the_gate() {
        let strip = StripParams::default();
        strip.set_gate_on(true);
        strip.set_gate_named("Threshold", -40.0);
        strip.set_gate_named("Damping", -60.0);

        // A voice arriving 12 dB under where the gate opens.
        let audio = tone(300.0, -52.0, 400);

        let shut = to_db(peak(&through(&strip, &audio)));
        assert!(shut < -95.0, "the gate should stay shut, got {shut} dB");

        strip.set_trim(20.0);
        let open = to_db(peak(&through(&strip, &audio)));
        assert!(
            (open + 32.0).abs() < 1.0,
            "20 dB of trim should carry it 20 dB up, got {open} dB"
        );
    }

    /// The trim is part of the chain, not beside it: the compressor's detector
    /// hears it, so lifting the input deepens the reduction.
    #[test]
    fn the_trim_runs_ahead_of_the_compressor() {
        let strip = StripParams::default();
        strip.set_comp_on(true);
        strip.set_comp_named("Threshold", -30.0);
        strip.set_comp_named("Ratio", 4.0);
        strip.set_comp_named("Attack", 2.0);
        strip.set_comp_named("Release", 60.0);
        strip.set_comp_named("Knee", 0.0);

        // 6 dB over the threshold at 4:1 is 4.5 dB taken off. The tolerance is
        // wide because a peak detector settles a little under the crest of a
        // steady sine, which is a property of the detector and not of the trim.
        let plain = level(&strip, -24.0);
        assert!((plain + 28.5).abs() < 1.5, "expected about -28.5, got {plain}");

        // The same tone 6 dB up is 12 dB over the threshold, so 9 dB comes off
        // and under 2.5 dB of the 6 dB arrives at the output. A trim placed
        // after the compressor would have delivered all 6.
        strip.set_trim(6.0);
        let lifted = level(&strip, -24.0);
        assert!(
            lifted - plain > 0.5 && lifted - plain < 2.5,
            "6 dB of trim should mostly be compressed away, got {lifted} against {plain}"
        );
    }

    #[test]
    fn the_trim_is_clamped_at_both_ends() {
        let strip = StripParams::default();
        assert_eq!(strip.trim(), 0.0, "a strip starts untrimmed");
        strip.set_trim(100.0);
        assert_eq!(strip.trim(), TRIM_MAX);
        strip.set_trim(-100.0);
        assert_eq!(strip.trim(), TRIM_MIN);
    }

    /// A tone, `db` below full scale, lasting `ms` milliseconds.
    fn tone(frequency: f32, db: f32, ms: u32) -> Vec<[f32; 2]> {
        let amplitude = from_db(db);
        let frames = (RATE as f32 * ms as f32 / 1000.0) as usize;
        (0..frames)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let s = (2.0 * std::f32::consts::PI * frequency * t).sin() * amplitude;
                [s, s]
            })
            .collect()
    }

    /// Run audio through a strip's whole channel strip.
    fn through(strip: &StripParams, audio: &[[f32; 2]]) -> Vec<[f32; 2]> {
        let mut buffer = audio.to_vec();
        let mut chain = StripChain::default();
        chain.process(strip, &mut buffer);
        buffer
    }

    /// What a steady tone comes back at, in dB, once the strip has settled.
    fn level(strip: &StripParams, input_db: f32) -> f32 {
        let out = through(strip, &tone(700.0, input_db, 600));
        to_db(peak(&out[out.len() / 2..]))
    }

    /// Nothing switched on is nothing done: an unassigned strip is not coloured
    /// by the parts of the chain that are sitting there, and an untrimmed input
    /// is a multiply by exactly one.
    #[test]
    fn a_strip_with_its_dynamics_off_passes_audio_untouched() {
        let strip = StripParams::default();
        let audio = tone(440.0, -12.0, 100);
        assert_eq!(
            through(&strip, &audio),
            audio,
            "a strip with no gate, no compressor and no EQ must not touch the signal"
        );
    }

    #[test]
    fn the_gate_shuts_the_room_and_opens_for_a_voice() {
        let strip = StripParams::default();
        strip.set_gate_on(true);
        strip.set_gate_named("Threshold", -40.0);
        strip.set_gate_named("Attack", 2.0);
        strip.set_gate_named("Release", 40.0);
        strip.set_gate_named("Hold", 80.0);
        strip.set_gate_named("Damping", -60.0);

        let room = tone(300.0, -70.0, 500);
        let voice = tone(300.0, -12.0, 500);
        let mut audio = room.clone();
        audio.extend(voice.clone());
        audio.extend(room.clone());
        let n = room.len();

        let out = through(&strip, &audio);

        let shut = to_db(peak(&out[..n / 2]));
        assert!(shut < -95.0, "a shut gate should bury the room, got {shut} dB");

        let open = to_db(peak(&out[n + n / 2..]));
        assert!(
            (open + 12.0).abs() < 1.0,
            "an open gate should pass the voice whole, got {open} dB"
        );

        let again = to_db(peak(&out[out.len() - n / 4..]));
        assert!(again < -95.0, "and it should shut again, got {again} dB");
    }

    /// The hold is what makes a gate usable on speech: a pause between words must
    /// not shut it, and a real silence must.
    #[test]
    fn the_gate_holds_through_the_gaps_between_words() {
        let strip = StripParams::default();
        strip.set_gate_on(true);
        strip.set_gate_named("Threshold", -40.0);
        strip.set_gate_named("Attack", 1.0);
        strip.set_gate_named("Release", 10.0);
        strip.set_gate_named("Hold", 300.0);
        strip.set_gate_named("Damping", -60.0);

        let word = tone(300.0, -12.0, 200);
        let breath = tone(300.0, -60.0, 900);
        let mut audio = word.clone();
        audio.extend(breath.clone());
        let n = word.len();

        let out = through(&strip, &audio);
        let passed = |from: usize, to: usize| peak(&out[n + from..n + to]) / peak(&breath[from..to]);

        // The word is 200 ms, then the hold runs for 300 ms behind it, so the
        // gate may not start to close until half a second in.
        let during_the_hold = passed(2_400, 9_600);
        assert!(
            during_the_hold > 0.9,
            "the hold should keep it open through a pause, ratio {during_the_hold}"
        );
        let past_the_hold = passed(28_800, 38_400);
        assert!(
            past_the_hold < 0.05,
            "past the hold and the release it should be shut, ratio {past_the_hold}"
        );
    }

    #[test]
    fn the_compressor_pulls_the_loud_back_and_passes_the_quiet() {
        let strip = StripParams::default();
        strip.set_comp_on(true);
        strip.set_comp_named("Threshold", -20.0);
        strip.set_comp_named("Ratio", 4.0);
        strip.set_comp_named("Attack", 2.0);
        strip.set_comp_named("Release", 60.0);
        strip.set_comp_named("Knee", 0.0);

        // 12 dB over the threshold at 4:1 is 12 * (1 - 1/4) = 9 dB taken off.
        let loud = level(&strip, -8.0);
        assert!((loud + 17.0).abs() < 1.0, "expected about -17 dB, got {loud}");

        // Well below the threshold it is not a compressor any more.
        let quiet = level(&strip, -40.0);
        assert!((quiet + 40.0).abs() < 0.5, "the quiet should pass, got {quiet}");

        // And the makeup gain hands back what the ratio took.
        strip.set_comp_named("GainOut", 6.0);
        let lifted = level(&strip, -8.0);
        assert!(
            (lifted - (loud + 6.0)).abs() < 0.5,
            "6 dB of makeup should be 6 dB, got {lifted} against {loud}"
        );
    }

    /// The knee is the difference between a voice that is ridden and one that
    /// sounds switched: with width, the compression eases in around the corner.
    #[test]
    fn the_knee_bends_the_corner_instead_of_switching_it() {
        let strip = StripParams::default();
        strip.set_comp_on(true);
        strip.set_comp_named("Threshold", -20.0);
        strip.set_comp_named("Ratio", 4.0);
        strip.set_comp_named("Attack", 2.0);
        strip.set_comp_named("Release", 60.0);
        strip.set_comp_named("Knee", 0.0);

        let hard = level(&strip, -20.0);
        assert!(
            (hard + 20.0).abs() < 0.5,
            "a tone at the threshold with no knee should pass, got {hard}"
        );

        strip.set_comp_named("Knee", 24.0);
        let soft = level(&strip, -20.0);
        assert!(
            soft < hard - 1.5,
            "a wide knee should already be working at the threshold: {soft} against {hard}"
        );
    }

    /// Why the cut runs first: rumble a person cannot hear would otherwise be
    /// loud enough to hold the gate open, and the room would come back with it.
    #[test]
    fn the_low_cut_keeps_rumble_out_of_the_gate() {
        let cut = StripParams::default();
        cut.set_gate_on(true);
        cut.set_gate_named("Threshold", -36.0);
        cut.set_gate_named("Attack", 1.0);
        cut.set_gate_named("Release", 10.0);
        cut.set_gate_named("Hold", 300.0);
        cut.set_gate_named("Damping", -60.0);
        cut.set_eq_on(true);
        cut.set_cut(150.0);

        // The same strip without the cut, as the control.
        let uncut = StripParams::default();
        uncut.set_gate_on(true);
        uncut.set_gate_named("Threshold", -36.0);
        uncut.set_gate_named("Attack", 1.0);
        uncut.set_gate_named("Release", 10.0);
        uncut.set_gate_named("Hold", 300.0);
        uncut.set_gate_named("Damping", -60.0);

        // 25 Hz is far enough under the 150 Hz corner to be cut by 31 dB, which
        // leaves it 7 dB below the gate's threshold — quiet to the gate, and an
        // unusable amount of rumble in the room.
        let thump = tone(25.0, -12.0, 600);
        let room = tone(1_000.0, -70.0, 600);
        let mut audio = thump.clone();
        audio.extend(room.clone());
        let n = thump.len();

        // Just after the thump: whatever the hold left open is audible here.
        let window = n + n / 8..n + n / 4;
        let with_cut = peak(&through(&cut, &audio)[window.clone()]);
        let without = peak(&through(&uncut, &audio)[window.clone()]);

        assert!(
            without > 1e-4,
            "the rumble should be loud enough to hold the gate open: {without}"
        );
        assert!(
            with_cut < without * 0.05,
            "the cut should keep the rumble from opening the gate: {with_cut} against {without}"
        );
    }
}
