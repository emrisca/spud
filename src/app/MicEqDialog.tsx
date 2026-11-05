import { Modal } from "@/components/interior/modal";
import { PressDepth } from "@/components/interior/press-depth";
import { formatDb, stripParam, stripTitle, type Strip } from "@/engine/model";

/**
 * The bands, in the order a voice is shaped: what comes out of the bottom, what
 * makes a close microphone sound boxy, then where consonants live.
 *
 * The field names are the engine's; the indices are the snapshot's, which is
 * treble, mid, bass, body.
 */
const BANDS = [
  { field: "EQGain3", index: 2, label: "Bass", hz: "120 Hz" },
  { field: "EQGain4", index: 3, label: "Body", hz: "350 Hz" },
  { field: "EQGain2", index: 1, label: "Mid", hz: "1 kHz" },
  { field: "EQGain1", index: 0, label: "Treble", hz: "6 kHz" },
] as const;

/** Low-cut corners. 90 Hz is where a talking voice starts. */
const CUTS = [0, 60, 90, 120, 150] as const;

/** The starting point for a microphone, and the engine's own seed. */
const VOICE = { cut: 90, bands: [2, 1.5, -2, -3] as const };
const FLAT = { cut: 0, bands: [0, 0, 0, 0] as const };

type Props = {
  strip: Strip | null;
  write: (name: string, value: number) => void;
  onClose: () => void;
};

/**
 * One input's equaliser, in full.
 *
 * A hardware column has no room for knobs (its fader is the tall thing), so the
 * EQ chip opens this instead. It is the only place the low cut and the body band
 * appear, because they are the two settings that make a microphone sound like a
 * microphone rather than a room.
 */
export function MicEqDialog({ strip, write, onClose }: Props) {
  const preset = (cut: number, bands: readonly number[]) => {
    if (!strip) return;
    write(stripParam(strip.index, "EQCut"), cut);
    for (const band of BANDS) {
      write(stripParam(strip.index, band.field), bands[band.index]);
    }
    if (!strip.eq) write(stripParam(strip.index, "EQ.on"), 1);
  };

  const voice =
    strip !== null &&
    strip.eq_cut === VOICE.cut &&
    BANDS.every((band) => strip.eq_gain[band.index] === VOICE.bands[band.index]);

  return (
    <Modal
      open={strip !== null}
      onClose={onClose}
      title={strip ? `Mic EQ, ${stripTitle(strip)}` : "Mic EQ"}
      description="Shapes this input on its way to every bus it is routed to."
      maxWidth={470}
      maxHeight="min(86vh, 720px)"
      footer={
        <div className="sp__footer-row">
          <span className="eq-hint">{voice ? "voice preset" : "custom"}</span>
          <span style={{ flex: 1 }} />
          <PressDepth depth={3} tilt={5} onClick={onClose}>
            Done
          </PressDepth>
        </div>
      }
    >
      {strip ? (
        <div className="sp__stack">
          <button
            type="button"
            className="sp__row sp__row--switch"
            role="switch"
            aria-checked={strip.eq}
            onClick={() => write(stripParam(strip.index, "EQ.on"), strip.eq ? 0 : 1)}
          >
            <span className="sp__row-body">
              <span className="sp__row-name">Equaliser</span>
              <span className="sp__row-note">
                {strip.eq
                  ? "On: the bands below are applied before the fader."
                  : "Off: the input passes through untouched, settings and all."}
              </span>
            </span>
            <span className="sp__switch" data-on={strip.eq ? "true" : undefined} aria-hidden="true" />
          </button>

          <div className="sp__row" data-dim={strip.eq ? undefined : "true"}>
            <span className="sp__row-name">Low cut</span>
            <span className="sp__row-note">
              A 12 dB/octave cut under the voice: rumble, desk thumps, the boom of
              a close microphone.
            </span>
            <div className="eq-chips" role="group" aria-label="Low cut frequency">
              {CUTS.map((hz) => (
                <button
                  key={hz}
                  type="button"
                  className="chip"
                  data-on={strip.eq_cut === hz ? "true" : undefined}
                  aria-pressed={strip.eq_cut === hz}
                  onClick={() => write(stripParam(strip.index, "EQCut"), hz)}
                >
                  {hz === 0 ? "Off" : `${hz} Hz`}
                </button>
              ))}
            </div>
          </div>

          <div className="eq-bands" data-dim={strip.eq ? undefined : "true"}>
            {BANDS.map((band) => (
              <Band
                key={band.field}
                label={band.label}
                hz={band.hz}
                value={strip.eq_gain[band.index] ?? 0}
                onChange={(value) => write(stripParam(strip.index, band.field), value)}
              />
            ))}
          </div>

          <div className="eq-presets">
            <span className="sp__row-note">Start from</span>
            <button
              type="button"
              className="chip"
              data-on={voice ? "true" : undefined}
              onClick={() => preset(VOICE.cut, VOICE.bands)}
            >
              Voice
            </button>
            <button
              type="button"
              className="chip"
              onClick={() => preset(FLAT.cut, FLAT.bands)}
            >
              Flat
            </button>
          </div>

          <p className="sp__fineprint">
            Voice is where a microphone starts. Flat hands the input back exactly
            as the device delivers it.
          </p>
        </div>
      ) : null}
    </Modal>
  );
}

function Band({
  label,
  hz,
  value,
  onChange,
}: {
  label: string;
  hz: string;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="param">
      <span className="param__head">
        <span className="param__label">
          {label}
          <span className="eq-hz">{hz}</span>
        </span>
        <span className="param__value">{formatDb(value)} dB</span>
      </span>
      <input
        className="param__range"
        type="range"
        min={-12}
        max={12}
        step={0.5}
        value={value}
        aria-label={`${label} ${hz} gain`}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </label>
  );
}
