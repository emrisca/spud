# Spud

An eight input, eight bus mixing console for Windows, in a Tauri window, with
its own audio engine. Spud captures each input through WASAPI, sums it into
every bus it is routed to, and renders each bus to the output device you chose.
No other mixer has to be installed for the mixing to work.

The look follows the console layout it was modelled on: strips on the left,
buses on the right, meters beside every fader. The audio path is Spud's own and
owes nothing to it at runtime.

## What it does

- **Five hardware inputs and three virtual inputs.** Any capture device can be a
  strip: a microphone, a line input, or the capture side of a virtual cable.
- **Eight buses, each a real output.** A1 to A5 are speakers and headphones. B1
  to B3 render into a virtual cable's Input side, so another program records
  that bus from the cable's Output.
- **Per strip:** fader (−60 dB to +12 dB, with the bottom of the throw as true
  silence), mute, solo, mono, and a routing button per bus.
- **Per input:** an input level, a denoiser, a gate and a compressor behind the
  `DYN` chip. All of it runs on the samples, in the order a channel strip runs
  them: input level, denoiser, low cut, gate, equaliser, compressor.
- **Per input, an equaliser.** A hardware strip opens the full panel from the
  `EQ` chip: bass at 120 Hz, body at 350 Hz, mid at 1 kHz, treble at 6 kHz, and a
  low cut at 60, 90, 120 or 150 Hz. A virtual strip shows its three bands as
  knobs on the column.
- **The denoiser** learns the quietest thing an input hears and pulls the signal
  down towards that floor, so hiss, a fan or a noisy preamp comes down while a
  voice, which sits well above the floor, passes. It is broadband rather than
  spectral, which keeps it cheap enough to run on every strip of every bus, and
  the reduction is smooth rather than switched, so the room does not pump between
  words. It is off until you switch it on, and the amount is yours.
- **Low latency.** The console holds 20 ms between the cables and the speakers,
  plus the 5 ms a bus's limiter needs to look ahead. It gives itself another
  5 ms, up to 60 ms, only if a stream actually runs dry.
- **Per bus:** fader, mute, mono, a device picker, a name you set by clicking
  the word under the bus key, and a `LIM` ceiling that stops a boosted mix from
  reaching a device squared off.
- **Meters** for every strip and bus, read from the audio thread.
- **Per-application audio.** Each virtual input lists the programs Windows is
  rendering into the matching cable, with volume and mute. Quit the program and
  the row goes with it.

## The virtual cable

