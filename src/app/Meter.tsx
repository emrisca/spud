import { dbToPos } from "@/engine/model";
import { useClip, useMeterDb } from "@/engine/meters";

type MeterProps = {
  kind: "strip" | "bus";
  index: number;
};

/**
 * A peak meter.
 *
 * The gradient is painted once on the full column and a panel-coloured block
 * covers everything above the current level, so the colours stay anchored to
 * absolute dBFS instead of stretching with the bar.
 */
export function Meter({ kind, index }: MeterProps) {
  const db = useMeterDb(kind, index);
  const clip = useClip(index);
  const position = dbToPos(db);

  return (
    <div
      className={`meter meter--${kind}`}
      aria-hidden="true"
      title={db <= -59.9 ? "-∞ dBFS" : `${db.toFixed(1)} dBFS`}
    >
      <div className="meter__column" />
      <div className="meter__mask" style={{ height: `${(1 - position) * 100}%` }} />
      <span className="meter__clip" data-on={kind === "bus" && clip ? "true" : undefined} />
    </div>
  );
}
