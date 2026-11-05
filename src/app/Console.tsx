import { createContext, useContext, useEffect, useRef, useState } from "react";
import { Fader } from "./Fader";
import { Knob } from "./Knob";
import { Meter } from "./Meter";
import {
  ROUTES,
  busParam,
  dynamicsLabel,
  loopsCable,
  dynamicsOn,
  formatDb,
  routeName,
  shortDevice,
  stripParam,
  stripTitle,
  type AppSession,
  type Bus,
  type Snapshot,
  type Strip,
  type VirtualInput,
} from "@/engine/model";

type ConsoleApi = {
  snapshot: Snapshot;
  write: (name: string, value: number) => void;
  writeText: (name: string, value: string) => void;
  /** Open the device picker for a bus output. */
  pickDevice: (bus: number) => void;
  /** Open the device picker for a strip input. */
  pickInput: (strip: number) => void;
  /** Open the full equaliser for one input. */
  editEq: (strip: number) => void;
  /** Open the gate and compressor for one input. */
  editDynamics: (strip: number) => void;
  /** Programs playing into each virtual input, from Windows audio sessions. */
  apps: VirtualInput[];
  setApp: (pid: number, patch: { volume?: number; mute?: boolean }) => void;
};

const ConsoleContext = createContext<ConsoleApi | null>(null);

export const useConsole = () => {
  const value = useContext(ConsoleContext);
  if (!value) throw new Error("useConsole outside <Console>");
  return value;
};

type ConsoleProps = ConsoleApi;

/**
 * The console proper: five hardware inputs, three virtual inputs, eight buses.
 *
 * The split is not decoration. A hardware input is a device you choose; a
 * virtual input is a cable other programs play into, so it carries the
 * equaliser and the eight-channel position switches that decide what leaves it.
 */
export function Console({ snapshot, ...api }: ConsoleProps) {
  const hardware = snapshot.strips.filter((strip) => !strip.virtual_input);
  const virtual = snapshot.strips.filter((strip) => strip.virtual_input);
  // A struck route says what it cannot do; this one line says why, and only
  // appears when such a chip is actually on screen.
  const struck = snapshot.strips.some((strip) =>
    ROUTES.some((_, position) => loopsCable(strip, snapshot.buses, position)),
  );

  return (
    <ConsoleContext.Provider value={{ snapshot, ...api }}>
      <div className="console">
        <div className="console__inputs">
          <div className="console__heads" aria-hidden="true">
            <span className="section-head" style={{ gridColumn: `span ${hardware.length}` }}>
              Hardware inputs
            </span>
            <span
              className="section-head section-head--virtual"
              style={{ gridColumn: `span ${virtual.length}` }}
            >
              Virtual inputs
            </span>
          </div>
          <div className="console__strips">
            {hardware.map((strip) => (
              <HardwareColumn key={strip.index} strip={strip} />
            ))}
            {virtual.map((strip) => (
              <VirtualColumn key={strip.index} strip={strip} />
            ))}
          </div>
        </div>
        <section className="console__buses" aria-label="Output buses">
          {/* The same label row the inputs carry: without it the bus keys sat a row
              higher than every strip, and the one block every route points at was
              the only group with no name. */}
          <div className="console__bus-head" aria-hidden="true">
            <span className="section-head">Output buses</span>
            {struck ? (
              <span className="section-note">struck route = the input&rsquo;s own cable</span>
            ) : null}
          </div>
          <div className="console__bus-panel">
            {snapshot.buses.map((bus) => (
              <BusColumn key={bus.index} bus={bus} />
            ))}
          </div>
        </section>
      </div>
    </ConsoleContext.Provider>
  );
}

/** Label editing, shared by both kinds of input. */
function StripHeader({ strip }: { strip: Strip }) {
  const { writeText, pickInput } = useConsole();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(strip.label);

  useEffect(() => {
    if (!editing) setDraft(strip.label);
  }, [strip.label, editing]);

  const commit = () => {
    setEditing(false);
    if (draft !== strip.label) writeText(stripParam(strip.index, "Label"), draft);
  };

  return (
    <header className="strip__head">
      {editing ? (
        <input
          className="strip__label-input"
          value={draft}
          autoFocus
          aria-label={`Rename ${stripTitle(strip)}`}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
            if (event.key === "Escape") {
              setDraft(strip.label);
              setEditing(false);
            }
          }}
        />
      ) : (
        <button
          type="button"
          className="strip__label"
          title="Click to rename this input"
          onClick={() => {
            setDraft(strip.label);
            setEditing(true);
          }}
        >
          {stripTitle(strip)}
        </button>
      )}
      <button
        type="button"
        className="strip__device"
        title={strip.device || "No device assigned. Click to choose one"}
        onClick={() => pickInput(strip.index)}
      >
        {strip.device || (strip.virtual_input ? "no cable" : "choose device")}
      </button>
    </header>
  );
}