Virtual inputs need a **virtual audio cable**, because Windows has no user-mode
way to create an endpoint for other programs to render into. [VB-CABLE](https://vb-audio.com/Cable/)
is free and signed, and installs one render and capture pair.

Each cable is one virtual input, so install as many as you have programs to
separate:

| Installer | Cables | Virtual inputs |
| --- | --- | --- |
| VB-CABLE | `CABLE` | Virtual 1 |
| VB-CABLE **A+B** pack | `CABLE-A`, `CABLE-B` | Virtual 2, Virtual 3 |

The A+B pack fills the remaining two. Together with the plain cable it covers all
three virtual inputs, which is the limit: the console has three by design.

1. Install the cables and restart Windows if the installer asks.
2. Open Spud. Each cable's capture side is wired to a virtual strip and its
   render side to the matching B bus (`CABLE` to Virtual 1 and B1, `CABLE-A` to
   Virtual 2 and B2, and so on). The pairing is by cable name, not by list order,
   so it survives a restart that enumerates the endpoints differently.
3. In Windows sound settings, set the program's output to that cable's Input
   side. The program appears on the matching virtual strip and its fader
   controls it.

Hardware inputs and the physical buses work with no cable installed at all.

### Spud does not install the cables

The installer copies Spud and nothing else. A virtual cable is a signed kernel
driver from VB-Audio, the A+B pack is licensed per machine, and redistributing it
needs a distribution licence. Download the pack from
<https://vb-audio.com/Cable/>, run its setup as administrator, and restart
Windows before opening Spud.

Spud's MSI and NSIS installers both install Spud alone. The NSIS build is per
machine, because Spud writes a sign-in entry.

## Using Spud as a microphone

A microphone is a **recording** device and a bus renders into a **playback**
device. Windows will not let an application write into a recording endpoint; the
audio engine answers `AUDCLNT_E_WRONG_ENDPOINT_TYPE`. That is why your microphone
never appears in a bus's device list, and why a cable is the way back.

1. On any input, your microphone or a virtual one, switch on the routing button
   for `B1`, `B2` or `B3`. That bus now carries the input.
2. In the program that needs a microphone (Discord, OBS, a game), choose
   `CABLE Output` as its input device. For the A and B pack, choose
   `CABLE-A Output` or `CABLE-B Output`.
3. To cover every program at once, set that Output as Windows' default recording
   device in Settings, System, Sound.

One rule is enforced rather than merely documented: a virtual input that
captures a cable cannot be routed back into that same cable. Doing so would send
the signal to its own input and howl. The routing button is struck through and
the engine refuses the assignment.

## The ceiling on every bus

Every bus ends in a **look-ahead peak limiter**, and it is on from the first
launch. It acts only above −0.3 dBFS, so a mix that never gets there sounds
exactly as it would with the limiter absent.

A bus is what another program records, and a signal that leaves on a cable above
full scale is clipped by every program that records it. A strip is easy to push
over that line without noticing: +4 dB of mid on the equaliser, +3 dB of
compressor makeup, a fader past unity. The sum of several strips adds up even
when no single one is loud.

- **Where it sits:** the very last thing on the bus, after the fader and the mono
  fold, immediately before the device.
- **It holds at −0.3 dBFS** (0.966), which leaves the last third of a decibel as
  headroom for the 24-bit cable and whatever records it.
- **It looks ahead 5 ms.** The audio is delayed by that much while the gain is
  worked out, so the gain is already down when a transient lands. The waveform is
  turned down instead of shaved off, which is the difference between a limiter
  and a clipper.
- **Release is 80 ms**, so a loud word ducks the bus briefly rather than for the
  rest of the sentence.
- **Both channels share one gain**, so a peak on one side never pulls the stereo
  image sideways.

The `LIM` chip sits in each bus column beside `M` and `Ø`. It is lit in moss while
the ceiling is on, the quiet colour, because protection is the normal state.
Press it and that bus is unlimited: its mix passes through sample for sample,
including anything above full scale.

### What the lamp means

The **red lamp** beside a bus meter lights while the mix **entering** the ceiling
is above it, which means the limiter is holding the bus down. It stays lit for a
second and a half after the last such moment, so a single transient is  visible instead of a one-frame flicker. The meter shows the mix as it leaves
  the bus fader and before the ceiling, so pulling the fader down moves it, and
  you can see how far over you are driving the bus while it leaves clean.

Read together: **meter over the ceiling with the lamp lit** means the limiter is
working. **Meter over the ceiling with the lamp dark** means the bus is unlimited
and that level is going to the device as it is.

### With the ceiling off

Switching `LIM` off on a bus that is driven hard is audible. Measured here with a
1 kHz tone into a strip raised 6 dB above unity, the bus carried 3.1 into the
ceiling, about +10 dBFS. What came out the far side of the cable:

| `LIM` | What left the bus | Distortion on the cable |
| --- | --- | --- |
| on | 0.966, the ceiling, a clean sine | −101 dB (none measurable) |
| off | passed through above full scale, squared off by the driver | −10 dB (31 % THD) |

That is the crunch a clipped virtual cable is known for, and one lit `LIM` chip
prevents it.

## Devices

Spud opens the streams it has been given at launch and reports any it could not
open. Two things reopen them:

- **By itself, when a device the wiring uses appears or leaves.** Spud compares
  the endpoint list with the last one about every two seconds. A microphone that
  was unplugged and plugged back in gets its stream back without anyone pressing
  anything. A device nobody assigned changes nothing, so the mix is not
  interrupted for a webcam.
- **The Rescan button in Settings**, which reopens every stream and fills empty
  inputs and outputs from the automatic wiring.

A slot you set to **None** stays empty. Rescan fills empty slots only when it is
pressed, and the automatic restart never rewrites a choice.

### If a bus will not open

Spud reports `the device is held exclusively by another program` when Windows
refuses a device. Some mixers, Voicemeeter is one, take their hardware outputs in
exclusive mode, so while they run nothing else can open them. Close the other
program, or assign that bus a different device.

## Settings

Settings holds what a settings window is for:

| Row | What it does |
| --- | --- |
| Engine | How many streams are open, and the reason for any assignment that failed |
| Updates | The installed version, a check on demand, and the install button |
| Rescan devices | Reopens every stream and fills empty slots |
| Appearance | Mode, presets, hue and saturation |
| Launch at sign-in | Writes or removes the Windows Run entry, and shows the copy it names |
| Virtual inputs | Which cable each virtual strip captures, and which bus feeds it |

Closing the window hides Spud to the notification area and the mix keeps
running. Left-click the tray icon to bring the console back; right-click it for
**Quit**, which stops the streams first.

**Launch at sign-in** writes one value under
`HKEY_CURRENT_USER\...\Run`, pointing at this executable with `--tray`. Sign-in
starts Spud hidden in the notification area, so the mix is live before you open
the window. Turning the switch off deletes that value. Nothing is written
anywhere else, and no administrator rights are involved.

## What Spud remembers

Closing the app and opening it again brings the console back as you left it:
every strip and bus keeps its fader position, input level, mute, solo, mono,
routing, equaliser and dynamics settings, label and device choice, including a
slot you deliberately set to None.

| What | Where |
| --- | --- |
| The console (faders, input levels, routing, equaliser, dynamics, labels, devices, each bus ceiling) | `%APPDATA%\dev.spud.console\settings.json` |
| Theme mode, hue and chroma | the window's own storage, written as you change it |
| Launch at sign-in | one value under `HKEY_CURRENT_USER\...\Run` |

The file is plain JSON, so it can be read, copied between machines, or deleted to
start from a fresh console (close Spud first). It is written half a second after
the last change, so dragging a fader is one write and not fifty, and immediately
when you quit from the tray. A route that would loop is dropped when the file is
loaded, in case a device moved since it was saved.

A microphone's equaliser, gate and compressor are restored once you have touched
them. Until then the voice chain is applied at every launch, so an upgrade can
improve the defaults without overwriting a setting you chose.

## Build and run

```sh
npm install
npm run tauri dev      # development window, hot reload
npm run tauri build    # Windows installer in src-tauri/target/release/bundle
```

Requirements: Node 18 or newer, a Rust toolchain with the MSVC target, and the
WebView2 runtime, which is present on Windows 10 and 11. The Rust half uses
`cpal` for audio devices and `windows` for WASAPI session enumeration.

## Tests

```sh
cd src-tauri && cargo test
```

The suite covers the mixing rules (routing, mute, solo, the fader taper, the
equaliser response, the low cut keeping rumble out of the gate), the gate's hold
and release, the compressor ratio, knee and makeup gain, the input level carrying
a quiet microphone over the gate's threshold, the denoiser pulling a room down and
leaving a voice, the bus ceiling holding a boosted strip at −0.3 dBFS and the lamp
reporting it, the bus meter reading after the fader, the ring and its
fractional-rate reader, the meter decay, the device-change detection, what the
console remembers,
the snapshot it draws, and, on a machine with audio devices, device enumeration
and the audio thread's refusal to die on a bad assignment.

## Updates

Spud checks `https://github.com/emrisca/spud/releases/latest/download/latest.json`
twenty seconds after launch and every six hours while it runs. When a newer
version is waiting, a row appears under the header with that version and the
first line of its notes. **Update now** downloads, verifies the signature and
installs it. **Later** hides that version until the next launch. Nothing installs
without the click, and a failed check is silent, because a machine with no
network is not a machine with a problem.

The keypair already exists. `.secrets/spud-updater.key` is the private half,
which Git ignores, and its public half is `plugins.updater.pubkey` in
`src-tauri/tauri.conf.json`. The key has no password, so a local release build
needs only the path:

```sh
TAURI_SIGNING_PRIVATE_KEY=.secrets/spud-updater.key npm run tauri build
```

Paste the contents of the key into the repository secret
`TAURI_SIGNING_PRIVATE_KEY`. A key generated without a password needs no
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Pushing a tag such as `v0.1.3` runs
`.github/workflows/release.yml` on a Windows runner: it installs, type checks,
builds the NSIS installer, writes `latest.json` beside it and publishes both.
The per-machine install mode means an update raises the usual Windows elevation
prompt.

The installed app reads `latest.json` without a token, so the repository has to
be public. A private one turns every update check into a 404.

A version bump is three files and one tag: `package.json`,
`src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` all say the same number,
and the tag is `v` plus that number. `tauri-action` takes `v__VERSION__` from the
config, so a tag that drifts publishes a release no installed copy will be
offered.

## How the audio is put together

```
capture device -> downmix -> resample to 48 kHz -> ring -┬-> bus stream -> output device
                                                        ├-> bus stream -> output device
                                                        └-> ...
```

- `src-tauri/src/mixer.rs` holds the parameters, meters, sample ring, channel
  strip (low cut, gate, four bands, compressor) and the bus limiter. Shared state
  is atomic, so no callback takes a lock.
- `src-tauri/src/audio.rs` gives one thread every `cpal::Stream`. A device change
  rebuilds the whole set, and the window never blocks on it. The same thread
  compares the endpoint list on a timer and rebuilds when a device the wiring
  uses appears or leaves.
- `src-tauri/src/sessions.rs` reads which programs are playing into each cable
  from Windows audio sessions.
- `src-tauri/src/state.rs` binds the console's vocabulary (`Strip[2].Gain`,
  `Bus[5].Mute`, `A1` to `B3`) to the mixer and builds the snapshot the UI draws.
- `src-tauri/src/store.rs` writes the console to `settings.json` and reads it
  back, dropping a route that has become a loop.
- `src-tauri/src/startup.rs` owns the sign-in entry. `src-tauri/src/update.rs`
  owns the updater state.

Each bus mixes for itself from the input rings, so an output at 44.1 kHz and one
at 48 kHz run side by side, and a device that fails to open cannot starve the
others. The console polls levels at 20 Hz and the snapshot at 1.25 Hz, and skips
a snapshot that has not changed, so an idle window stays idle.

## Not in this build

The FX sends, panning, recording, and the multichannel position switches are not
wired to audio, so the console shows no controls for them rather than offering
buttons that would not move audio.
