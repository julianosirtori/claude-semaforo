import type { CSSProperties, PointerEvent as ReactPointerEvent } from "react";

interface Props {
  empty: boolean;
  badgeVar: string; // e.g. "var(--wait)"
  badgeLabel: string; // count, or "·" when empty
  hasWait: boolean;
  onPointerDown: (e: ReactPointerEvent) => void;
}

export function Pill({ empty, badgeVar, badgeLabel, hasWait, onPointerDown }: Props) {
  return (
    <div
      className={`pill${empty ? " pill--empty" : ""}`}
      style={{ ["--badge" as keyof CSSProperties]: badgeVar } as CSSProperties}
      onPointerDown={onPointerDown}
      onClick={(e) => e.stopPropagation()}
      title="Claude Semáforo"
    >
      {hasWait && <span className="pill__halo" />}
      <div className="pill__face">
        <span className="pill__count">{badgeLabel}</span>
      </div>
      <span className="pill__dot" />
    </div>
  );
}