/**
 * The routing buttons, one per bus.
 *
 * A virtual input captures a cable, and the matching B bus renders into that
 * same cable. Sending it there would be a loop through Windows: the bus would
 * hear itself, so that one button is off limits. The engine refuses it too;
 * this is the explanation rather than the enforcement.
 */
function Routing({ strip }: { strip: Strip }) {
  const { write, snapshot } = useConsole();
  return (
    <div className="strip__routes" role="group" aria-label={`${stripTitle(strip)} routing`}>
      {ROUTES.map((route, position) => {
        const looped = loopsCable(strip, snapshot.buses, position);
        return (
          <button
            key={route}
            type="button"
            className={`route${position >= 5 ? " route--virtual" : ""}`}
            aria-pressed={strip.routes[position]}
            aria-label={`Send ${stripTitle(strip)} to ${route}`}
            title={
              looped
                ? `${route} renders into the cable this input captures, so sending audio there would loop`
                : undefined
            }
            disabled={looped}
            onClick={() => write(routeName(strip.index, route), strip.routes[position] ? 0 : 1)}
          >
            {route}
          </button>
        );
      })}
    </div>
  );
}

function FaderBlock({ strip }: { strip: Strip }) {
  const { write } = useConsole();
  return (
    <>
      <div className="strip__body">
        <Meter kind="strip" index={strip.index} />
        <Fader
          label={`${stripTitle(strip)} gain`}
          value={strip.gain}
          onChange={(db) => write(stripParam(strip.index, "Gain"), db)}
        />
      </div>
      <div className="strip__readout">
        <span className="strip__db">{formatDb(strip.gain)}</span>
        <span className="strip__unit">dB</span>
      </div>
    </>
  );
}

function MiniRow({ strip }: { strip: Strip }) {
  const { write } = useConsole();
  const toggle = (field: string, on: boolean) => write(stripParam(strip.index, field), on ? 0 : 1);

  return (
    <div className="strip__buttons">
      <button
        type="button"
        className="mini mini--mute"
        aria-pressed={strip.mute}
        aria-label={`Mute ${stripTitle(strip)}`}
        onClick={() => toggle("Mute", strip.mute)}
      >
        M
      </button>
      <button
        type="button"
        className="mini mini--solo"
        aria-pressed={strip.solo}
        aria-label={`Solo ${stripTitle(strip)}`}
        onClick={() => toggle("Solo", strip.solo)}
      >
        S
      </button>
      <button
        type="button"
        className="mini"
        aria-pressed={strip.mono}
        aria-label={`Mono ${stripTitle(strip)}`}
        onClick={() => toggle("Mono", strip.mono)}
      >
        Ø
      </button>
    </div>
  );
}

function HardwareColumn({ strip }: { strip: Strip }) {
  const { editEq, editDynamics } = useConsole();
  return (
    <article className={`strip${strip.mute ? " strip--muted" : ""}`} data-strip={strip.index}>
      <StripHeader strip={strip} />
      <FaderBlock strip={strip} />
      <MiniRow strip={strip} />
      <Routing strip={strip} />
      <div className="strip__fx">
        {/* The three knobs of a virtual input do not fit beside a microphone's
            fader, so both panels open from here: the equaliser (which is also
            where the low cut and the body band live) and the input level with
            the gate and the compressor, which share one dialog because they
            share the signal path. */}
        <DynamicsChips strip={strip} onEq={() => editEq(strip.index)} onDynamics={() => editDynamics(strip.index)} />
      </div>
    </article>
  );
}

/**
 * `EQ` and `DYN`, lit while the thing behind them is switched in.
 *
 * `onEq` is left off for a virtual input, whose equaliser is already on the
 * column as knobs and a switch, so there is no sense in offering it twice.
 */
