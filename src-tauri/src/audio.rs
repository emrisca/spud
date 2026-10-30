//! The audio path: devices in, devices out, and the streams between.
//!
//! Every stream is owned by one long-lived thread. That is not a detail — a
//! `cpal::Stream` stops the moment it is dropped, so somebody has to hold them,
//! and keeping them off the UI thread means a device change never blocks a
//! button. The thread waits on a channel; a rebuild request replaces the whole
//! set of streams at once.
//!
//! Data flow, per block:
//!
//! ```text
//! capture device ─▶ downmix ─▶ resample to 48 k ─▶ Ring ─┬─▶ bus 0 stream ─▶ output device
//!                                                        ├─▶ bus 1 stream ─▶ output device
//!                                                        └─▶ … up to 8
//! ```
//!
//! Each bus reads the rings it needs and mixes for itself, so buses at different
//! rates stay independent and a missing output cannot starve the others.

use crate::mixer::{self, Limiter, Mixer, Reader, Ring, StripChain, BUSES, RATE, STRIPS};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Which device is wired to which channel. Empty means unassigned.
#[derive(Clone)]
pub struct Config {
    pub strip_device: [String; STRIPS],
    pub bus_device: [String; BUSES],
}

impl Default for Config {
    fn default() -> Self {
        Config {
            strip_device: Default::default(),
            bus_device: Default::default(),
        }
    }
}

#[derive(serde::Serialize, Clone, Default)]
pub struct Status {
    pub running: bool,
    pub rate: u32,
    pub buffer: u32,
    /// Devices with a live stream right now.
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// Assignments that could not be opened, with the reason.
    pub errors: Vec<String>,
    pub missing: Vec<String>,
}

#[derive(serde::Serialize, Clone)]
pub struct DeviceDesc {
    pub index: i32,
    pub kind: String,
    pub name: String,
    pub hardware: String,
}

enum Msg {
    Rebuild,
    Stop,
}

pub struct Engine {
    tx: Sender<Msg>,
    pub mix: Arc<Mixer>,
    status: Arc<Mutex<Status>>,
    config: Arc<Mutex<Config>>,
}

