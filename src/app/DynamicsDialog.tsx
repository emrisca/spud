import { Modal } from "@/components/interior/modal";
import { PressDepth } from "@/components/interior/press-depth";
import { stripParam, stripTitle, type Strip } from "@/engine/model";

/**
 * The gate and the compressor, in the order they run.
 *
 * Both are wired to audio rather than to a picture of a control: the engine's
 * `mixer.rs` applies every one of these to the strip's samples before the fader.
 * The dialog opens on the strip's chip — there is no room for five knobs beside
 * a microphone's fader.
 */

/** The engine's own microphone defaults, mirrored from `lib.rs`. */
const VOICE_GATE = { threshold: -42, attack: 3, release: 200, hold: 220, damping: -50 };
const VOICE_COMP = {
  threshold: -18,
  ratio: 3,
  attack: 10,
  release: 150,
  knee: 6,
  gain_out: 3,
};

const db = (value: number) => `${value > 0 ? "+" : ""}${value.toFixed(0)} dB`;
/** The input trim moves in half decibels, so it reads to one place. */
const trimDb = (value: number) => `${value > 0 ? "+" : ""}${value.toFixed(1)} dB`;
const ms = (value: number) => `${value.toFixed(0)} ms`;
const ratio = (value: number) => `${value.toFixed(1)}:1`;

/** How far the gate pulls the signal down, phrased as what it does. */
const depth = (value: number) =>
  value >= -0.5 ? "nothing" : value <= -59.5 ? "silence" : `${value.toFixed(0)} dB`;

type Props = {
  strip: Strip | null;
  write: (name: string, value: number) => void;
  onClose: () => void;
};

