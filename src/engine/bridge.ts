/**
 * One interface, two implementations.
 *
 * In the desktop window every call is a Tauri command into the mixer in this
 * process. In a browser (`npm run dev` opened directly) there is no engine, so a
 * small in-memory stand-in answers instead, with the same shapes, so the
 * console can be designed and tested without touching anyone's audio.
 */

import { invoke } from "@tauri-apps/api/core";
import {
  ROUTES,
  applyParam,
  type Bus,
  type DeviceDesc,
  type EngineStatus,
  type Levels,
  type AppSession,
  type Snapshot,
  type Strip,
  type VirtualInput,
} from "./model";

export const isPreview = !("__TAURI_INTERNALS__" in window);

/** Emitted when an executable chosen in the file picker comes back. */
export const STARTUP_EXE = "startup-exe";

/** The sign-in entry as Windows has it, and the copy running now. */
export type StartupView = {
  enabled: boolean;
  /** The executable the entry names; null when there is no entry. */
  entry: string | null;
  /** This process's own executable. */
  running: string | null;
};

/**
 * An update as the shell reports it: what is waiting, and how far a download
 * got. The status is one of `idle`, `checking`, `available`, `downloading`,
 * `installing`, `restarting`, `upToDate` or `error`.
 */
export type UpdateView = {
  currentVersion: string;
  status: string;
  version: string | null;
  notes: string | null;
  downloaded: number;
  total: number | null;
  error: string | null;
};

/** What an unassigned strip reports: the engine's own resting values. */
const blankComp = () => ({
  threshold: -16,
  ratio: 2.5,
  attack: 11,
  release: 180,
  knee: 6,
  gain_in: 0,
  gain_out: 0,
});

const blankGate = () => ({
  threshold: -60,
  attack: 0,
  release: 220,
  hold: 160,
  damping: 0,
});

/** The microphone chain the engine seeds, mirrored from `lib.rs`. */
const VOICE_GATE = {
  threshold: -42,
  attack: 3,
  release: 200,
  hold: 220,
  damping: -50,
};
const VOICE_COMP = {
  threshold: -18,
  ratio: 3,
  attack: 10,
  release: 150,
  knee: 6,
  gain_in: 0,
  gain_out: 3,
};

const MIC = "Microphone (2- HyperX Cloud III Wireless)";
const HEADPHONES = "Headphones (2- HyperX Cloud III Wireless)";
const CABLE_IN = "CABLE Input (VB-Audio Virtual Cable)";
const CABLE_A_IN = "CABLE-A Input (VB-Audio Virtual Cable A)";
const CABLE_B_IN = "CABLE-B Input (VB-Audio Virtual Cable B)";
const CABLE_OUT = "CABLE Output (VB-Audio Virtual Cable)";
const CABLE_A_OUT = "CABLE-A Output (VB-Audio Virtual Cable A)";
const CABLE_B_OUT = "CABLE-B Output (VB-Audio Virtual Cable B)";

/** Mirrors the wiring the engine picks on a machine with one cable. */
function initialSnapshot(): Snapshot {
  const devices = [MIC, "", "", "", "", CABLE_OUT, CABLE_A_OUT, CABLE_B_OUT];
  const strips: Strip[] = Array.from({ length: 8 }, (_, index) => ({
    index,
    label: "",
    device: devices[index],
    virtual_input: index >= 5,
    gain: index === 0 ? 6 : 0,
    trim: 0,
    mute: false,
    solo: false,
    mono: false,
    routes: ROUTES.map((route) => route === "A1"),
    position: [false, false, false, false],
    mc: false,
    // The engine seeds the voice chain (EQ, gate and compressor) on the
    // hardware inputs; the virtual ones start switched out, because they are
    // carrying other programs already.
    comp: index < 5 ? 1 : 0,
    gate: index < 5 ? 1 : 0,
    eq: index < 5,
    eq_gain: index < 5 ? [2, 1.5, -2, -3] : [0, 0, 0, 0],
    eq_cut: index < 5 ? 90 : 0,
    pan: 0,
    limit: 0,
    denoiser: 0,
    fx1: 0,
    fx2: 0,
    comp_detail: index < 5 ? { ...VOICE_COMP } : blankComp(),
    gate_detail: index < 5 ? { ...VOICE_GATE } : blankGate(),
  }));

  const names = [HEADPHONES, "", "", "", "", CABLE_IN, CABLE_A_IN, CABLE_B_IN];
  const buses: Bus[] = Array.from({ length: 8 }, (_, index) => ({
    index,
    key: ROUTES[index],
    physical: index < 5,
    label: "",
    device: names[index],
    gain: index === 0 ? -4 : 0,
    mute: false,
    mono: false,
    limit: true,
    eq: false,
    sel: false,
  }));

  return { strips, buses, sample_rate: 48000, buffer: 480, latency: 20 };
}

