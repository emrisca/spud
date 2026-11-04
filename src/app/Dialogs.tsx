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
  // The entry can name a different copy than the one running — an installed
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
              ? `${status.edition} ${status.version} · ${status.running ? `${status.inputs} in, ${status.outputs} out` : "no streams open"}${preview ? " · preview data" : ""}`
              : "starting"}
          </span>
          {status?.errors?.length ? (
            <ul className="sp__errors">
              {status.errors.map((error) => (
                <li key={error}>{error}</li>
              ))}
            </ul>
          ) : null}
        </div>

        <button type="button" className="sp__row sp__row--button" onClick={onRescan}>
          <span className="sp__row-name">Rescan devices</span>
          <span className="sp__row-note">
            Reopens every stream and fills unassigned slots. Do this after plugging in an
            interface or installing the virtual cable.
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
                ? "Windows starts Spud when you sign in — hidden in the notification area, mixing straight away."
                : "Spud starts only when you open it."}
            </span>
          </span>
          <span
            className="sp__switch"
            data-on={startup.enabled ? "true" : undefined}
            aria-hidden="true"
          />
        </button>

        <div className="sp__row">
          <span className="sp__row-name">The copy it starts</span>
          <span className="sp__row-note">
            {startup.entry
              ? aimedElsewhere
                ? "Windows starts this one, not the Spud open now."
                : "The Spud that is open now — sign-in would start this copy."
              : "No entry yet. Switching it on above aims it at the copy that is running."}
          </span>
          {startup.entry ? (
            <span className="sp__path" title={startup.entry}>
              {startup.entry}
            </span>
          ) : null}
          <span className="sp__row-actions">
            <button type="button" className="sp__link sp__link--button" onClick={onChooseExe}>
              Choose another copy…
            </button>
            {aimedElsewhere ? (
              <button
                type="button"
                className="sp__link sp__link--button"
                onClick={onUseThisCopy}
              >
                Use this copy
              </button>
            ) : null}
          </span>
        </div>

        <div className="sp__row">
          <span className="sp__row-name">Virtual inputs</span>
          <span className="sp__row-note">
            {cableReady
              ? "One cable per virtual input. Send an app to a cable's Input side in Windows sound settings and it lands on that strip — a different app on a different cable gets its own strip."
              : "No cable found. Spud needs a free signed virtual cable for programs to route audio into it."}
          </span>
          {cableReady ? null : (
            <a
              className="sp__link"
              href="https://vb-audio.com/Cable/"
              target="_blank"
              rel="noreferrer"
            >
              Get VB-CABLE (free)
            </a>
          )}
          {cableReady ? null : (
            <a
              className="sp__link"
              href="https://vb-audio.com/Cable/"
              target="_blank"
              rel="noreferrer"
            >
              Add the CABLE-A and CABLE-B pack
            </a>
          )}
          {pairs.length ? (
            <ul className="sp__pairs">
              {pairs.map((pair) => (
                <li key={pair.bus}>
                  <span>{pair.strip}</span>
                  <span className="sp__pairs-arrow">→</span>
                  <span className="sp__pairs-cable">{pair.cable}</span>
                  <span className="sp__pairs-arrow">→</span>
                  <span>{pair.bus}</span>
                </li>
              ))}
            </ul>
          ) : null}
        </div>

        <div className="sp__row">
          <span className="sp__row-name">Mic out</span>
          <span className="sp__row-note">
            Send any input to B1–B3 and that bus plays into the cable. Another program then
            picks the cable's Output as its microphone — or set it as Windows' default
            recording device to cover every program at once. A cable cannot be sent back
            into itself, so that one routing button stays off.
          </span>
        </div>

        <div className="sp__row">
          <span className="sp__row-name">Closing the window</span>
          <span className="sp__row-note">
            The window hides to the notification area and the mix keeps running. Left-click
            the tray icon to bring it back; right-click it to quit.
          </span>
        </div>

        <Updates />

        <p className="sp__fineprint">
          Spud mixes in this process: it captures each input, sums it into every bus it is
          routed to, and renders each bus to a device. Nothing else has to be installed
          except the cable the virtual inputs use.
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
      {loading ? <p className="sp__fineprint">Reading device list…</p> : null}
      {/* A microphone is a recording device, and Windows will not let an
          application render into one — the audio engine answers
          AUDCLNT_E_WRONG_ENDPOINT_TYPE — so no amount of looking will find a
          microphone in the list below. Saying so here is the only way the list
          stops looking broken. */}
      {kind === "bus" ? (
        <p className="sp__fineprint">
          Your microphone will not appear here: it is a recording device, and Windows does
          not allow an application to write into one. To put audio back into a microphone,
          send the input to a cable on B1–B3 and point the program that wants it at that
          cable&rsquo;s Output — or set the cable as Windows&rsquo; default recording device and every
          program hears it at once.
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
            Windows reported no devices for this direction. Check that the audio service is
            running, then rescan from settings.
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

  useEffect(() => {
    if (!native) return;
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
  }, [native]);

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
            {working ? "Installing…" : "Update now"}
          </button>
        ) : (
          <button
            type="button"
            className="sp__link sp__link--button"
            disabled={!native || checking}
            onClick={() => void check()}
          >
            {checking ? "Checking…" : "Check for updates"}
          </button>
        )}
      </span>
    </div>
  );
}

/** A byte count as the update row writes one. */
const megabytes = (bytes: number): string => `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
