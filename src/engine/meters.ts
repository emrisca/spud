/**
 * Meter ballistics.
 *
 * The engine hands back linear peaks 20 times a second. Raw numbers make a
 * jittery bar, so peaks are held for a moment and fall at a fixed rate — the
 * behaviour of every hardware meter, and the reason a fader move reads as a
 * move rather than a flicker.
 *
 * Kept outside React so a 20 Hz update repaints the meters and nothing else.
 */

import { useSyncExternalStore } from "react";
import { LEVEL_BOTTOM, toDb, type Levels } from "./model";

const CHANNELS = 8;
const FALL_DB_PER_POLL = 1.4;
const CLIP_HOLD_MS = 1500;

const stripDb: number[] = Array(CHANNELS).fill(LEVEL_BOTTOM);
const busDb: number[] = Array(CHANNELS).fill(LEVEL_BOTTOM);
const clipping: number[] = Array(CHANNELS).fill(0);

const listeners = new Set<() => void>();

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

const notify = () => listeners.forEach((listener) => listener());

const rise = (current: number, sample: number) =>
  sample > current ? sample : Math.max(sample, current - FALL_DB_PER_POLL);

/**
 * Feed one poll in. `buses_post` is the mix *entering* the ceiling, so a bus
 * above `ceiling` is a bus the limiter is holding down — which is what the lamp
 * reports. The threshold comes from the engine rather than a number here: a copy
 * of it is how the lamp ends up lighting at full scale while the ceiling sits
 * half a decibel lower and the limiter's work goes unshown.
 */
export function pushLevels(levels: Levels): void {
  for (let i = 0; i < CHANNELS; i += 1) {
    stripDb[i] = rise(stripDb[i], toDb(levels.strips[i] ?? 0));
    busDb[i] = rise(busDb[i], toDb(levels.buses[i] ?? 0));
    if ((levels.buses_post[i] ?? 0) > levels.ceiling) clipping[i] = performance.now() + CLIP_HOLD_MS;
  }
  notify();
}

export function silence(): void {
  for (let i = 0; i < CHANNELS; i += 1) {
    stripDb[i] = LEVEL_BOTTOM;
    busDb[i] = LEVEL_BOTTOM;
    clipping[i] = 0;
  }
  notify();
}

/** One channel's level in dBFS. Only that meter re-renders when it changes. */
export function useMeterDb(kind: "strip" | "bus", index: number): number {
  return useSyncExternalStore(
    subscribe,
    () => (kind === "strip" ? stripDb[index] : busDb[index]) ?? LEVEL_BOTTOM,
  );
}

/** True while the bus has clipped within the last second and a half. */
export function useClip(index: number): boolean {
  return useSyncExternalStore(subscribe, () => clipping[index] > performance.now());
}