function DynamicsChips({
  strip,
  onEq,
  onDynamics,
}: {
  strip: Strip;
  onEq?: () => void;
  onDynamics: () => void;
}) {
  const dynamics = dynamicsOn(strip);
  return (
    <>
      {onEq ? (
        <button
          type="button"
          className="chip"
          data-on={strip.eq ? "true" : undefined}
          aria-pressed={strip.eq}
          aria-label={`Equaliser for ${stripTitle(strip)}`}
          title={`${stripTitle(strip)} equaliser, ${strip.eq ? "on" : "off"}`}
          onClick={onEq}
        >
          EQ
        </button>
      ) : null}        <button
          type="button"
          className="chip"
          data-on={dynamics ? "true" : undefined}
        aria-pressed={dynamics}
        aria-label={`Input level, gate and compressor for ${stripTitle(strip)}`}
        title={`${stripTitle(strip)}: ${dynamicsLabel(strip)}, input ${formatDb(strip.trim)} dB`}
        onClick={onDynamics}
      >
        DYN
      </button>
    </>
  );
}

/**
 * A virtual input: the equaliser above the same fader and routing as any other
 * input.
 *
 * The three knobs are the mixer's three bands, in band order (treble, mid,
 * bass), and they only move audio while the EQ chip is lit.
 */
function VirtualColumn({ strip }: { strip: Strip }) {
  const { write, snapshot, apps, editDynamics } = useConsole();
  const playing = apps.find((entry) => entry.strip === strip.index)?.apps ?? [];
  const rate = snapshot.sample_rate ? `${(snapshot.sample_rate / 1000).toFixed(1)} kHz` : "no rate";
  // The window between the cable and the speakers, which is the number that
  // matters when a video is a fraction of a second out of step with its sound.
  const lag = snapshot.latency ? `${Math.round(snapshot.latency)} ms` : "no buffer";

  return (
    <article
      className={`strip strip--virtual${strip.mute ? " strip--muted" : ""}`}
      data-strip={strip.index}
    >
      <StripHeader strip={strip} />
      <span
        className="strip__rate"
        title={`Mixed at ${rate}; the console holds ${lag} of buffer, and the devices add their own periods on top`}
      >
        {rate}, {lag}
      </span>

      <section className="strip__eq" aria-label={`${stripTitle(strip)} equaliser`}>
        <h3 className="strip__caption">
          Equaliser
          <button
            type="button"
            className="chip chip--tiny"
            data-on={strip.eq ? "true" : undefined}
            aria-pressed={strip.eq}
            aria-label={`Equaliser ${strip.eq ? "on" : "off"} for ${stripTitle(strip)}`}
            onClick={() => write(stripParam(strip.index, "EQ.on"), strip.eq ? 0 : 1)}
          >
            {strip.eq ? "on" : "off"}
          </button>
        </h3>
        <div className="strip__knobs">
          <Knob
            label="Treble"
            value={strip.eq_gain[0]}
            min={-12}
            max={12}
            onChange={(value) => write(stripParam(strip.index, "EQGain1"), value)}
          />
          <Knob
            label="Mid"
            value={strip.eq_gain[1]}
            min={-12}
            max={12}
            onChange={(value) => write(stripParam(strip.index, "EQGain2"), value)}
          />
          <Knob
            label="Bass"
            value={strip.eq_gain[2]}
            min={-12}
            max={12}
            onChange={(value) => write(stripParam(strip.index, "EQGain3"), value)}
          />
        </div>
      </section>

      <AppList apps={playing} />

      <FaderBlock strip={strip} />
      <MiniRow strip={strip} />
      <Routing strip={strip} />
      <div className="strip__fx">
        {/* A virtual input carries another program's audio, so its chain starts
            switched out, but a game or a browser on a cable is exactly what a
            gate and a compressor are for, so the chip is here. */}
        <DynamicsChips strip={strip} onDynamics={() => editDynamics(strip.index)} />
      </div>
    </article>
  );
}

/**
 * The programs Windows reports as playing into this input.
 *
 * There is no bookkeeping to do: a row is drawn because the session exists, and
 * when the program quits its session ends and the row disappears on the next
 * poll. Nothing is remembered, so nothing can go stale.
 */
