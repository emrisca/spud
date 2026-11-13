import { useEffect, useState } from "react";
import { Modal } from "@/components/interior/modal";
import { PressDepth } from "@/components/interior/press-depth";
import { engine, isPreview, type StartupView, type UpdateView } from "@/engine/bridge";
import { cableKey, type DeviceDesc, type EngineStatus } from "@/engine/model";

/** Which control asked for a device: a bus output or a strip input. */
export type PickTarget = { kind: "bus" | "strip"; index: number };

type Resolved = PickTarget & { name: string; current: string };

type SettingsProps = {
  open: boolean;
  status: EngineStatus | null;
  preview: boolean;
  /** True when at least one virtual input found a cable to capture. */
  cableReady: boolean;
  /** Which cable each virtual input captures, and the bus that feeds it. */
  pairs: { strip: string; bus: string; cable: string }[];
  /** The sign-in entry: whether it exists, which copy it names, and this one. */
  startup: StartupView;
  onClose: () => void;
  onAppearance: () => void;
  onRescan: () => void;
  onStartup: (enabled: boolean) => void;
  onChooseExe: () => void;
  onUseThisCopy: () => void;
};

/**
 * Settings: what the engine has open, what it starts, and how it looks.
 *
 * Everything else the console could explain lives where the question is asked.
 * The microphone note is in the output picker, where somebody looks for their
 * microphone; the tray behaviour is one line at the bottom. A settings window
 * that repeats the README is a settings window nobody reads.
 */
export function SettingsDialog({
  open,
  status,
  preview,
  cableReady,
  pairs,
  startup,
  onClose,
  onAppearance,
  onRescan,
  onStartup,
  onChooseExe,
  onUseThisCopy,
}: SettingsProps) {
  // The entry can name a different copy than the one running: an installed
  // Spud while a build is open, or a path that moved. Saying which executable
  // Windows will actually start is the difference between a switch that looks
  // right and one that is.
  const aimedElsewhere =
    Boolean(startup.entry) &&
    Boolean(startup.running) &&
    (startup.entry ?? "").toLowerCase() !== (startup.running ?? "").toLowerCase();

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Settings"
      description="What the mixer has open, where it sends it, and how it looks."
      maxWidth={470}
      footer={
        <div className="sp__footer-row">
          <span style={{ flex: 1 }} />
          <PressDepth depth={3} tilt={5} onClick={onClose}>
            Done
          </PressDepth>
        </div>
      }
    >
      <div className="sp__stack">
        <div className="sp__row">
          <span className="sp__row-name">Engine</span>
          <span className="sp__row-note">
            {status
              ? status.running
                ? `${status.inputs} inputs and ${status.outputs} outputs open${preview ? ", preview data" : ""}`
                : `No streams open${preview ? ", preview data" : ""}`
              : "Starting"}
          </span>
          {status?.errors?.length ? (
            <ul className="sp__errors">
              {status.errors.map((error) => (
                <li key={error}>{error}</li>
              ))}
            </ul>
          ) : null}
        </div>

        <Updates />

        <button type="button" className="sp__row sp__row--button" onClick={onRescan}>
          <span className="sp__row-name">Rescan devices</span>
          <span className="sp__row-note">
            Reopens every stream and fills empty inputs and outputs. A device you plug
            in or remove also reopens the streams by itself within a few seconds.
          </span>
        </button>

        <button type="button" className="sp__row sp__row--button" onClick={onAppearance}>
          <span className="sp__row-name">Appearance</span>
          <span className="sp__row-note">Mode, presets, hue and saturation</span>
        </button>

        <button
          type="button"
          className="sp__row sp__row--switch"
          role="switch"
          aria-checked={startup.enabled}
          onClick={() => onStartup(!startup.enabled)}
        >
          <span className="sp__row-body">
            <span className="sp__row-name">Launch at sign-in</span>
            <span className="sp__row-note">
              {startup.enabled
                ? aimedElsewhere
                  ? "Windows starts the copy below when you sign in. That is not the copy open now."
                  : "Windows starts this copy when you sign in, hidden in the notification area."
                : "Spud starts only when you open it."}
            </span>
            {startup.enabled && startup.entry ? (
              <span className="sp__path" title={startup.entry}>
                {startup.entry}
              </span>
            ) : null}
          </span>
          <span
            className="sp__switch"
            data-on={startup.enabled ? "true" : undefined}
            aria-hidden="true"
          />
        </button>

        {startup.enabled ? (
          <div className="sp__row sp__row--links">
            <button type="button" className="sp__link sp__link--button" onClick={onChooseExe}>
              Choose another copy...
            </button>
            {aimedElsewhere ? (
              <button type="button" className="sp__link sp__link--button" onClick={onUseThisCopy}>
                Use this copy
              </button>
            ) : null}
          </div>
        ) : null}

        <div className="sp__row">
          <span className="sp__row-name">Virtual inputs</span>
          <span className="sp__row-note">
            {cableReady
              ? "One cable per virtual input. Send a program to a cable's Input side in Windows sound settings and it lands on that strip."
              : "No cable found. A virtual input needs a signed virtual cable for programs to route audio into it."}
          </span>
          {cableReady ? null : (
            <a
              className="sp__link"
              href="https://vb-audio.com/Cable/"
              target="_blank"
              rel="noreferrer"
            >
              Get VB-CABLE (free, and the A+B pack fills all three)
            </a>
          )}
          {pairs.length ? (
            <ul className="sp__pairs">
              {pairs.map((pair) => (
                <li key={pair.bus}>
                  {pair.strip} listens to <span className="sp__pairs-cable">{pair.cable}</span>;
                  {" "}
                  {pair.bus} plays into it.
                </li>
              ))}
            </ul>
          ) : null}
        </div>

        <p className="sp__fineprint">
          Closing the window hides Spud to the notification area and the mix keeps
          running. Left-click the tray icon to bring it back; right-click it to quit.
        </p>
      </div>
    </Modal>
  );
}