let mock = initialSnapshot();
/** The browser stand-in remembers the switch for the session; the engine uses
 * Windows' own Run key. */
let previewStartup = false;
let previewStartupPath: string | null = null;

/** Where a browser stand-in pretends this copy lives. */
const PREVIEW_RUNNING = "C:\\Program Files\\Spud\\spud.exe";

/** What the browser stand-in reports for the sign-in entry. */
const previewStartupView = (): StartupView => ({
  enabled: previewStartup,
  entry: previewStartup ? (previewStartupPath ?? PREVIEW_RUNNING) : null,
  running: PREVIEW_RUNNING,
});

function applyToMock(name: string, value: number): boolean {
  return applyParam(mock, name, value);
}

function applyScriptToMock(script: string): boolean {
  let ok = true;
  for (const statement of script.split(";")) {
    const match = /^\s*([A-Za-z]+\[\d+\]\.[\w.]+)\s*=\s*([-\d.]+)\s*$/.exec(statement);
    if (match) ok = applyToMock(match[1], Number(match[2])) && ok;
  }
  return ok;
}

/** A slow, believable movement so meters are legible without an engine. */
function mockLevels(): Levels {
  const t = performance.now() / 1000;
  const strips = mock.strips.map((strip, i) => {
    if (strip.mute || !strip.device) return 0;
    const activity = 0.12 + 0.55 * Math.abs(Math.sin(t * 0.9 + i * 1.37) * Math.sin(t * 0.31 + i));
    return strip.index === 0 ? activity : activity * 0.4;
  });
  const buses = mock.buses.map((bus, i) => {
    if (bus.mute || !bus.device) return 0;
    const fed = mock.strips.some((strip) => !strip.mute && strip.routes[i]);
    return fed ? 0.1 + 0.4 * Math.abs(Math.sin(t * 0.7 + i)) : 0;
  });
  // The stand-in's ceiling is the engine's -0.3 dBFS, and only the browser
  // preview's lamps read it: the running console is handed the real number.
  return { strips, buses, buses_post: buses, ceiling: 10 ** (-0.3 / 20) };
}

const MOCK_OUTPUTS: DeviceDesc[] = [
  { index: 0, kind: "wdm", name: HEADPHONES, hardware: "default" },
  { index: 1, kind: "wdm", name: "LG ULTRAGEAR (NVIDIA High Definition Audio)", hardware: "WASAPI" },
  { index: 2, kind: "wdm", name: "Realtek Digital Output (Realtek(R) Audio)", hardware: "WASAPI" },
  { index: 3, kind: "wdm", name: CABLE_IN, hardware: "WASAPI" },
  { index: 4, kind: "wdm", name: CABLE_A_IN, hardware: "WASAPI" },
  { index: 5, kind: "wdm", name: CABLE_B_IN, hardware: "WASAPI" },
];

const MOCK_INPUTS: DeviceDesc[] = [
  { index: 0, kind: "wdm", name: MIC, hardware: "default" },
  { index: 1, kind: "wdm", name: "Line In (Realtek(R) Audio)", hardware: "WASAPI" },
  { index: 2, kind: "wdm", name: CABLE_OUT, hardware: "WASAPI" },
  { index: 3, kind: "wdm", name: CABLE_A_OUT, hardware: "WASAPI" },
  { index: 4, kind: "wdm", name: CABLE_B_OUT, hardware: "WASAPI" },
];

const MOCK_STATUS: EngineStatus = {
  found: true,
  dll: null,
  kind: 3,
  edition: "Spud",
  version: "0.1.0",
  strips: 8,
  buses: 8,
  running: true,
  inputs: 2,
  outputs: 2,
  errors: [],
};

/** The browser stand-in is never offered a release, because there is no
 * installer behind the page. */
const PREVIEW_UPDATE: UpdateView = {
  currentVersion: "0.1.1",
  status: "upToDate",
  version: null,
  notes: null,
  downloaded: 0,
  total: null,
  error: null,
};