export function DynamicsDialog({ strip, write, onClose }: Props) {
  const set = (field: string, value: number) => {
    if (strip) write(stripParam(strip.index, field), value);
  };

  const gate = strip?.gate_detail ?? null;
  const comp = strip?.comp_detail ?? null;
  const gateOn = (strip?.gate ?? 0) >= 0.5;
  const compOn = (strip?.comp ?? 0) >= 0.5;

  const onVoice =
    gate !== null &&
    comp !== null &&
    gateOn &&
    compOn &&
    gate.threshold === VOICE_GATE.threshold &&
    gate.attack === VOICE_GATE.attack &&
    gate.release === VOICE_GATE.release &&
    gate.hold === VOICE_GATE.hold &&
    gate.damping === VOICE_GATE.damping &&
    comp.threshold === VOICE_COMP.threshold &&
    comp.ratio === VOICE_COMP.ratio &&
    comp.attack === VOICE_COMP.attack &&
    comp.release === VOICE_COMP.release &&
    comp.knee === VOICE_COMP.knee &&
    comp.gain_out === VOICE_COMP.gain_out;

  const voice = () => {
    set("Gate", 1);
    set("Gate.Threshold", VOICE_GATE.threshold);
    set("Gate.Attack", VOICE_GATE.attack);
    set("Gate.Release", VOICE_GATE.release);
    set("Gate.Hold", VOICE_GATE.hold);
    set("Gate.Damping", VOICE_GATE.damping);
    set("Comp", 1);
    set("Comp.Threshold", VOICE_COMP.threshold);
    set("Comp.Ratio", VOICE_COMP.ratio);
    set("Comp.Attack", VOICE_COMP.attack);
    set("Comp.Release", VOICE_COMP.release);
    set("Comp.Knee", VOICE_COMP.knee);
    set("Comp.GainIn", 0);
    set("Comp.GainOut", VOICE_COMP.gain_out);
  };

  const off = () => {
    set("Gate", 0);
    set("Comp", 0);
  };

  return (
    <Modal
      open={strip !== null}
      onClose={onClose}
      title={strip ? `Input & Dynamics — ${stripTitle(strip)}` : "Input & Dynamics"}
      description="The level into this strip, then the gate and the compressor — all ahead of the fader, so they shape it on its way to every bus it feeds."
      maxWidth={500}
      maxHeight="min(86vh, 720px)"
      footer={
        <div className="sp__footer-row">
          <span className="eq-hint">{onVoice ? "voice chain" : gateOn || compOn ? "custom" : "off"}</span>
          <span style={{ flex: 1 }} />
          <PressDepth depth={3} tilt={5} onClick={onClose}>
            Done
          </PressDepth>
        </div>
      }
    >
      {strip && gate && comp ? (
        <div className="sp__stack">
          <div className="sp__row">
            <span className="sp__row-name">Input</span>
            <span className="sp__row-note">
              Level into this whole strip. The gate, the equaliser and the
              compressor all work at it, so a quiet microphone comes up here
              rather than by moving the gate&rsquo;s threshold down to meet it.
            </span>
            <Slider
              label="Level"
              note="ahead of everything else"
              value={strip.trim}
              min={-24}
              max={24}
              step={0.5}
              format={trimDb}
              onChange={(value) => set("Trim", value)}
            />
          </div>

          <button
            type="button"
            className="sp__row sp__row--switch"
            role="switch"
            aria-checked={gateOn}
            onClick={() => set("Gate", gateOn ? 0 : 1)}
          >
            <span className="sp__row-body">
              <span className="sp__row-name">Noise gate</span>
              <span className="sp__row-note">
                Shuts this input while nobody is talking. The hold keeps it open
                through the gaps between words, so speech stays whole.
              </span>
            </span>
            <span className="sp__switch" data-on={gateOn ? "true" : undefined} aria-hidden="true" />
          </button>

          <div className="dyn-grid" data-dim={gateOn ? undefined : "true"}>
            <Slider
              label="Threshold"
              note="below this, the input is shut"
              value={gate.threshold}
              min={-60}
              max={0}
              step={1}
              format={db}
              onChange={(value) => set("Gate.Threshold", value)}
            />
            <Slider
              label="Depth"
              note="how far it pulls down when shut"
              value={gate.damping}
              min={-60}
              max={0}
              step={1}
              format={depth}
              onChange={(value) => set("Gate.Damping", value)}
            />
            <Slider
              label="Attack"
              note="how fast it opens"
              value={gate.attack}
              min={0}
              max={200}
              step={1}
              format={ms}
              onChange={(value) => set("Gate.Attack", value)}
            />
            <Slider
              label="Hold"
              note="how long it stays open"
              value={gate.hold}
              min={0}
              max={2000}
              step={10}
              format={ms}
              onChange={(value) => set("Gate.Hold", value)}
            />
            <Slider
              label="Release"
              note="how slowly it closes"
              value={gate.release}
              min={0}
              max={2000}
              step={10}
              format={ms}
              onChange={(value) => set("Gate.Release", value)}
            />
          </div>

          <button
            type="button"
            className="sp__row sp__row--switch"
            role="switch"
            aria-checked={compOn}
            onClick={() => set("Comp", compOn ? 0 : 1)}
          >
            <span className="sp__row-body">
              <span className="sp__row-name">Compressor</span>
              <span className="sp__row-note">
                Pulls the loud parts back so the level stays even to whoever is
                listening. The makeup gain hands back what the ratio takes off.
              </span>
            </span>
            <span className="sp__switch" data-on={compOn ? "true" : undefined} aria-hidden="true" />
          </button>

          <div className="dyn-grid" data-dim={compOn ? undefined : "true"}>
            <Slider
              label="Threshold"
              note="where it starts working"
              value={comp.threshold}
              min={-60}
              max={0}
              step={1}
              format={db}
              onChange={(value) => set("Comp.Threshold", value)}
            />
            <Slider
              label="Ratio"
              note="how hard it holds the level"
              value={comp.ratio}
              min={1}
              max={20}
              step={0.1}
              format={ratio}
              onChange={(value) => set("Comp.Ratio", value)}
            />
            <Slider
              label="Attack"
              note="how fast it catches a peak"
              value={comp.attack}
              min={0}
              max={200}
              step={1}
              format={ms}
              onChange={(value) => set("Comp.Attack", value)}
            />
            <Slider
              label="Release"
              note="how fast it lets go"
              value={comp.release}
              min={0}
              max={2000}
              step={10}
              format={ms}
              onChange={(value) => set("Comp.Release", value)}
            />
            <Slider
              label="Knee"
              note="how gently it eases in"
              value={comp.knee}
              min={0}
              max={24}
              step={1}
              format={db}
              onChange={(value) => set("Comp.Knee", value)}
            />
            <Slider
              label="Makeup"
              note="level handed back after"
              value={comp.gain_out}
              min={-12}
              max={24}
              step={0.5}
              format={db}
              onChange={(value) => set("Comp.GainOut", value)}
            />
          </div>

          <div className="eq-presets">
            <span className="sp__row-note">Start from</span>
            <button
              type="button"
              className="chip"
              data-on={onVoice ? "true" : undefined}
              onClick={voice}
            >
              Voice
            </button>
            <button type="button" className="chip" onClick={off}>
              Off
            </button>
          </div>

          <p className="sp__fineprint">
            Voice is where a microphone starts: a gate at −42 dB with a 220 ms
            hold, and a gentle 3:1 over −18 dB with a 6 dB knee. Both run after
            the low cut, so rumble cannot hold the gate open. The input level is
            yours to set and is not part of the preset.
          </p>
        </div>
      ) : null}
    </Modal>
  );
}

function Slider({
  label,
  note,
  value,
  min,
  max,
  step,
  format,
  onChange,
}: {
  label: string;
  note: string;
  value: number;
  min: number;
  max: number;
  step: number;
  format: (value: number) => string;
  onChange: (value: number) => void;
}) {
  return (
    <label className="param">
      <span className="param__head">
        <span className="param__label">
          {label}
          <span className="eq-hz">{note}</span>
        </span>
        <span className="param__value">{format(value)}</span>
      </span>
      <input
        className="param__range"
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        aria-label={`${label} — ${note}`}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </label>
  );
}