function AppList({ apps }: { apps: AppSession[] }) {
  if (apps.length === 0) return null;

  return (
    <section className="strip__apps" aria-label="Applications on this input">
      <h3 className="strip__caption">
        Apps
        <span className="strip__apps-count">{apps.length}</span>
      </h3>
      {apps.slice(0, 5).map((app) => (
        <AppRow key={`${app.pid}-${app.name}`} app={app} />
      ))}
    </section>
  );
}

function AppRow({ app }: { app: AppSession }) {
  const { setApp } = useConsole();
  const [volume, setVolume] = useState(app.volume);
  const holding = useRef(false);

  useEffect(() => {
    if (!holding.current) setVolume(app.volume);
  }, [app.volume]);

  return (
    <div className="app-row" data-playing={app.playing ? "true" : undefined} data-muted={app.mute ? "true" : undefined}>
      <span className="app-row__dot" aria-hidden="true" />
      <span className="app-row__name" title={`${app.name}, pid ${app.pid}`}>
        {app.name}
      </span>
      <button
        type="button"
        className="app-row__mute"
        aria-pressed={app.mute}
        aria-label={`Mute ${app.name}`}
        onClick={() => setApp(app.pid, { mute: !app.mute })}
      >
        M
      </button>
      <input
        className="app-row__vol"
        type="range"
        min={0}
        max={1}
        step={0.01}
        value={volume}
        aria-label={`${app.name} volume`}
        onPointerDown={() => {
          holding.current = true;
        }}
        onPointerUp={() => {
          holding.current = false;
        }}
        onBlur={() => {
          holding.current = false;
        }}
        onChange={(event) => {
          const next = Number(event.target.value);
          holding.current = true;
          setVolume(next);
          setApp(app.pid, { volume: next });
        }}
      />
    </div>
  );
}

function BusColumn({ bus }: { bus: Bus }) {
  const { write, pickDevice } = useConsole();
  const toggle = (name: string, on: boolean) => write(name, on ? 0 : 1);

  return (
    <article className={`bus${bus.mute ? " bus--muted" : ""}`} data-bus={bus.index}>
      <span
        className={`bus__key${bus.physical ? "" : " bus__key--virtual"}`}
        title={
          bus.physical
            ? "A hardware output: speakers, headphones, an interface"
            : "Renders into that cable's Input side, so another program can record it from the cable's Output. A microphone, as far as it knows"
        }
      >
        {bus.key}
      </span>
      <div className="bus__body">
        <Meter kind="bus" index={bus.index} />
        <Fader
          label={`Bus ${bus.key} gain`}
          value={bus.gain}
          onChange={(db) => write(busParam(bus.index, "Gain"), db)}
        />
      </div>
      <span className="bus__db">{formatDb(bus.gain)}</span>
      <button
        type="button"
        className="bus__device"
        title={bus.device || "No output assigned. Click to choose one"}
        onClick={() => pickDevice(bus.index)}
      >
        {/* The column is 44 px wide: the vendor suffix is what made the name
            wrap into three fragments, and `CABLE Input` beside `CABLE-A Input`
            is no help at this size. A cable shows which cable it is. */}
        {shortDevice(bus.device) || (bus.physical ? "assign" : "no cable")}
      </button>
      <div className="bus__buttons">
        <button
          type="button"
          className="mini mini--mute"
          aria-pressed={bus.mute}
          aria-label={`Mute bus ${bus.key}`}
          onClick={() => toggle(busParam(bus.index, "Mute"), bus.mute)}
        >
          M
        </button>
        <button
          type="button"
          className="mini"
          aria-pressed={bus.mono}
          aria-label={`Mono bus ${bus.key}`}
          onClick={() => toggle(busParam(bus.index, "Mono"), bus.mono)}
        >
          Ø
        </button>
        {/* The ceiling, on the bus rather than on the strips: what leaves here
            is what another program records, so one boosted input is enough to
            clip somebody else's capture. */}
        <button
          type="button"
          className="mini mini--limit"
          aria-pressed={bus.limit}
          aria-label={`Limiter ${bus.limit ? "on" : "off"} for bus ${bus.key}`}
          title={
            bus.limit
              ? `Holds ${bus.key} just under full scale, so a strip raised by EQ or makeup gain cannot arrive clipped`
              : `${bus.key} is unlimited, so a boosted strip can reach the device over full scale and be clipped there`
          }
          onClick={() => toggle(busParam(bus.index, "Limit"), bus.limit)}
        >
          LIM
        </button>
      </div>
    </article>
  );
}