export const engine = {
  status(): Promise<EngineStatus> {
    return isPreview ? Promise.resolve(MOCK_STATUS) : invoke<EngineStatus>("engine_status");
  },

  snapshot(): Promise<Snapshot | null> {
    if (isPreview) return Promise.resolve(structuredClone(mock));
    return invoke<Snapshot | null>("snapshot");
  },

  levels(): Promise<Levels> {
    return isPreview ? Promise.resolve(mockLevels()) : invoke<Levels>("levels");
  },

  setParam(name: string, value: number): Promise<boolean> {
    if (isPreview) return Promise.resolve(applyToMock(name, value));
    return invoke<boolean>("set_param", { name, value });
  },

  setText(name: string, value: string): Promise<boolean> {
    if (isPreview) {
      const match = /^(?:Strip|Bus)\[(\d+)\]\.Label$/.exec(name);
      if (match) {
        const index = Number(match[1]);
        const target = name.startsWith("Strip") ? mock.strips[index] : mock.buses[index];
        if (target) target.label = value;
      }
      return Promise.resolve(true);
    }
    return invoke<boolean>("set_text", { name, value });
  },

  script(script: string): Promise<boolean> {
    if (isPreview) return Promise.resolve(applyScriptToMock(script));
    return invoke<boolean>("apply_script", { script });
  },

  setBusDevice(bus: number, name: string): Promise<boolean> {
    if (isPreview) {
      const target = mock.buses[bus];
      if (target) target.device = name;
      return Promise.resolve(true);
    }
    return invoke<boolean>("set_bus_device", { bus, kind: "wdm", name });
  },

  setStripDevice(strip: number, name: string): Promise<boolean> {
    if (isPreview) {
      const target = mock.strips[strip];
      if (target) target.device = name;
      return Promise.resolve(true);
    }
    return invoke<boolean>("set_strip_device", { strip, name });
  },

  /** The sign-in entry: whether there is one, and which copy it aims at. */
  startup(): Promise<StartupView> {
    return isPreview ? Promise.resolve(previewStartupView()) : invoke<StartupView>("startup_state");
  },

  /** Turn the sign-in entry on or off, aimed at the copy that is running. */
  setStartup(enabled: boolean): Promise<StartupView> {
    if (isPreview) {
      previewStartup = enabled;
      return Promise.resolve(previewStartupView());
    }
    return invoke<StartupView>("set_startup", { enabled });
  },

  /** Aim the sign-in entry at another copy, creating the entry if it was off. */
  pointStartupAt(path: string): Promise<StartupView> {
    if (isPreview) {
      previewStartup = true;
      previewStartupPath = path;
      return Promise.resolve(previewStartupView());
    }
    return invoke<StartupView>("point_startup_at", { path });
  },

  /** Open Windows' file picker; the answer comes back as `STARTUP_EXE`. */
  chooseStartupExe(): Promise<void> {
    return isPreview ? Promise.resolve() : invoke<void>("choose_startup_exe");
  },

  /** Reopen every stream and fill unassigned slots. */
  rescan(): Promise<boolean> {
    if (isPreview) return Promise.resolve(true);
    return invoke<boolean>("run_engine");
  },

  /** What the updater last found. Polled; the shell changes it on its own clock. */
  updateState(): Promise<UpdateView> {
    return isPreview ? Promise.resolve(PREVIEW_UPDATE) : invoke<UpdateView>("update_state");
  },

  /** Ask the release endpoint again, now. */
  updateCheck(): Promise<UpdateView> {
    return isPreview ? Promise.resolve(PREVIEW_UPDATE) : invoke<UpdateView>("update_check");
  },

  /** Download, verify and install; the shell restarts itself when it lands. */
  updateInstall(): Promise<void> {
    return isPreview ? Promise.resolve() : invoke<void>("update_install");
  },

  devices(): Promise<[DeviceDesc[], DeviceDesc[]]> {
    return isPreview
      ? Promise.resolve([MOCK_OUTPUTS, MOCK_INPUTS])
      : invoke<[DeviceDesc[], DeviceDesc[]]>("devices");
  },

  /** Programs playing into each virtual cable, straight from Windows. */
  sessions(): Promise<VirtualInput[]> {
    if (isPreview) {
      const apps: AppSession[] = [
        { pid: 18096, name: "steam", volume: 1, mute: false, playing: false },
        { pid: 12556, name: "Discord", volume: 0.62, mute: false, playing: true },
      ];
      return Promise.resolve([
        { strip: 5, endpoint: CABLE_IN, apps: apps.slice(0, 1) },
        { strip: 6, endpoint: CABLE_A_IN, apps: [] },
        { strip: 7, endpoint: CABLE_B_IN, apps: apps.slice(1) },
      ]);
    }
    return invoke<VirtualInput[]>("app_sessions");
  },

  /** Per-application volume and mute, on the program's own audio session. */
  setAppAudio(pid: number, patch: { volume?: number; mute?: boolean }): Promise<boolean> {
    if (isPreview) return Promise.resolve(true);
    return invoke<boolean>("set_app_audio", { pid, ...patch });
  },
};
