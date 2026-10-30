# Spud

An eight-in, eight-bus mixing console with **its own audio engine**, in a Tauri
window. Nothing else has to be installed for the mixing to work: Spud captures
each input through WASAPI, sums it into every bus it is routed to, and renders
each bus to the output device you chose.

The look follows the console layout it was modelled on — strips on the left,
buses on the right, meters beside every fader — but the audio path is Spud's own
and owes nothing to it at runtime.

## What it does

- **Five hardware inputs and three virtual inputs.** Any capture device can be
  a strip, including a microphone, a line input, or the capture side of a virtual
  cable.
- **Eight buses, each a real output.** A1–A5 are for speakers and headphones;
  B1–B3 render into a virtual cable's Input side, so another program records that
  bus from the cable's Output — a microphone, as far as it knows.
- **Per strip:** fader (−60 dB to +12 dB, with the bottom of the throw as true
  silence), mute, solo, mono, and a routing button per bus.
- **Per virtual strip:** a three-band equaliser (treble shelf at 6 kHz, mid peak
  at 1 kHz, bass shelf at 120 Hz), switched in with the EQ chip.
- **Per hardware strip:** the same three bands plus a **low cut** (60–150 Hz) and
  a **body** band at 350 Hz, in a panel behind the EQ chip. A microphone starts
  on a voice preset — 90 Hz cut, −3 dB of boxiness, a little presence and air —
  and **Voice** / **Flat** put it back or take it away.
- **Per input, an input level and then a gate and a compressor**, behind the DYN
  chip. The input level (−24 to +24 dB) is the first thing in the strip, so the
  gate, the equaliser and the compressor all work at the level you set — which is
  how a quiet microphone comes up into the chain instead of the gate's threshold
  being moved down to meet it. The fader cannot do this job: it sits after all of
  them, so it changes what a listener hears and not what the gate hears. The gate
  shuts a quiet room out and holds through the gaps between words; the compressor
  rides the level. All of it is real, and runs on the samples in the order a
  channel strip runs them — input level, low cut, gate, bands, compressor — and a
  microphone starts on the voice chain (gate at −42 dB with a 220 ms hold, 3:1
  over −18 dB, 6 dB knee, +3 dB of makeup). **Voice** / **Off** switch the
  dynamics in or out; the input level is yours, and no preset moves it.
- **Low latency.** The console holds 20 ms between the cables and the speakers,
  plus the 5 ms a bus's limiter needs to look ahead, and gives itself more only
  if a stream actually runs dry, so a video and its sound stay together.
