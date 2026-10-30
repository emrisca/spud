/**
 * The console's model of the mixer.
 *
 * Names and shapes mirror `src-tauri/src/state.rs`. A control exists here only
 * when the engine behind it does: gain, mute, solo, mono, routing, three bands
 * of EQ and the meters.
 */

/** Potato's routing targets, in panel order. A1–A5 are physical, B1–B3 virtual. */
export const ROUTES = ["A1", "A2", "A3", "A4", "A5", "B1", "B2", "B3"] as const;
export type Route = (typeof ROUTES)[number];

export const GAIN_MIN = -60;
export const GAIN_MAX = 12;
export const LEVEL_TOP = 0;
export const LEVEL_BOTTOM = -60;

export type CompDetail = {
  threshold: number;
  ratio: number;
  attack: number;
  release: number;
  knee: number;
  gain_in: number;
  gain_out: number;
};

export type GateDetail = {
  threshold: number;
  attack: number;
  release: number;
  hold: number;
  damping: number;
};

export type Strip = {
  index: number;
  label: string;
  device: string;
  virtual_input: boolean;
  gain: number;
  /**
   * Level into the strip in dB, ahead of the gate, the equaliser and the
   * compressor. Distinct from `gain`, which is the fader and sits after them.
   */
  trim: number;
  mute: boolean;
  solo: boolean;
  mono: boolean;
  routes: boolean[];
  /** Eight-channel position switches: Front, Rear, Center, LFE. */
  position: boolean[];
  mc: boolean;
  comp: number;
  gate: number;
  eq: boolean;
  /** Treble, mid, bass, body — the order the dialog and the knobs read. */
  eq_gain: number[];
  /** Low-cut corner in Hz, 0 when it is off. */
  eq_cut: number;
  pan: number;
  limit: number;
  denoiser: number;
  fx1: number;
  fx2: number;
  comp_detail: CompDetail;
  gate_detail: GateDetail;
};

export type Bus = {
  index: number;
  key: string;
  physical: boolean;
  label: string;
  device: string;
  gain: number;
  mute: boolean;
  mono: boolean;
  /** The ceiling the engine holds this bus under. */
  limit: boolean;
  eq: boolean;
  sel: boolean;
};

export type Snapshot = {
  strips: Strip[];
  buses: Bus[];
  sample_rate: number;
  /** Frames per capture callback, as the devices deliver them. */
  buffer: number;
  /** What the readers hold behind the devices, in milliseconds. */
  latency: number;
};

/** The position switches in the order the engine's `mode.*` names use. */
export const POSITION = ["Front", "Rear", "Center", "LFE"] as const;

export type Levels = {
  strips: number[];
  buses: number[];
  /** The mix entering each bus's ceiling, before the limiter pulls it down. */
  buses_post: number[];
  /** The engine's ceiling, in samples. The lamp compares against this. */
  ceiling: number;
};

export type EngineStatus = {
  found: boolean;
  dll: string | null;
  kind: number;
  edition: string;
  version: string;
  strips: number;
  buses: number;
  /** True while at least one stream is open. */
  running: boolean;
  /** Inputs and outputs with a live stream. */
  inputs: number;
  outputs: number;
  /** Assignments that could not be opened, with the reason. */
  errors: string[];
};

/** One program playing into a virtual input, from its Windows audio session. */
export type AppSession = {
  pid: number;
  name: string;
  volume: number;
  mute: boolean;
  playing: boolean;
};

export type VirtualInput = {
  strip: number;
  endpoint: string;
  apps: AppSession[];
};

export type DeviceDesc = {
  index: number;
  kind: string;
  name: string;
  hardware: string;
};

export const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

/** Fader position (0..1) for a gain in dB. */
export const gainToPos = (db: number) =>
  (clamp(db, GAIN_MIN, GAIN_MAX) - GAIN_MIN) / (GAIN_MAX - GAIN_MIN);

export const posToGain = (pos: number) =>
  GAIN_MIN + clamp(pos, 0, 1) * (GAIN_MAX - GAIN_MIN);

