import { useCallback, useRef } from "react";
import { GAIN_MAX, GAIN_MIN, clamp, formatDb, gainToPos, posToGain } from "@/engine/model";

type FaderProps = {
  value: number;
  onChange: (db: number) => void;
  /** Announced to assistive tech; the visible name lives in the strip header. */
  label: string;
};

/** Ticks the eye needs: the top, unity, and a way down. */
const TICKS = [12, 0, -12, -24, -48];

/**
 * A console fader.
 *
 * Dragging writes continuously so the engine follows the hand; holding Shift
 * quarters the travel for fine work, and a double-click returns to unity. The
 * element carries the slider role itself so the keyboard drives it.
 */
export function Fader({ value, onChange, label }: FaderProps) {
  const track = useRef<HTMLDivElement>(null);
  const drag = useRef<{ startY: number; startDb: number; fine: boolean } | null>(null);

  const commit = useCallback(
    (db: number) => onChange(clamp(Math.round(db * 10) / 10, GAIN_MIN, GAIN_MAX)),
    [onChange],
  );

  const at = (clientY: number, fine: boolean) => {
    const box = track.current?.getBoundingClientRect();
    if (!box) return;
    if (fine && drag.current) {
      const travelled = (drag.current.startY - clientY) / box.height;
      commit(drag.current.startDb + travelled * (GAIN_MAX - GAIN_MIN));
      return;
    }
    commit(posToGain(1 - (clientY - box.top) / box.height));
  };

  const onPointerDown = (event: React.PointerEvent) => {
    // preventDefault keeps the drag from selecting text, but it also suppresses
    // the focus a click would normally give, so take it explicitly, or the
    // arrow keys stop working the moment the mouse has been used.
    event.preventDefault();
    (event.currentTarget as HTMLDivElement).focus();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { startY: event.clientY, startDb: value, fine: event.shiftKey };
    at(event.clientY, event.shiftKey);
  };

  const onPointerMove = (event: React.PointerEvent) => {
    if (!drag.current) return;
    at(event.clientY, event.shiftKey);
  };

  const onPointerUp = (event: React.PointerEvent) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    drag.current = null;
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    const step = event.shiftKey ? 0.1 : 0.5;
    const moves: Record<string, number> = {
      ArrowUp: step,
      ArrowRight: step,
      ArrowDown: -step,
      ArrowLeft: -step,
      PageUp: 3,
      PageDown: -3,
    };
    if (event.key in moves) {
      event.preventDefault();
      commit(value + moves[event.key]);
    } else if (event.key === "Home") {
      event.preventDefault();
      commit(GAIN_MIN);
    } else if (event.key === "End") {
      event.preventDefault();
      commit(GAIN_MAX);
    }
  };

  const position = gainToPos(value);

  return (
    <div
      className="fader"
      ref={track}
      role="slider"
      tabIndex={0}
      aria-label={label}
      aria-valuemin={GAIN_MIN}
      aria-valuemax={GAIN_MAX}
      aria-valuenow={Number(value.toFixed(1))}
      aria-valuetext={`${formatDb(value)} dB`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onDoubleClick={() => commit(0)}
      onKeyDown={onKeyDown}
    >
      <div className="fader__slot" />
      <div className="fader__fill" style={{ height: `${position * 100}%` }} />
      {TICKS.map((db) => (
        <div
          key={db}
          className="fader__tick"
          data-unity={db === 0 ? "" : undefined}
          style={{ bottom: `${gainToPos(db) * 100}%` }}
          aria-hidden="true"
        />
      ))}
      <div className="fader__thumb" style={{ bottom: `${position * 100}%` }}>
        <span />
      </div>
    </div>
  );
}