impl Engine {
    /// Start the audio thread. It builds the streams it can and reports what it
    /// could not; nothing here fails hard, because an unassigned mixer is still
    /// a usable window.
    pub fn start() -> Engine {
        let mix = Arc::new(Mixer::default());
        let status = Arc::new(Mutex::new(Status::default()));
        let config = Arc::new(Mutex::new(auto_config()));
        let (tx, rx) = mpsc::channel();

        let thread_mix = Arc::clone(&mix);
        let thread_status = Arc::clone(&status);
        let thread_config = Arc::clone(&config);
        std::thread::Builder::new()
            .name("spud-audio".into())
            .spawn(move || run(rx, thread_mix, thread_status, thread_config))
            .expect("the audio thread");

        Engine {
            tx,
            mix,
            status,
            config,
        }
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    pub fn config(&self) -> Config {
        self.config.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// The live configuration itself, so the settings writer can read it
    /// without taking a copy on every keystroke.
    pub fn config_handle(&self) -> Arc<Mutex<Config>> {
        Arc::clone(&self.config)
    }

    pub fn assign(&self, config: Config) {
        if let Ok(mut slot) = self.config.lock() {
            *slot = config;
        }
        let _ = self.tx.send(Msg::Rebuild);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(rx: Receiver<Msg>, mix: Arc<Mixer>, status: Arc<Mutex<Status>>, config: Arc<Mutex<Config>>) {
    let host = cpal::default_host();
    let rings: Vec<Arc<Ring>> = (0..STRIPS).map(|_| Arc::new(Ring::new(2))).collect();
    let mut live: Vec<Stream>;

    loop {
        let desired = config.lock().map(|c| c.clone()).unwrap_or_default();
        live = build(&host, &desired, &mix, &rings, &status);

        match rx.recv() {
            Ok(Msg::Rebuild) => continue,
            _ => break,
        }
    }
    drop(live);
    if let Ok(mut slot) = status.lock() {
        slot.running = false;
        slot.inputs.clear();
        slot.outputs.clear();
    }
}

/// Build every assigned stream, replacing the previous set.
fn build(
    host: &cpal::Host,
    config: &Config,
    mix: &Arc<Mixer>,
    rings: &[Arc<Ring>],
    status: &Arc<Mutex<Status>>,
) -> Vec<Stream> {
    // Enumerate once per rebuild. Asking the host for its device list inside
    // each open() costs tens of milliseconds on Windows and there are fourteen
    // of them, which is the difference between a device change feeling instant
    // and feeling like a hiccup.
    let inputs: Vec<cpal::Device> = host.input_devices().map(|list| list.collect()).unwrap_or_default();
    let outputs: Vec<cpal::Device> = host
        .output_devices()
        .map(|list| list.collect())
        .unwrap_or_default();

    let mut streams = Vec::new();
    let mut next = Status {
        running: false,
        rate: RATE,
        buffer: 512,
        inputs: Vec::new(),
        outputs: Vec::new(),
        errors: Vec::new(),
        missing: Vec::new(),
    };

    for strip in 0..STRIPS {
        let name = config.strip_device[strip].trim();
        if name.is_empty() {
            continue;
        }
        let state = Capture {
            strip,
            ring: Arc::clone(&rings[strip]),
            mix: Arc::clone(mix),
            shape: Shape::default(),
        };
        match open_input(&inputs, name, state) {
            Ok(stream) => {
                next.inputs.push(name.to_string());
                streams.push(stream);
            }
            Err(reason) => {
                next.missing.push(name.to_string());
                next.errors.push(format!("input {strip}: {reason}"));
            }
        }
    }

    for bus in 0..BUSES {
        let name = config.bus_device[bus].trim();
        if name.is_empty() {
            continue;
        }
        let state = BusStream {
            bus,
            mix: Arc::clone(mix),
            rings: rings.to_vec(),
            readers: std::array::from_fn(|_| Reader::idle()),
            chain: Default::default(),
            strip_level: [0.0; STRIPS],
            bus_level: 0.0,
            scratch: vec![Vec::new(); STRIPS],
            mixbuf: Vec::new(),
            limit: Limiter::new(),
            step: 1.0,
        };
        match open_output(&outputs, name, state) {
            Ok(stream) => {
                next.outputs.push(name.to_string());
                streams.push(stream);
            }
            Err(reason) => {
                next.missing.push(name.to_string());
                next.errors.push(format!("output {bus}: {reason}"));
            }
        }
    }

    next.running = !streams.is_empty();
    if let Ok(mut slot) = status.lock() {
        *slot = next;
    }
    streams
}

fn find_input(devices: &[cpal::Device], name: &str) -> Result<cpal::Device, String> {
    for device in devices {
        if device.name().map(|n| n == name).unwrap_or(false) {
            return Ok(device.clone());
        }
    }
    Err(format!("\"{name}\" is not a capture device right now"))
}

fn find_output(devices: &[cpal::Device], name: &str) -> Result<cpal::Device, String> {
    for device in devices {
        if device.name().map(|n| n == name).unwrap_or(false) {
            return Ok(device.clone());
        }
    }
    Err(format!("\"{name}\" is not an output device right now"))
}

/// WASAPI failures arrive as an HRESULT in a string. The two that a person can
/// actually act on get said in words.
fn readable(reason: String) -> String {
    if reason.contains("0x8889000A") {
        format!("{reason} — the device is held exclusively by another program")
    } else if reason.contains("0x88890001") || reason.contains("0x80070005") {
        format!("{reason} — Windows refused access to the device")
    } else {
        reason
    }
}

/// One period of buffer, in frames, when the device will say what it allows.
///
/// cpal opens a shared-mode stream at the device's default buffer, and on this
/// machine that is several periods deep — measured as 22 ms for a render stream
/// where the engine's own period is 10 ms. In a chain that already crosses a
/// virtual cable, every extra period is lip-sync the listener can see, so ask
/// for one period and keep the default only for a device that will not answer.
fn small_buffer(supported: &cpal::SupportedStreamConfig) -> cpal::BufferSize {
    match *supported.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => {
            let one_period = supported.sample_rate().0 / 100;
            cpal::BufferSize::Fixed(one_period.clamp(min, max))
        }
        cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
    }
}

fn open_input(devices: &[cpal::Device], name: &str, capture: Capture) -> Result<Stream, String> {
    let device = find_input(devices, name)?;
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let mut config: StreamConfig = supported.config();
    config.buffer_size = small_buffer(&supported);
    let capture = Capture {
        shape: Shape::new(config.sample_rate.0, config.channels as usize),
        ..capture
    };
    let stream = match format {
        SampleFormat::F32 => build_input::<f32>(&device, &config, capture),
        SampleFormat::I16 => build_input::<i16>(&device, &config, capture),
        SampleFormat::U16 => build_input::<u16>(&device, &config, capture),
        other => return Err(format!("sample format {other:?} is not supported here")),
    }
    .map_err(|e| readable(e.to_string()))?;
    stream.play().map_err(|e| readable(e.to_string()))?;
    Ok(stream)
}

fn open_output(devices: &[cpal::Device], name: &str, mut bus: BusStream) -> Result<Stream, String> {
    let device = find_output(devices, name)?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let mut config: StreamConfig = supported.config();
    config.buffer_size = small_buffer(&supported);
    let channels = config.channels as usize;
    let rate = config.sample_rate.0 as f64;
    // Bus streams read 48 kHz rings, so a device at another rate steps by the
    // ratio instead of resampling a separate copy of everything.
    bus.step = RATE as f64 / rate;
    let stream = match format {
        SampleFormat::F32 => build_output::<f32>(&device, &config, bus, channels),
        SampleFormat::I16 => build_output::<i16>(&device, &config, bus, channels),
        SampleFormat::U16 => build_output::<u16>(&device, &config, bus, channels),
        other => return Err(format!("sample format {other:?} is not supported here")),
    }
    .map_err(|e| readable(e.to_string()))?;
    stream.play().map_err(|e| readable(e.to_string()))?;
    Ok(stream)
}

fn build_input<T>(device: &cpal::Device, config: &StreamConfig, capture: Capture) -> Result<Stream, cpal::BuildStreamError>
where
    T: Sample + SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let mut capture = capture;
    let error_callback = |error| eprintln!("spud: capture stream error: {error}");
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| capture.fill(data),
        error_callback,
        None,
    )
}

fn build_output<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut bus: BusStream,
    channels: usize,
) -> Result<Stream, cpal::BuildStreamError>
where
    T: Sample + SizedSample + Send + 'static + FromSample<f32>,
{
    let error_callback = |error| eprintln!("spud: output stream error: {error}");
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let frames = data.len() / channels.max(1);
            bus.render(frames);
            for (index, chunk) in data.chunks_mut(channels.max(1)).enumerate() {
                let frame = bus.mixbuf.get(index).copied().unwrap_or([0.0, 0.0]);
                match chunk.len() {
                    1 => chunk[0] = T::from_sample((frame[0] + frame[1]) * 0.5),
                    _ => {
                        chunk[0] = T::from_sample(frame[0].clamp(-1.0, 1.0));
                        chunk[1] = T::from_sample(frame[1].clamp(-1.0, 1.0));
                        // A surround endpoint gets silence in the channels this
                        // console does not address, rather than a copy of the
                        // front pair.
                        for sample in chunk.iter_mut().skip(2) {
                            *sample = T::from_sample(0.0f32);
                        }
                    }
                }
            }
        },
        error_callback,
        None,
    )
}