/** `-∞` at the bottom, `+9.7`, `0.0`, `-5.3` — the engine's own resolution. */
export function formatDb(db: number): string {
  if (!Number.isFinite(db) || db <= GAIN_MIN + 0.05) return "-∞";
  const rounded = Math.abs(db) < 0.05 ? 0 : db;
  return `${rounded > 0 ? "+" : ""}${rounded.toFixed(1)}`;
}

/** True while either half of a strip's dynamics is switched in. */
export const dynamicsOn = (strip: Strip) => strip.gate >= 0.5 || strip.comp >= 0.5;

/** Which halves, for the chip that opens them. */
export const dynamicsLabel = (strip: Strip) =>
  `gate ${strip.gate >= 0.5 ? "on" : "off"}, compressor ${strip.comp >= 0.5 ? "on" : "off"}`;

/** Linear peak to dBFS, floored at the meter's bottom. */
export function toDb(linear: number): number {
  if (!Number.isFinite(linear) || linear <= 1e-6) return LEVEL_BOTTOM;
  return Math.max(LEVEL_BOTTOM, 20 * Math.log10(linear));
}

/** dBFS to a 0..1 meter height. */
export const dbToPos = (db: number) =>
  clamp((db - LEVEL_BOTTOM) / (LEVEL_TOP - LEVEL_BOTTOM), 0, 1);

export const routeName = (strip: number, route: Route) => `Strip[${strip}].${route}`;
export const stripParam = (strip: number, field: string) => `Strip[${strip}].${field}`;
export const busParam = (bus: number, field: string) => `Bus[${bus}].${field}`;

/**
 * Which cable a device belongs to, ignoring direction — the engine's own rule.
 *
 * Both sides of one cable answer the same key (`CABLE-A Input` and
 * `CABLE-A Output` are both `cable-a`), which is what makes "this input captures
 * the cable that bus feeds" answerable without a device table.
 */
export function cableKey(name: string): string | null {
  if (!name) return null;
  const body = name.split("(")[0].trim().toLowerCase();
  if (!body.startsWith("cable")) return null;
  const suffixes: [string, string][] = [
    [" input", ""],
    [" output", ""],
    [" in 16 ch", " 16ch"],
    [" out 16 ch", " 16ch"],
  ];
  for (const [suffix, mark] of suffixes) {
    if (body.endsWith(suffix)) return body.slice(0, -suffix.length) + mark;
  }
  return null;
}

/**
 * Whether this input may not be sent to `position`.
 *
 * A virtual input captures a cable's Output side and the matching B bus renders
 * into that same cable's Input side, so routing one into the other is a loop
 * through Windows — the bus would hear itself. The engine refuses it; the
 * console says so before the click.
 */
export function loopsCable(strip: Strip, buses: Bus[], position: number): boolean {
  const mine = cableKey(strip.device);
  return mine !== null && mine === cableKey(buses[position]?.device ?? "");
}

/**
 * `CABLE-A Input (VB-Audio Virtual Cable A)` -> `CABLE-A`.
 *
 * The bus columns are 44 px wide, and `CABLE Input` / `CABLE-A Input` read the
 * same at that size: the only part worth showing is which cable it is. Other
 * devices keep their first words, which is where the name is.
 */
export function shortDevice(name: string): string {
  const head = name.split("(")[0].trim();
  if (!head) return "";
  if (!cableKey(name)) {
    // A bus column is 44 px: "Headphones" broke into "Headpho / nes" there, so
    // the headphone output keeps the console's own word for it. The button's
    // tooltip and the device dialog still show the full name.
    const plain = head.replace(/\s+/g, " ");
    return /^headphones?$/i.test(plain) ? "Phones" : plain;
  }
  const wide = /16 ?ch/i.test(head);
  const stem = head.replace(/\s+(input|output|in 16 ?ch|out 16 ?ch)$/i, "").trim();
  return wide && !/16 ?ch$/i.test(stem) ? `${stem} 16ch` : stem;
}

/** Labels the mixer keeps per strip, editable from the console. */
export const DEFAULT_LABELS = [
  "Hardware 1",
  "Hardware 2",
  "Hardware 3",
  "Hardware 4",
  "Hardware 5",
  "Virtual 1",
  "Virtual 2",
  "Virtual 3",
];

export const stripTitle = (strip: Strip) =>
  strip.label.trim() || DEFAULT_LABELS[strip.index] || `Strip ${strip.index + 1}`;