- **Per bus:** fader, mute, mono, a device picker, and a **LIM** ceiling that
  stops a boosted strip from arriving at a device squared off (see
  [The ceiling on every bus](#the-ceiling-on-every-bus)).
- **Meters** for every strip and bus, read from the audio thread.
- **Per-application audio.** Each virtual input lists the programs Windows has
  rendering into the matching cable, with volume and mute, exactly as long as
  those sessions exist. Quit the program and the row goes with it.

Not in this build: the FX sends, panning, the denoiser, recording, and the
multichannel position switches. The console shows no controls for them rather
than offering buttons that would not move audio.

## The one thing to install

Virtual inputs need a **virtual audio cable**, because Windows has no user-mode
way to create an endpoint for other programs to render into. [VB-CABLE](https://vb-audio.com/Cable/)
is free and signed, and installs one render/capture pair.

Each cable is **one** virtual input, so install as many as you have programs to
separate:

| Installer | Cables | Virtual inputs |
| --- | --- | --- |
| VB-CABLE | `CABLE` | Virtual 1 |
| VB-CABLE **A+B** pack | `CABLE-A`, `CABLE-B` | Virtual 2, Virtual 3 |

The A+B pack is the one to get: together with the plain cable it fills all three
virtual inputs, which is the limit — the console has three of them by design.

1. Install the cable(s) and reboot if the installer asks.
2. In **Settings → Rescan devices**, or on the next launch, Spud wires each
   cable's capture side to a virtual strip and its render side to the matching B
   bus (`CABLE` → Virtual 1 / B1, `CABLE-A` → Virtual 2 / B2, and so on). The
   pairing is by cable name, not by list order, so it survives a reboot that
   enumerates the endpoints differently.
3. In Windows sound settings, set the program's output to that cable's Input
   side. It appears on the matching virtual strip, and the strip's fader
   controls it. Two programs on two cables get two independent strips.

Hardware inputs and the physical buses work with no cable installed at all.

### Spud does not install the cables for you

The installer copies Spud and nothing else. A virtual cable is a signed kernel
driver from VB-Audio, the A+B pack is licensed per machine, and redistributing
it needs a distribution licence, so no driver setup travels inside Spud's
build. Download the pack from <https://vb-audio.com/Cable/>, run its setup as
administrator, and restart Windows before opening Spud.

Both Windows installers behave the same way: the MSI and the NSIS
`…-setup.exe` install Spud alone. The NSIS build is still per-machine, because
Spud writes a sign-in entry and nothing else in it needs a driver.

## Using Spud as a microphone

This is the equivalent of sending a bus back to a mic. The engine renders the
bus into a cable; Windows then offers the other side of that cable as a
**recording** device:

That is also the only route there is, which is worth knowing before hunting for
your microphone in a bus's device list: it will never be in one. A microphone is
a **recording** device and a bus renders into a **playback** device, and Windows
refuses to let an application write into a recording endpoint (the audio engine
answers `AUDCLNT_E_WRONG_ENDPOINT_TYPE`). Voicemeeter manages it by installing
its own driver into the audio path; Spud talks to Windows through the standard
user-mode API and cannot. The cable is the supported way to get audio back to a
microphone, and the two ends are already wired for you:

1. On any input — your real microphone, a virtual input, or both — switch on the
   routing button for `B1` (or `B2`/`B3`). That bus is now carrying that input.
2. In the program that needs a microphone (Discord, OBS, a game), choose
   `CABLE Output` as its input device. For `CABLE-A`/`B` pick `CABLE-A Output`
   and `CABLE-B Output`.
3. To cover every program at once, set that Output as Windows' **default
   recording device** in Settings → System → Sound.

One rule is enforced rather than merely documented: a virtual input that
captures a cable cannot be routed back into that same cable. The routing button
is disabled and the engine refuses the assignment, because the signal would
return to its own input and howl.

## The ceiling on every bus

Every bus ends in a **look-ahead peak limiter**, and it is on from the first
launch. It is not a mode you switch on to fix a mix: it only acts above
−0.3 dBFS, so a mix that never gets there sounds exactly as it would with the
limiter absent.

What it is for: a bus is what another program records, and a signal that leaves
on a cable above full scale is clipped by every program that records it. A
strip is easy to push over that line without noticing — +4 dB of mid on the EQ,
+3 dB of compressor makeup, a fader past unity — and the sum of several strips
adds up even when no single one is loud.

- **Where it sits:** the very last thing on the bus, after the fader and the
  mono fold, immediately before the device. Every strip's routing, EQ and
  dynamics have already been summed by then, and one ceiling covers all of them.
- **The ceiling is −0.3 dBFS** (0.966), not full scale, which leaves the last
  third of a decibel as headroom for the 24-bit cable and whatever records it.
- **It looks ahead 5 ms.** The audio is delayed by that much while the gain is
  worked out, so the gain is already down when a transient lands. The waveform
  is turned down instead of shaved off, which is the difference between a
  limiter and a clipper — no squared-off edges, no harshness added to the bus.
  Those five milliseconds are the only cost; they are why a bus with the ceiling
  on arrives 5 ms later than the number in the latency bullet above.
- **Release is 80 ms**, so a loud word ducks the bus briefly rather than for the
  rest of the sentence.
- **Both channels share one gain**, so a peak on one side never pulls the stereo
  image sideways.

### The LIM chip

The `LIM` chip sits in each bus column beside `M` and `Ø`, and it is lit in moss
when the ceiling is on — the quiet colour, because protection is the normal
state and not something shouting for attention. Press it and that bus is
**unlimited**: its mix is passed through sample for sample, including anything
above full scale. Hover either state for a one-line reminder of which it is.

Leave it on. The only reason to switch it off is to measure what a boost is
really doing, because the difference is otherwise hidden — which is exactly what
the lamp is for.

### What the lamp means

The **red lamp** beside a bus meter is not a clip lamp. Since the ceiling is
holding the bus, nothing the bus sends on can reach full scale, so a lamp that
lit at full scale would never light at all and the limiter's work would be
invisible.

Instead the lamp lights while the mix **entering** the ceiling is above it —
that is, while the limiter is actively holding the bus down. It stays lit for a
second and a half after the last such moment, so a single transient is visible
rather than a one-frame flicker. The meter still shows the level going *into*
the bus, so you can see how far over you are driving it while the bus itself
leaves clean.

Read together: **meter over the ceiling with the lamp lit** means the limiter is
working — the bus is louder than it can leave, and you are trading dynamics for
level. **Meter over the ceiling with the lamp dark** means the bus is unlimited
and that level is going to the device as it is.

### With the ceiling off

Switching `LIM` off on a bus that is being driven hard is audible, and not in a
subtle way. Measured on this machine with a 1 kHz tone into a strip raised +6 dB
above unity, the bus carried **3.1** into the ceiling (about +10 dBFS). What
came out the far side of the cable:

| `LIM` | What left the bus | Distortion on the cable |
| --- | --- | --- |
| on | 0.966 — the ceiling, a clean sine | −101 dB (none measurable) |
| off | passed through above full scale, squared off by the driver | −10 dB (31 % THD) |

That is the crunch a clipped virtual cable is known for, and one lit `LIM` chip
on the bus is all that prevents it. The setting is remembered per bus with the
rest of the console.

## Closing the window

The close button hides Spud to the notification area instead of quitting. The
mix is this process, so it keeps running while the window is away — closing the
window never interrupts audio. Left-click the tray icon to bring the console
back; right-click it for **Quit**, which stops the streams first.

**Launch at sign-in** lives in Settings. It writes one value under
`HKEY_CURRENT_USER\...\Run`, pointing at this executable with `--tray`, so
sign-in starts Spud hidden in the notification area and the mix is live before
you open the window. Turning the switch off deletes that value; nothing is
written anywhere else, so no administrator rights are involved.

### If a bus will not open

Spud reports `the device is held exclusively by another program` when Windows
refuses a device. Some mixers — Voicemeeter is one — take their hardware outputs
in exclusive mode, so while they run nothing else can open them. Close the other
program, or assign that bus a different device.

## What Spud remembers

Closing the app and opening it again brings the console back exactly as you left
it — every strip and bus keeps its fader position, input level, mute, solo,
mono, routing, EQ and dynamics settings, label and device choice, including a
slot you deliberately set to **None**.

| What | Where |
| --- | --- |
| The console (faders, input levels, routing, EQ, dynamics, labels, devices, each bus's ceiling) | `%APPDATA%\dev.spud.console\settings.json` |
| Theme mode, preset, hue and chroma | the window's own storage, written as you change it |
| Launch at sign-in | one value under `HKEY_CURRENT_USER\…\Run` |

The file is printed as plain JSON, so it can be read, copied between machines,
or deleted to start from a fresh console (close Spud first). It is written about
half a second after the last change — dragging a fader is one write, not fifty —
and immediately when you quit from the tray. A route that would loop is dropped
when the file is loaded, in case a device moved since it was saved.

A microphone's EQ, gate and compressor are only restored once you have touched
them. Until then the voice chain is re-applied at every launch, so an upgrade can
improve the defaults without overwriting a setting you chose.

## Build and run

```sh
npm install
npm run tauri dev      # development window, hot reload
npm run tauri build    # Windows installer in src-tauri/target/release/bundle
```

Requirements: Node 18+, a Rust toolchain with the MSVC target, and the WebView2
runtime (present on Windows 10/11). The Rust half uses `cpal` for the audio
devices and `windows` for the WASAPI session enumeration.

## How the audio is put together

```
capture device ─▶ downmix ─▶ resample to 48 kHz ─▶ ring ─┬─▶ bus stream ─▶ output device
                                                         ├─▶ bus stream ─▶ output device
                                                         └─▶ …
```

- `src-tauri/src/mixer.rs` — parameters, meters, the sample ring, the channel
  strip (the low cut, the gate, the four bands and the compressor) and the bus
  limiter. All shared state is atomic, so no callback ever takes a lock.
- `src-tauri/src/audio.rs` — one thread owns every `cpal::Stream`. A device
  change rebuilds the whole set; the window never blocks on it.
- `src-tauri/src/sessions.rs` — which programs are playing into each cable, read
  from Windows audio sessions.
- `src-tauri/src/state.rs` — the console's vocabulary (`Strip[2].Gain`,
  `Bus[5].Mute`, `A1`…`B3`) bound to the mixer, and the snapshot the UI draws.

Each bus mixes for itself from the input rings, so an output at 44.1 kHz and one
at 48 kHz can run side by side, and a device that fails to open cannot starve the
others.

## Tests

```sh
cd src-tauri && cargo test
```

The suite covers the mixing rules (routing, mute, solo, fader taper, the
equaliser's response, the low cut keeping rumble out of the gate), the gate's
hold and release, the compressor's ratio, knee and makeup gain, the input level
carrying a quiet microphone over the gate's threshold and running ahead of the
compressor, the bus ceiling keeping a boosted strip at −0.3 dBFS with the lamp
reporting it (and the bypass handing the boost straight back), the ring and its
fractional-rate reader, the meter decay, what the console remembers, the snapshot
it draws, and — on a machine with audio devices — device enumeration and the
audio thread's refusal to die on a bad assignment.
