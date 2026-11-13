import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppearanceDialog } from "./AppearanceDialog";
import { UpdateBanner } from "./UpdateBanner";
import { Console } from "./Console";
import { DeviceDialog, SettingsDialog, type PickTarget } from "./Dialogs";
import { DynamicsDialog } from "./DynamicsDialog";
import { MicEqDialog } from "./MicEqDialog";
import { listen } from "@tauri-apps/api/event";
import { STARTUP_EXE, engine, isPreview, type StartupView } from "@/engine/bridge";
import { pushLevels, silence } from "@/engine/meters";
import {
  applyParam,
  shortDevice,
  stripTitle,
  type EngineStatus,
  type Snapshot,
  type VirtualInput,
} from "@/engine/model";
import { ThemeProvider } from "@/theme/ThemeProvider";

const LEVEL_POLL_MS = 50;
const SNAPSHOT_POLL_MS = 800;
const SESSION_POLL_MS = 1500;

const Gear = () => (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <circle cx="12" cy="12" r="3.1" />
    <path d="M19.4 14.5a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1.08-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 8.5a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
  </svg>
);

export function App() {
  return (
    <ThemeProvider>
      <Shell />
    </ThemeProvider>
  );
}

function Shell() {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [apps, setApps] = useState<VirtualInput[]>([]);
  const [picking, setPicking] = useState<PickTarget | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [appearanceOpen, setAppearanceOpen] = useState(false);
  /** Which input's equaliser is open, if any. */
  const [eqStrip, setEqStrip] = useState<number | null>(null);
  /** Which input's gate and compressor are open, if any. */
  const [dynStrip, setDynStrip] = useState<number | null>(null);
  const [restarting, setRestarting] = useState(false);
  /** The sign-in entry as Windows holds it: the switch, its target, and this copy. */
  const [startup, setStartup] = useState<StartupView>({
    enabled: false,
    entry: null,
    running: null,
  });

  /**
   * The last snapshot and status the window accepted, as text.
   *
   * The mixer is polled twice a second and most polls find nothing changed.
   * Comparing the answer with the last one keeps an idle console from
   * re-rendering eight strips and eight buses for nothing; a fader being dragged
   * still lands, because the engine's answer then differs from the last poll.
   */
  const seen = useRef({ status: "", snapshot: "" });

  const acceptStatus = useCallback((next: EngineStatus) => {
    const key = JSON.stringify(next);
    if (key === seen.current.status) return;
    seen.current.status = key;
    setStatus(next);
  }, []);

  const acceptSnapshot = useCallback((next: Snapshot) => {
    const key = JSON.stringify(next);
    if (key === seen.current.snapshot) return;
    seen.current.snapshot = key;
    setSnapshot(next);
  }, []);

  const refresh = useCallback(async () => {
    const [nextStatus, nextSnapshot] = await Promise.all([engine.status(), engine.snapshot()]);
    acceptStatus(nextStatus);
    if (nextSnapshot) acceptSnapshot(nextSnapshot);
    return nextStatus;
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The sign-in entry lives in Windows' Run key, not in the mixer, so it is
  // read once rather than polled with the snapshot.
  useEffect(() => {
    void engine.startup().then(setStartup);
  }, []);

  const toggleStartup = useCallback(async (enabled: boolean) => {
    setStartup(await engine.setStartup(enabled));
  }, []);

  const pointStartupAt = useCallback(async (path: string) => {
    setStartup(await engine.pointStartupAt(path));
  }, []);

  const chooseStartupExe = useCallback(() => {
    void engine.chooseStartupExe();
  }, []);

  // The file picker answers long after the click that opened it, and from
  // outside this window, so the chosen path arrives as an event rather than as
  // a return value. It names the copy Windows should start, which need not be
  // the copy running now.
  useEffect(() => {
    if (isPreview) return;
    const stop = listen<string>(STARTUP_EXE, (event) => {
      void pointStartupAt(event.payload);
    });
    return () => {
      void stop.then((off) => off());
    };
  }, [pointStartupAt]);

  // The mixer is this process, so the snapshot is cheap; 800 ms keeps a rename
  // or a second window in step without polling hard. The status rides the same
  // clock: streams open a moment after launch, so a status read made only at
  // mount left the header saying "no streams open" over a running mix, and
  // hid a device that failed to open for as long as the window stayed up.
  useEffect(() => {
    let live = true;
    const id = window.setInterval(async () => {
      const [nextStatus, next] = await Promise.all([engine.status(), engine.snapshot()]);
      if (!live) return;
      acceptStatus(nextStatus);
      if (next) acceptSnapshot(next);
    }, SNAPSHOT_POLL_MS);
    return () => {
      live = false;
      window.clearInterval(id);
    };
  }, []);

  // Audio sessions come from Windows, not the mixer, so they get their own clock.
  useEffect(() => {
    let live = true;
    const load = async () => {
      const sessions = await engine.sessions();
      if (live) setApps(sessions);
    };
    void load();
    const id = window.setInterval(load, SESSION_POLL_MS);
    return () => {
      live = false;
      window.clearInterval(id);
    };
  }, []);

  useEffect(() => {
    let live = true;
    const id = window.setInterval(async () => {
      const levels = await engine.levels();
      if (live) pushLevels(levels);
    }, LEVEL_POLL_MS);
    return () => {
      live = false;
      window.clearInterval(id);
      silence();
    };
  }, []);

  const write = useCallback((name: string, value: number) => {
    setSnapshot((current) => {
      if (!current) return current;
      const next = structuredClone(current);
      return applyParam(next, name, value) ? next : current;
    });
    void engine.setParam(name, value);
  }, []);

  const writeText = useCallback((name: string, value: string) => {
    setSnapshot((current) => {
      if (!current) return current;
      const next = structuredClone(current);
      const match = /^(Strip|Bus)\[(\d+)\]\.Label$/.exec(name);
      if (match) {
        const index = Number(match[2]);
        const target = match[1] === "Strip" ? next.strips[index] : next.buses[index];
        if (target) target.label = value;
      }
      return next;
    });
    void engine.setText(name, value);
  }, []);

  const setApp = useCallback(
    (pid: number, patch: { volume?: number; mute?: boolean }) => {
      setApps((current) =>
        current.map((input) => ({
          ...input,
          apps: input.apps.map((app) =>
            app.pid === pid
              ? {
                  ...app,
                  volume: patch.volume ?? app.volume,
                  mute: patch.mute ?? app.mute,
                }
              : app,
          ),
        })),
      );
      void engine.setAppAudio(pid, patch);
    },
    [],
  );

  const rescan = useCallback(async () => {
    setRestarting(true);
    await engine.rescan();
    window.setTimeout(async () => {
      await refresh();
      setRestarting(false);
    }, 900);
  }, [refresh]);

  /** A device was chosen: show it at once, then let the engine rebuild streams. */
  const choose = useCallback(
    (kind: PickTarget["kind"], index: number, name: string) => {
      setSnapshot((current) => {
        if (!current) return current;
        const next = structuredClone(current);
        if (kind === "bus") {
          if (next.buses[index]) next.buses[index].device = name;
        } else if (next.strips[index]) {
          next.strips[index].device = name;
        }
        return next;
      });
      if (kind === "bus") void engine.setBusDevice(index, name);
      else void engine.setStripDevice(index, name);
      setPicking(null);
    },
    [],
  );

  const target = useMemo(() => {
    if (!picking || !snapshot) return null;
    if (picking.kind === "bus") {
      const bus = snapshot.buses[picking.index];
      return bus ? { ...picking, name: `Output for ${bus.key}`, current: bus.device } : null;
    }
    const strip = snapshot.strips[picking.index];
    return strip
      ? { ...picking, name: `Input for ${stripTitle(strip)}`, current: strip.device }
      : null;
  }, [picking, snapshot]);

  // Which cable each virtual input captures and which bus pours into it. The
  // bus column has room for the cable's name and nothing else, so the settings
  // panel is where the three-way pairing is written out.
  const pairs = useMemo(() => {
    if (!snapshot) return [];
    return snapshot.strips.slice(5).flatMap((strip, offset) => {
      const bus = snapshot.buses[5 + offset];
      if (!bus || !strip.device) return [];
      return [
        {
          strip: stripTitle(strip),
          bus: bus.key,
          cable: shortDevice(bus.device) || "no cable",
        },
      ];
    });
  }, [snapshot]);

  const state = !status ? "waiting" : isPreview ? "preview" : status.running ? "live" : "idle";
  const cableReady = snapshot ? snapshot.strips.slice(5).some((strip) => strip.device) : false;

  return (
    <div className="sp">
      <header className="sp__bar">
        <div className="sp__brand">
          <span className="sp__mark" aria-hidden="true">
            <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
              <path d="M3 2.5v11M8 2.5v11M13 2.5v11" />
              <circle cx="3" cy="6" r="1.6" fill="currentColor" stroke="none" />
              <circle cx="8" cy="10" r="1.6" fill="currentColor" stroke="none" />
              <circle cx="13" cy="4.5" r="1.6" fill="currentColor" stroke="none" />
            </svg>
          </span>
          <div className="sp__titles">
            <h1 className="sp__name">Spud</h1>
          </div>
        </div>

        <span className={`sp__pill sp__pill--${state}`}>
          <i className="sp__dot" />
          {state === "live"
            ? `${status?.inputs} in, ${status?.outputs} out`
            : state === "preview"
              ? "preview data"
              : state === "idle"
                ? "no streams open"
                : "starting the audio thread"}
        </span>

        {/* A bus whose device would not open is drawn exactly like one that did,
            so the header carries the count. The messages themselves are long and
            belong where the device is changed, not over the console. */}
        {state === "live" && status?.errors?.length ? (
          <button
            type="button"
            className="sp__pill sp__pill--failed"
            title={status.errors.join("\n")}
            onClick={() => setSettingsOpen(true)}
          >
            {status.errors.length} device{status.errors.length === 1 ? "" : "s"} would not open
          </button>
        ) : null}

        <span className="sp__spacer" />

        <button
          type="button"
          className="sp__icon"
          onClick={() => setSettingsOpen(true)}
          aria-label="Open settings"
          title="Settings"
        >
          <Gear />
        </button>
      </header>

      {/* Only while a newer release is waiting: it is one row, and it goes away
          again when the person chooses Later. */}
      <UpdateBanner />

      <main className="sp__body">
        {snapshot ? (
          <Console
            snapshot={snapshot}
            write={write}
            writeText={writeText}
            pickDevice={(index) => setPicking({ kind: "bus", index })}
            pickInput={(index) => setPicking({ kind: "strip", index })}
            editEq={setEqStrip}
            editDynamics={setDynStrip}
            apps={apps}
            setApp={setApp}
          />
        ) : (
          <div className="sp__empty">
            <h2 className="sp__empty-title">Waiting for the mixer</h2>
            <p className="sp__empty-note">
              The audio thread starts with the window and reports what it opens.
              A device you plug in later is picked up without a rescan.
            </p>
          </div>
        )}
      </main>

      <footer className="sp__foot">
        <span className="sp__foot-item">
          {isPreview ? "in-memory stand-in, no audio is touched" : "mixing in this process"}
          {snapshot
            ? `, ${(snapshot.sample_rate / 1000).toFixed(1)} kHz, ${Math.round(snapshot.latency)} ms buffer`
            : ""}
        </span>
        <span className="sp__spacer" />
        {restarting ? (
          <span className="sp__foot-item sp__foot-item--busy">reopening streams...</span>
        ) : null}
      </footer>

      <DeviceDialog target={target} onClose={() => setPicking(null)} onPick={choose} />

      <MicEqDialog
        strip={eqStrip === null ? null : (snapshot?.strips[eqStrip] ?? null)}
        write={write}
        onClose={() => setEqStrip(null)}
      />

      <DynamicsDialog
        strip={dynStrip === null ? null : (snapshot?.strips[dynStrip] ?? null)}
        write={write}
        onClose={() => setDynStrip(null)}
      />

      <SettingsDialog
        open={settingsOpen}
        status={status}
        preview={isPreview}
        cableReady={cableReady}
        pairs={pairs}
          startup={startup}
          onStartup={(enabled) => void toggleStartup(enabled)}
          onChooseExe={chooseStartupExe}
          onUseThisCopy={() => void pointStartupAt(startup.running ?? "")}
        onClose={() => setSettingsOpen(false)}
        onAppearance={() => {
          setSettingsOpen(false);
          setAppearanceOpen(true);
        }}
        onRescan={() => void rescan()}
      />

      <AppearanceDialog open={appearanceOpen} onClose={() => setAppearanceOpen(false)} />
    </div>
  );
}