/// How one capture callback turns interleaved device samples into 48 kHz stereo.
struct Shape {
    /// Device frames consumed per 48 kHz frame.
    step: f64,
    /// Position inside the block, carried between callbacks.
    cursor: f64,
    channels: usize,
    previous: [f32; 2],
    have_previous: bool,
    input: Vec<[f32; 2]>,
    output: Vec<[f32; 2]>,
}

impl Default for Shape {
    fn default() -> Self {
        Shape::new(RATE, 2)
    }
}

impl Shape {
    fn new(rate: u32, channels: usize) -> Shape {
        Shape {
            step: rate as f64 / RATE as f64,
            cursor: 0.0,
            channels: channels.max(1),
            previous: [0.0, 0.0],
            have_previous: false,
            input: Vec::with_capacity(2048),
            output: Vec::with_capacity(2048),
        }
    }
}

struct Capture {
    strip: usize,
    ring: Arc<Ring>,
    mix: Arc<Mixer>,
    shape: Shape,
}

impl Capture {
    /// Downmix to stereo, resample to the mixing rate, publish, and report the
    /// pre-fader peak so the strip meter reads the device, not the fader.
    fn fill<T>(&mut self, data: &[T])
    where
        T: Sample + Copy,
        f32: FromSample<T>,
    {
        let channels = self.shape.channels;
        self.shape.input.clear();
        self.mix.set_block(data.len() / channels.max(1));
        let mut loudest = 0.0f32;
        for frame in data.chunks_exact(channels) {
            let stereo = match channels {
                1 => {
                    let sample = f32::from_sample(frame[0]);
                    [sample, sample]
                }
                _ => [f32::from_sample(frame[0]), f32::from_sample(frame[1])],
            };
            loudest = loudest.max(stereo[0].abs()).max(stereo[1].abs());
            self.shape.input.push(stereo);
        }
        self.mix.set_strip_peak(self.strip, loudest);

        if self.shape.step == 1.0 {
            self.ring.push(&self.shape.input);
            return;
        }

        self.shape.output.clear();
        let input = &self.shape.input;
        let mut cursor = self.shape.cursor;
        let length = input.len() as f64;
        while cursor + 1.0 < length {
            let base = cursor.floor() as i64;
            let fraction = (cursor - base as f64) as f32;
            let a = if base < 1 {
                if self.shape.have_previous {
                    self.shape.previous
                } else {
                    input[0]
                }
            } else {
                input[(base - 1) as usize]
            };
            let b = input[base.max(0) as usize];
            self.shape.output.push([
                a[0] + (b[0] - a[0]) * fraction,
                a[1] + (b[1] - a[1]) * fraction,
            ]);
            cursor += self.shape.step;
        }
        self.shape.cursor = cursor - length;
        if let Some(last) = input.last().copied() {
            self.shape.previous = last;
            self.shape.have_previous = true;
        }
        self.ring.push(&self.shape.output);
    }
}

/// One output bus: reads the strips routed to it and mixes them for its device.
struct BusStream {
    bus: usize,
    mix: Arc<Mixer>,
    rings: Vec<Arc<Ring>>,
    readers: [Reader; STRIPS],
    chain: [StripChain; STRIPS],
    strip_level: [f32; STRIPS],
    bus_level: f32,
    scratch: Vec<Vec<[f32; 2]>>,
    mixbuf: Vec<[f32; 2]>,
    /// The ceiling, after the bus gain. One per output stream, so a bus nobody
    /// is driving never touches another one's gain.
    limit: Limiter,
    step: f64,
}