type DeviceProps = {
  target: Resolved | null;
  onClose: () => void;
  onPick: (kind: PickTarget["kind"], index: number, name: string) => void;
};

export function DeviceDialog({ target, onClose, onPick }: DeviceProps) {
  const [devices, setDevices] = useState<DeviceDesc[]>([]);
  const [loading, setLoading] = useState(false);
  const kind = target?.kind;
  const index = target?.index;

  useEffect(() => {
    if (kind === undefined) return;
    let live = true;
    setLoading(true);
    engine
      .devices()
      .then(([outputs, inputs]) => live && setDevices(kind === "bus" ? outputs : inputs))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [kind, index]);

  return (
    <Modal
      open={target !== null}
      onClose={onClose}
      title={target?.name ?? "Device"}
      description={
        kind === "bus"
          ? "Windows' playback devices, because a bus renders into one. Picking one reopens the streams, so audio pauses for a moment."
          : "Windows' recording devices, because an input captures from one. Picking one reopens the streams, so audio pauses for a moment."
      }
      maxWidth={540}
      maxHeight="min(80vh, 640px)"
    >
      {/* A list takes a shape while it loads, not a sentence. Four rows is what
          fits without the dialog jumping when the real list arrives. */}
      {loading ? (
        <div className="sp__skeletons" aria-hidden="true">
          {[0, 1, 2, 3].map((row) => (
            <div className="sp__skeleton" key={row}>
              <span className="sp__skeleton-name" />
              <span className="sp__skeleton-hw" />
            </div>
          ))}
        </div>
      ) : null}
      {/* A microphone is a recording device, and Windows will not let an
          application render into one (the audio engine answers
          AUDCLNT_E_WRONG_ENDPOINT_TYPE), so no amount of looking will find a
          microphone in the list below. Saying so here is the only way the list
          stops looking broken. */}
      {kind === "bus" ? (
        <p className="sp__fineprint">
          Your microphone will not appear here. It is a recording device, and Windows
          does not allow an application to write into one. To send audio back to a
          microphone, route the input to B1, B2 or B3, then pick that cable&rsquo;s Output
          in the program that needs it. Setting that Output as Windows&rsquo; default
          recording device covers every program at once.
        </p>
      ) : null}
      <div className="sp__stack">
        {/* Unassigning is a real choice: a bus with no device is silent, and an
            input with no device captures nothing. Without this row the only way
            back was to pick a different device instead. */}
        <button
          type="button"
          className="sp__device sp__device--none"
          aria-current={target?.current === ""}
          onClick={() => target && onPick(target.kind, target.index, "")}
        >
          <span className="sp__device-name">None</span>
          <span className="sp__device-hw">
            leave this {kind === "bus" ? "output" : "input"} unassigned
          </span>
          {target?.current === "" ? <span className="dot" /> : null}
        </button>
        {devices.map((device) => {
          const current = target?.current === device.name;
          return (
            <button
              key={`${device.index}-${device.name}`}
              type="button"
              className="sp__device"
              aria-current={current}
              onClick={() => target && onPick(target.kind, target.index, device.name)}
            >
              <span className="sp__device-name">{device.name}</span>
              {/* A cable has two sides and this list holds exactly one of them:
                  its Input side for a bus, its Output side for an input. The tag
                  is what distinguishes a cable from a sound card's port, since
                  Windows gives them no other clue. */}
              <span className="sp__device-hw">
                {cableKey(device.name) ? "virtual cable" : device.hardware}
              </span>
              {current ? <span className="dot" /> : null}
            </button>
          );
        })}
        {!loading && devices.length === 0 ? (
          <p className="sp__fineprint">
            Windows reported no devices for this direction. Check that the audio service
            is running, then rescan from settings.
          </p>
        ) : null}
      </div>
    </Modal>
  );
}

/**
 * The version, and the same check the shell runs on its own clock, on demand.
 *
 * The shell asks GitHub when Spud starts and every six hours after that, and the
 * banner under the header is where a release it found gets acted on. This row is
 * for the person who would rather not wait, and for the answer to be visible when
 * it arrives: what is installed, whether anything newer exists, and the first
 * lines of its notes. An install restarts Spud, so the button is one way only.
 */
function Updates() {
  const [state, setState] = useState<UpdateView | null>(null);
  const [checking, setChecking] = useState(false);
  const native = !isPreview;

  // Read even in the browser preview: the engine hands back the version this
  // build is, so the row names a version instead of standing there blank. Only
  // the check and the install need the shell.
  useEffect(() => {
    let live = true;
    engine
      .updateState()
      .then((next) => {
        if (live) setState(next);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);

  const check = async () => {
    setChecking(true);
    try {
      setState(await engine.updateCheck());
    } catch (reason) {
      setState((current) =>
        current ? { ...current, status: "error", error: String(reason) } : current,
      );
    } finally {
      setChecking(false);
    }
  };

  const working =
    state?.status === "downloading" ||
    state?.status === "installing" ||
    state?.status === "restarting";
  const offered = state?.status === "available" && Boolean(state.version);
  const version = state?.currentVersion ? `Spud ${state.currentVersion}` : "Spud";

  const note = !native
    ? "Updates install in the desktop app, not in a browser preview."
    : working
      ? state?.status === "downloading"
        ? `Downloading ${megabytes(state.downloaded)}${state.total ? ` of ${megabytes(state.total)}` : ""}. Spud restarts when it lands.`
        : "Installing. Spud restarts into the new build."
      : offered
        ? `Version ${state?.version} is available. Installing replaces this copy and restarts it.`
        : state?.status === "error"
          ? (state.error ?? "The check did not finish.")
          : state?.status === "upToDate"
            ? "This is the newest release."
            : "Checked when Spud starts, and every six hours after that.";

  return (
    <div className="sp__row" role="group" aria-label="Updates">
      <span className="sp__row-name">{version}</span>
      <span className="sp__row-note">{note}</span>
      {offered && state?.notes ? (
        <span className="sp__update-notes">
          {state.notes.split("\n").slice(0, 4).join("\n")}
        </span>
      ) : null}
      <span className="sp__row-actions">
        {offered || working ? (
          <button
            type="button"
            className="sp__link sp__link--button"
            disabled={working}
            onClick={() => void engine.updateInstall().catch(() => undefined)}
          >
            {working ? "Installing..." : "Update now"}
          </button>
        ) : (
          <button
            type="button"
            className="sp__link sp__link--button"
            disabled={!native || checking}
            onClick={() => void check()}
          >
            {checking ? "Checking..." : "Check for updates"}
          </button>
        )}
      </span>
    </div>
  );
}

/** A byte count as the update row writes one. */
const megabytes = (bytes: number): string => `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
