import { useRef } from "react";
import { clamp, formatDb } from "@/engine/model";

type KnobProps = {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
};

/** Travel, in pixels, for a full sweep of the knob. */
const TRAVEL = 130;

/**
 * A rotary control, the way the mixer's own equaliser draws one: drag up and
 * down, hold Shift for fine moves, double-click to return to flat. The angle is
 * limited to the 270° a real knob travels, so the pointer never reads backwards.
 */
export function Knob({ label, value, min, max, step = 0.5, onChange }: KnobProps) {
  const drag = useRef<{ y: number; start: number } | null>(null);

  const commit = (next: number) => {
    const quantised = Math.round(clamp(next, min, max) / step) * step;
    onChange(Number(quantised.toFixed(3)));
  };

  const progress = (clamp(value, min, max) - min) / (max - min);
  const angle = -135 + progress * 270;

  return (
    <div className="knob-wrap">
      <span className="knob__label">{label}</span>
      <div
        className="knob"
        role="slider"
        tabIndex={0}
        aria-label={label}
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={Number(value.toFixed(1))}
        aria-valuetext={`${formatDb(value)} dB`}
        onPointerDown={(event) => {
          event.preventDefault();
          event.currentTarget.focus();
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = { y: event.clientY, start: value };
        }}
        onPointerMove={(event) => {
          if (!drag.current) return;
          const scale = event.shiftKey ? 0.2 : 1;
          const travelled = ((drag.current.y - event.clientY) / TRAVEL) * (max - min) * scale;
          commit(drag.current.start + travelled);
        }}
        onPointerUp={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
          }
          drag.current = null;
        }}
        onDoubleClick={() => commit(0)}
        onKeyDown={(event) => {
          const fine = event.shiftKey ? step / 5 : step;
          if (event.key === "ArrowUp" || event.key === "ArrowRight") {
            event.preventDefault();
            commit(value + fine);
          } else if (event.key === "ArrowDown" || event.key === "ArrowLeft") {
            event.preventDefault();
            commit(value - fine);
          } else if (event.key === "Home") {
            event.preventDefault();
            commit(min);
          } else if (event.key === "End") {
            event.preventDefault();
            commit(max);
          }
        }}
      >
        <span className="knob__cap">
          <span className="knob__pointer" style={{ transform: `rotate(${angle}deg)` }} />
        </span>
      </div>
      <span className="knob__value">{formatDb(value)}</span>
    </div>
  );
}