impl BusStream {
    fn render(&mut self, frames: usize) {
        self.mixbuf.clear();
        self.mixbuf.resize(frames, [0.0; 2]);
        if frames == 0 {
            return;
        }

        let soloing = self.mix.soloing();
        for index in 0..STRIPS {
            let strip = &self.mix.strips[index];
            let audible =
                strip.routed_to(self.bus) && !strip.mute() && (!soloing || strip.solo());
            if !audible {
                // No read means no advance; on the next audible block the
                // reader clamps to the live edge by itself.
                self.strip_level[index] = 0.0;
                continue;
            }

            let buffer = &mut self.scratch[index];
            buffer.clear();
            buffer.resize(frames, [0.0; 2]);
            self.readers[index].pull(&self.rings[index], frames, self.step, buffer);
            // Gate, low cut, bands and compressor, in that order.
            self.chain[index].process(strip, buffer);

            let target = mixer::to_linear(strip.gain());
            let from = self.strip_level[index];
            let mono = strip.mono();
            for (position, frame) in buffer.iter().enumerate() {
                let weight = (position + 1) as f32 / frames as f32;
                let gain = from + (target - from) * weight;
                let mut sample = *frame;
                if mono {
                    let sum = (sample[0] + sample[1]) * 0.5;
                    sample = [sum, sum];
                }
                self.mixbuf[position][0] += sample[0] * gain;
                self.mixbuf[position][1] += sample[1] * gain;
            }
            self.strip_level[index] = target;
        }

        let bus = &self.mix.buses[self.bus];
        let muted = bus.mute();
        let target = if muted { 0.0 } else { mixer::to_linear(bus.gain()) };
        let from = self.bus_level;
        let mono = bus.mono();
        let limit_on = bus.limit_on();
        let mut loudest = 0.0f32;
        for (position, frame) in self.mixbuf.iter_mut().enumerate() {
            let weight = (position + 1) as f32 / frames as f32;
            let gain = from + (target - from) * weight;
            let mut sample = *frame;
            if mono {
                let sum = (sample[0] + sample[1]) * 0.5;
                sample = [sum, sum];
            }
            loudest = loudest.max(sample[0].abs()).max(sample[1].abs());
            *frame = [sample[0] * gain, sample[1] * gain];
        }
        self.bus_level = target;

        // The ceiling goes last, on the mixed bus: a strip that was equalised up
        // or compressed with makeup gain stops here instead of arriving at a
        // cable squared off by the output clamp.
        let entering = mixer::peak(&self.mixbuf);
        if limit_on {
            self.limit.process(&mut self.mixbuf);
        } else {
            self.limit.reset();
        }

        // `loudest` is the mix going into the bus, which is what the meter reads.
        // `held` is that mix after the bus gain but before the limiter, so a
        // console comparing it with the published ceiling lights its lamp exactly
        // while the ceiling is holding the bus down — rather than at full scale,
        // which the limiter now stops the bus from ever reaching. Switched off,
        // nothing is holding the bus down however far it is driven, so the lamp
        // is given nothing to light on: it reports the ceiling at work, and a
        // dark lamp beside a meter past the ceiling reads as what it is — this
        // bus is unlimited.
        let held = if limit_on { entering } else { 0.0 };
        self.mix.set_bus_peak(self.bus, loudest, held);
    }
}

/// Which cable a device belongs to, ignoring the direction.
///
/// A VB-CABLE install ships two endpoints per cable: a render side and a capture
/// side (`CABLE Input` / `CABLE Output`, `CABLE-A Input` / `CABLE-A Output`, …).
/// Both sides answer the same key, which is how a virtual strip is paired with
/// the bus that feeds it — and how a strip feeding its own cable is spotted.
///
/// The 16-channel Hi-Fi cables keep a mark of their own so they never collide
/// with the plain `CABLE` pair.
///
/// The endpoint names are dot-separated by hand in the drivers, so the direction
/// is matched after removing the spaces: Windows reports `CABLE-A In 16ch`,
/// `CABLE In 16 Ch` and `CABLE-B Output` in the same install, and a name that is
/// not recognised here is offered as if it were a microphone.
pub fn cable_key(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let body = lower.split('(').next()?.trim();
    let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    if !compact.starts_with("cable") {
        return None;
    }
    let (stem, wide) = if let Some(stem) = compact.strip_suffix("in16ch") {
        (stem, true)
    } else if let Some(stem) = compact.strip_suffix("out16ch") {
        (stem, true)
    } else if let Some(stem) = compact.strip_suffix("input") {
        (stem, false)
    } else if let Some(stem) = compact.strip_suffix("output") {
        (stem, false)
    } else {
        return None;
    };
    if !stem.starts_with("cable") {
        return None;
    }
    Some(if wide {
        format!("{stem} 16ch")
    } else {
        stem.to_string()
    })
}

/// Is this a virtual cable endpoint at all?
pub fn is_cable(name: &str) -> bool {
    cable_key(name).is_some()
}