/**
 * Apply one `Strip[i].Field` / `Bus[i].Field` assignment to a snapshot in
 * place, returning whether it landed.
 *
 * The console uses this twice: for the browser stand-in, and to move a control
 * the moment it is touched instead of waiting for the next poll. The engine is
 * still the authority — the next snapshot replaces whatever this guessed.
 */
export function applyParam(snapshot: Snapshot, name: string, value: number): boolean {
  const match = /^(Strip|Bus)\[(\d+)\]\.(.+)$/.exec(name);
  if (!match) return false;
  const index = Number(match[2]);
  const field = match[3];
  const on = value >= 0.5;

  if (match[1] === "Strip") {
    const strip = snapshot.strips[index];
    if (!strip) return false;
    const comp: Record<string, (v: number) => void> = {
      "Comp.Threshold": (v) => (strip.comp_detail.threshold = v),
      "Comp.Ratio": (v) => (strip.comp_detail.ratio = v),
      "Comp.Attack": (v) => (strip.comp_detail.attack = v),
      "Comp.Release": (v) => (strip.comp_detail.release = v),
      "Comp.Knee": (v) => (strip.comp_detail.knee = v),
      "Comp.GainIn": (v) => (strip.comp_detail.gain_in = v),
      "Comp.GainOut": (v) => (strip.comp_detail.gain_out = v),
      "Gate.Threshold": (v) => (strip.gate_detail.threshold = v),
      "Gate.Attack": (v) => (strip.gate_detail.attack = v),
      "Gate.Release": (v) => (strip.gate_detail.release = v),
      "Gate.Hold": (v) => (strip.gate_detail.hold = v),
      "Gate.Damping": (v) => (strip.gate_detail.damping = v),
      "EQGain1": (v) => (strip.eq_gain[0] = v),
      "EQGain2": (v) => (strip.eq_gain[1] = v),
      "EQGain3": (v) => (strip.eq_gain[2] = v),
      "EQGain4": (v) => (strip.eq_gain[3] = v),
      EQCut: (v) => (strip.eq_cut = v < 20 ? 0 : v),
    };
    if (field in comp) {
      comp[field](value);
      return true;
    }

    const scalars: Record<string, (v: number) => void> = {
      Gain: (v) => (strip.gain = v),
      Trim: (v) => (strip.trim = v),
      Mute: () => (strip.mute = on),
      Solo: () => (strip.solo = on),
      Mono: () => (strip.mono = on),
      Comp: (v) => (strip.comp = v),
      MC: () => (strip.mc = on),
      Gate: (v) => (strip.gate = v),
      "EQ.on": () => (strip.eq = on),
      Pan_x: (v) => (strip.pan = v),
      Limit: (v) => (strip.limit = v),
      Denoiser: (v) => (strip.denoiser = v),
      Fx1: (v) => (strip.fx1 = v),
      Fx2: (v) => (strip.fx2 = v),
    };
    if (field in scalars) {
      scalars[field](value);
      return true;
    }

    const route = (ROUTES as readonly string[]).indexOf(field);
    if (route >= 0) {
      // The engine refuses a cable routed back into itself; mirror that so the
      // optimistic update does not show a button the engine declined.
      const mine = cableKey(strip.device);
      if (on && mine !== null && mine === cableKey(snapshot.buses[route]?.device ?? "")) {
        return false;
      }
      strip.routes[route] = on;
      return true;
    }

    if (field.startsWith("mode.")) {
      const axis = (POSITION as readonly string[]).indexOf(field.slice(5));
      if (axis >= 0) {
        strip.position[axis] = on;
        return true;
      }
    }
    return false;
  }

  const bus = snapshot.buses[index];
  if (!bus) return false;
  const busScalars: Record<string, (v: number) => void> = {
    Gain: (v) => (bus.gain = v),
    Mute: () => (bus.mute = on),
    Mono: () => (bus.mono = on),
    Limit: () => (bus.limit = on),
    "EQ.on": () => (bus.eq = on),
    Sel: () => (bus.sel = on),
  };
  if (field in busScalars) {
    busScalars[field](value);
    return true;
  }
  return false;
}