/// Are these two devices the two sides of one cable?
pub fn same_cable(a: &str, b: &str) -> bool {
    match (cable_key(a), cable_key(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Each installed cable's capture side and render side, keyed by cable.
/// `(key, capture, render)` in the order the capture sides were found.
fn cable_pairs(outputs: &[String], inputs: &[String]) -> Vec<(String, Option<String>, Option<String>)> {
    let mut pairs: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    for (name, capture) in inputs.iter().map(|name| (name, true)).chain(outputs.iter().map(|name| (name, false))) {
        let Some(key) = cable_key(name) else { continue };
        let slot = match pairs.iter().position(|(known, _, _)| *known == key) {
            Some(slot) => slot,
            None => {
                pairs.push((key, None, None));
                pairs.len() - 1
            }
        };
        let entry = &mut pairs[slot];
        if capture {
            if entry.1.is_none() {
                entry.1 = Some(name.clone());
            }
        } else if entry.2.is_none() {
            entry.2 = Some(name.clone());
        }
    }
    pairs
}

/// A sensible first wiring, so the console is useful before anything is picked:
/// the first five non-cable inputs on strips 1–5, the default output on A1, and
/// one cable per virtual pair — its capture side on strip 6/7/8 and its render
/// side on the matching B bus (B1/B2/B3).
///
/// Pairing is by cable identity rather than list order, so `CABLE-A` stays on one
/// strip and one bus even if Windows enumerates its endpoints in a different
/// order than it did last boot.
pub fn auto_config() -> Config {
    const VIRTUAL_FROM: usize = 5;
    let (outputs, inputs) = devices();

    let mut config = Config::default();
    let hardware: Vec<&DeviceDesc> = inputs.iter().filter(|d| !is_cable(&d.name)).collect();
    for (slot, device) in hardware.iter().take(VIRTUAL_FROM).enumerate() {
        config.strip_device[slot] = device.name.clone();
    }

    if let Some(first) = outputs.iter().find(|d| !is_cable(&d.name)) {
        config.bus_device[0] = first.name.clone();
    }

    let output_names: Vec<String> = outputs.iter().map(|d| d.name.clone()).collect();
    let input_names: Vec<String> = inputs.iter().map(|d| d.name.clone()).collect();
    let slots = STRIPS - VIRTUAL_FROM;
    let mut used: Vec<String> = Vec::new();
    for (slot, (_, capture, render)) in cable_pairs(&output_names, &input_names)
        .iter()
        .take(slots)
        .enumerate()
    {
        if let Some(capture) = capture {
            config.strip_device[VIRTUAL_FROM + slot] = capture.clone();
        }
        // One cable feeds one B bus. Two buses on the same render side is both
        // undecodable in the console and a conflict in WASAPI.
        if let Some(render) = render {
            if !used.contains(render) {
                used.push(render.clone());
                config.bus_device[VIRTUAL_FROM + slot] = render.clone();
            }
        }
    }

    config
}

/// Put each virtual input back on its own cable's render side.
///
/// Only the B buses are touched, and only when one is empty or shares a cable
/// with an earlier one — a duplicate renders silence into one of the two cables,
/// so an app sent there would never be heard. A deliberate pairing, two
/// different cables, is left exactly as it is.
pub fn repair_renders(config: &mut Config) {
    let (outputs, inputs) = devices();
    let output_names: Vec<String> = outputs.iter().map(|d| d.name.clone()).collect();
    let input_names: Vec<String> = inputs.iter().map(|d| d.name.clone()).collect();
    repair_with(config, &cable_pairs(&output_names, &input_names));
}

fn repair_with(config: &mut Config, pairs: &[(String, Option<String>, Option<String>)]) {
    let render_of = |name: &str| -> Option<String> {
        let key = cable_key(name)?;
        pairs
            .iter()
            .find(|(known, _, _)| *known == key)
            .and_then(|(_, _, render)| render.clone())
    };

    for bus in 5..BUSES {
        let duplicated = (5..bus).any(|other| {
            !config.bus_device[other].trim().is_empty()
                && same_cable(&config.bus_device[other], &config.bus_device[bus])
        });
        if !config.bus_device[bus].trim().is_empty() && !duplicated {
            continue;
        }
        // B1 sits beside strip 5, B2 beside strip 6: the strip says which cable
        // this bus is supposed to be pouring into.
        if let Some(render) = render_of(&config.strip_device[bus]) {
            config.bus_device[bus] = render;
        }
    }
}

/// Every device the machine will let us use, for the two pickers.
pub fn devices() -> (Vec<DeviceDesc>, Vec<DeviceDesc>) {
    let host = cpal::default_host();
    let mut outputs = Vec::new();
    let mut inputs = Vec::new();

    if let Some(default) = host.default_output_device().and_then(|d| d.name().ok()) {
        if !default.is_empty() {
            outputs.push(DeviceDesc {
                index: 0,
                kind: "wdm".into(),
                name: default.clone(),
                hardware: "default".into(),
            });
        }
    }
    if let Ok(list) = host.output_devices() {
        for device in list {
            let Ok(name) = device.name() else { continue };
            if outputs.iter().any(|d: &DeviceDesc| d.name == name) {
                continue;
            }
            outputs.push(DeviceDesc {
                index: outputs.len() as i32,
                kind: "wdm".into(),
                name,
                hardware: "WASAPI".into(),
            });
        }
    }

    if let Some(default) = host.default_input_device().and_then(|d| d.name().ok()) {
        if !default.is_empty() {
            inputs.push(DeviceDesc {
                index: 0,
                kind: "wdm".into(),
                name: default.clone(),
                hardware: "default".into(),
            });
        }
    }
    if let Ok(list) = host.input_devices() {
        for device in list {
            let Ok(name) = device.name() else { continue };
            if inputs.iter().any(|d: &DeviceDesc| d.name == name) {
                continue;
            }
            inputs.push(DeviceDesc {
                index: inputs.len() as i32,
                kind: "wdm".into(),
                name,
                hardware: "WASAPI".into(),
            });
        }
    }

    (outputs, inputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn cable_sides_pair_and_ordinary_devices_do_not() {
        let pairs = [
            ("CABLE Input (VB-Audio Virtual Cable)", "cable"),
            ("CABLE Output (VB-Audio Virtual Cable)", "cable"),
            ("CABLE-A Input (VB-Audio Virtual Cable A)", "cable-a"),
            ("CABLE-B Output (VB-Audio Virtual Cable B)", "cable-b"),
            ("CABLE In 16 Ch (VB-Audio Hi-Fi Cable)", "cable 16ch"),
            ("CABLE Out 16 Ch (VB-Audio Hi-Fi Cable)", "cable 16ch"),
            // The names Windows actually reports for a current install.
            ("CABLE-A In 16ch (VB-Audio Virtual Cable A)", "cable-a 16ch"),
            ("CABLE-B In 16ch (VB-Audio Virtual Cable B)", "cable-b 16ch"),
            ("CABLE In 16ch (VB-Audio Virtual Cable)", "cable 16ch"),
        ];
        for (name, key) in pairs {
            assert_eq!(cable_key(name).as_deref(), Some(key), "{name}");
        }
        assert!(cable_key("Headphones (2- HyperX Cloud III Wireless)").is_none());
        assert!(cable_key("Microphone (USB Audio)").is_none());

        assert!(same_cable(
            "CABLE-A Input (VB-Audio Virtual Cable A)",
            "CABLE-A Output (VB-Audio Virtual Cable A)"
        ));
        assert!(same_cable(
            "CABLE Input (VB-Audio Virtual Cable)",
            "CABLE Output (VB-Audio Virtual Cable)"
        ));
        assert!(!same_cable(
            "CABLE-A Input (VB-Audio Virtual Cable A)",
            "CABLE-B Output (VB-Audio Virtual Cable B)"
        ));
        // A hardware microphone is never "the same cable" as anything.
        assert!(!same_cable("Microphone (USB Audio)", "CABLE Output (VB-Audio Virtual Cable)"));
    }

    /// Three cables, enumerated in a different order on each side, still land on
    /// the same strip and the same bus as each other.
    #[test]
    fn cables_are_paired_by_identity_not_by_list_order() {
        let inputs = [
            "CABLE-B Output (VB-Audio Virtual Cable B)".to_string(),
            "CABLE Output (VB-Audio Virtual Cable)".to_string(),
            "CABLE-A Output (VB-Audio Virtual Cable A)".to_string(),
            "Microphone (USB Audio)".to_string(),
        ];
        let outputs = [
            "CABLE-A Input (VB-Audio Virtual Cable A)".to_string(),
            "CABLE Input (VB-Audio Virtual Cable)".to_string(),
            "CABLE-B Input (VB-Audio Virtual Cable B)".to_string(),
        ];
        let pairs = cable_pairs(&outputs, &inputs);
        assert_eq!(pairs.len(), 3);
        for (key, capture, render) in &pairs {
            assert!(capture.is_some() && render.is_some(), "{key} has both sides");
            assert!(same_cable(capture.as_ref().unwrap(), render.as_ref().unwrap()));
        }
        let wired = |key: &str| {
            pairs
                .iter()
                .find(|(known, _, _)| known == key)
                .map(|(_, capture, render)| (capture.clone().unwrap(), render.clone().unwrap()))
                .unwrap()
        };
        assert_eq!(wired("cable-a").0, "CABLE-A Output (VB-Audio Virtual Cable A)");
        assert_eq!(wired("cable-a").1, "CABLE-A Input (VB-Audio Virtual Cable A)");
        assert_eq!(wired("cable-b").0, "CABLE-B Output (VB-Audio Virtual Cable B)");
    }

    /// A duplicate render side is repaired onto the cable that strip captures;
    /// a deliberate pairing is left alone.
    #[test]
    fn a_bus_sharing_a_cable_is_put_back_on_its_own() {
        let pairs = vec![
            (
                "cable".to_string(),
                Some("CABLE Output (VB-Audio Virtual Cable)".to_string()),
                Some("CABLE Input (VB-Audio Virtual Cable)".to_string()),
            ),
            (
                "cable-a".to_string(),
                Some("CABLE-A Output (VB-Audio Virtual Cable A)".to_string()),
                Some("CABLE-A Input (VB-Audio Virtual Cable A)".to_string()),
            ),
            (
                "cable-b".to_string(),
                Some("CABLE-B Output (VB-Audio Virtual Cable B)".to_string()),
                Some("CABLE-B Input (VB-Audio Virtual Cable B)".to_string()),
            ),
        ];
        let mut config = Config::default();
        for (slot, key) in ["cable", "cable-a", "cable-b"].iter().enumerate() {
            let pair = pairs.iter().find(|(known, _, _)| known == key).unwrap();
            config.strip_device[5 + slot] = pair.1.clone().unwrap();
        }
        config.bus_device[5] = "CABLE Input (VB-Audio Virtual Cable)".into();
        config.bus_device[6] = "CABLE-A Input (VB-Audio Virtual Cable A)".into();
        // B3 points at B1's cable: whatever is sent there is never heard.
        config.bus_device[7] = "CABLE Input (VB-Audio Virtual Cable)".into();

        repair_with(&mut config, &pairs);

        assert_eq!(config.bus_device[5], "CABLE Input (VB-Audio Virtual Cable)");
        assert_eq!(config.bus_device[7], "CABLE-B Input (VB-Audio Virtual Cable B)");
    }

    #[test]
    fn the_device_lists_are_usable() {
        let (outputs, inputs) = devices();
        println!("{} outputs, {} inputs", outputs.len(), inputs.len());
        for device in &outputs {
            println!("  out: {} [{}]", device.name, device.hardware);
        }
        for device in &inputs {
            println!("  in:  {} [{}]", device.name, device.hardware);
        }
        assert!(
            !outputs.is_empty() || !inputs.is_empty(),
            "a machine running the tests has at least one audio device"
        );
        assert!(outputs.iter().all(|d| !d.name.is_empty()));
    }

    /// The whole path a buffer takes inside a bus: ring, reader, routing, fader,
    /// meter. No device is involved, so this runs anywhere.
    #[test]
    fn a_routed_strip_arrives_on_its_bus_and_obeys_mute_and_solo() {
        let mix = Arc::new(Mixer::default());
        let rings: Vec<Arc<Ring>> = (0..STRIPS).map(|_| Arc::new(Ring::new(1))).collect();
        // Half-scale DC on strip 0, enough of it to cover the reader's latency.
        rings[0].push(&vec![[0.5f32, 0.5]; 8_192]);

        mix.strips[0].set_routed(0, true);
        mix.strips[0].set_routed(1, true);
        mix.strips[1].set_routed(0, false);

        let mut bus = BusStream {
            bus: 0,
            mix: Arc::clone(&mix),
            rings: rings.clone(),
            readers: std::array::from_fn(|_| Reader::idle()),
            chain: Default::default(),
            strip_level: [0.0; STRIPS],
            bus_level: 0.0,
            scratch: vec![Vec::new(); STRIPS],
            mixbuf: Vec::new(),
            limit: Limiter::new(),
            step: 1.0,
        };

        // The ceiling holds five milliseconds of look-ahead, so a level is read
        // after the line has filled rather than on the block that filled it.
        let settle = |bus: &mut BusStream| {
            bus.render(480);
            bus.render(480);
            mixer::peak(&bus.mixbuf)
        };

        let level = settle(&mut bus);
        assert!(
            (level - 0.5).abs() < 0.02,
            "a routed strip should arrive at unity, got {level}"
        );
        assert!(mix.bus_peak(0) > 0.4, "the bus meter should move");

        // Muting the strip takes it out of the sum.
        mix.strips[0].set_mute(true);
        assert!(settle(&mut bus) < 1e-6, "a muted strip must be silent");
        mix.strips[0].set_mute(false);

        // Solo elsewhere silences it without touching its own controls.
        mix.strips[2].set_solo(true);
        assert!(settle(&mut bus) < 1e-6, "solo must isolate");
        mix.strips[2].set_solo(false);

        // The fader is in dB, and the bottom of the throw is silence.
        mix.strips[0].set_gain(-6.0);
        let level = settle(&mut bus);
        assert!(
            (level - 0.5 * 0.501).abs() < 0.02,
            "-6 dB should halve the level, got {level}"
        );
        mix.strips[0].set_gain(mixer::GAIN_MIN);
        assert!(settle(&mut bus) < 1e-6, "the fader bottom is silence");
    }

    /// What leaves a bus: a strip driven past full scale stops at the ceiling,
    /// and switching the ceiling off hands the full level back — the difference
    /// is the limiter, not the routing.
    #[test]
    fn a_boosted_strip_stops_at_the_bus_ceiling() {
        let mix = Arc::new(Mixer::default());
        let rings: Vec<Arc<Ring>> = (0..STRIPS).map(|_| Arc::new(Ring::new(1))).collect();
        // Half scale in, the fader at the top: 0.5 × 4 ≈ 2.0 out.
        rings[0].push(&vec![[0.5f32, 0.5]; 8_192]);
        mix.strips[0].set_routed(0, true);
        mix.strips[0].set_gain(mixer::GAIN_MAX);

        let mut bus = BusStream {
            bus: 0,
            mix: Arc::clone(&mix),
            rings: rings.clone(),
            readers: std::array::from_fn(|_| Reader::idle()),
            chain: Default::default(),
            strip_level: [0.0; STRIPS],
            bus_level: 0.0,
            scratch: vec![Vec::new(); STRIPS],
            mixbuf: Vec::new(),
            limit: Limiter::new(),
            step: 1.0,
        };

        // One block to fill the look-ahead, the next to read a settled bus.
        bus.render(480);
        bus.render(480);
        let ceiling = 10f32.powf(mixer::LIMIT_CEILING_DB / 20.0);
        let limited = mixer::peak(&bus.mixbuf);
        assert!(
            limited <= ceiling + 1e-6,
            "the bus left at {limited}, ceiling is {ceiling}"
        );
        assert!(mix.bus_post(0) > ceiling, "the lamp should show the limiter working");

        // With the ceiling off the same wiring is over full scale again.
        mix.buses[0].set_limit(false);
        bus.render(480);
        let free = mixer::peak(&bus.mixbuf);
        assert!(free > 1.5, "an unlimited bus should pass the boost, got {free}");
    }

    /// With the ceiling off the bus is its own sum, sample for sample.
    ///
    /// Deliberately loud: 1.2 is above the ceiling, so a bypass that leaked any
    /// limiting, any look-ahead delay, or any gain would show up as a difference
    /// rather than passing on a quiet signal that nothing would have touched.
    #[test]
    fn the_limiter_off_is_passthrough() {
        let mix = Arc::new(Mixer::default());
        let rings: Vec<Arc<Ring>> = (0..STRIPS).map(|_| Arc::new(Ring::new(1))).collect();
        rings[0].push(&vec![[1.2f32, 1.2]; 8_192]);
        mix.strips[0].set_routed(0, true);
        mix.strips[0].set_gain(0.0);
        mix.buses[0].set_limit(false);

        let mut bus = BusStream {
            bus: 0,
            mix: Arc::clone(&mix),
            rings: rings.clone(),
            readers: std::array::from_fn(|_| Reader::idle()),
            chain: Default::default(),
            strip_level: [0.0; STRIPS],
            bus_level: 0.0,
            scratch: vec![Vec::new(); STRIPS],
            mixbuf: Vec::new(),
            limit: Limiter::new(),
            step: 1.0,
        };

        assert!(
            1.2 > mixer::ceiling(),
            "the signal must be over the ceiling for this to prove anything"
        );
        // One block settles the fader ramp from silence to unity.
        bus.render(480);
        bus.render(480);
        for frame in &bus.mixbuf {
            assert_eq!(*frame, [1.2, 1.2], "a bypassed ceiling must not touch the mix");
        }
        assert_eq!(
            mix.bus_post(0),
            0.0,
            "nothing is holding the bus down, so the lamp has nothing to light on"
        );
    }

    /// The wiring this machine would actually get: every assignment that names a
    /// device on this machine opens a stream. Skips quietly on a machine with no
    /// audio hardware, so the suite stays portable.
    #[test]
    fn the_default_wiring_opens_its_streams() {
        let (outputs, inputs) = devices();
        if outputs.is_empty() && inputs.is_empty() {
            eprintln!("no audio devices here — skipping");
            return;
        }

        let config = auto_config();
        let expected_in = config.strip_device.iter().filter(|d| !d.is_empty()).count();
        let expected_out = config.bus_device.iter().filter(|d| !d.is_empty()).count();

        let engine = Engine::start();
        engine.assign(config);
        // Building the streams means opening fourteen WASAPI endpoints, which
        // is not a fixed amount of time on a busy machine. Wait for the engine
        // to report something rather than guessing at a sleep.
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        while std::time::Instant::now() < deadline {
            let status = engine.status();
            if status.running || !status.errors.is_empty() || !status.missing.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let status = engine.status();
        println!(
            "{} in ({} wanted), {} out ({} wanted): {:?}",
            status.inputs.len(),
            expected_in,
            status.outputs.len(),
            expected_out,
            status.errors
        );
        assert!(status.running, "at least one stream should be open");
        assert!(
            status.inputs.len() >= expected_in.min(1),
            "the default wiring should open the inputs it named"
        );

        // The default output can legitimately be held exclusively by another
        // program (a running Voicemeeter does exactly that to hardware), so the
        // check is that *an* output opens, and that the failure is explained.
        if status.outputs.is_empty() {
            assert!(
                status.errors.iter().any(|e| e.contains("held exclusively")),
                "a busy default output should be reported as such: {:?}",
                status.errors
            );
            let mut opened = false;
            for device in &outputs {
                let mut config = Config::default();
                config.bus_device[0] = device.name.clone();
                engine.assign(config);
                std::thread::sleep(Duration::from_millis(400));
                if !engine.status().outputs.is_empty() {
                    println!("{} opened instead", device.name);
                    opened = true;
                    break;
                }
            }
            assert!(opened, "no output device on this machine could be opened");
        }
    }

    /// The audio thread builds the streams it can and says what it could not;
    /// it must never take the window down with it.
    #[test]
    fn the_thread_survives_an_impossible_assignment() {
        let engine = Engine::start();
        let mut config = Config::default();
        config.bus_device[0] = "a speaker that does not exist".into();
        engine.assign(config);
        std::thread::sleep(Duration::from_millis(600));
        let status = engine.status();
        assert!(!status.running);
        assert_eq!(status.missing.len(), 1);
        assert!(status.errors.iter().any(|e| e.contains("not an output device")));
    }
}
