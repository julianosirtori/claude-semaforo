// Window choreography for the always-on-top widget: collapse to the pill,
// expand to the panel while keeping the bottom-right corner anchored, and
// drag the whole window by the pill. All functions are no-ops outside Tauri.

import type { PointerEvent as ReactPointerEvent } from "react";
import { api } from "./api";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function winMod() {
  return import("@tauri-apps/api/window");
}

/**
 * Tell the backend whether the panel is open. The window itself is a fixed
 * panel-sized rect that never resizes — that's what kills the transparent-window
 * flicker. The backend uses this only to size the click-through region.
 */
export async function applyOpen(open: boolean): Promise<void> {
  await api.setPanelOpen(open);
}

/** Initial positioning is handled by the Rust side (position_window). This just
 *  tells the backend the panel starts collapsed and lets App mark itself ready. */
export async function restorePosition(_winX?: number, _winY?: number): Promise<void> {
  await api.setPanelOpen(false);
}

/**
 * Pointer drag from the pill. A press without movement is a click (→ onClick);
 * any movement drags the window. The corner is persisted when the drag ends.
 */
export function beginDrag(e: ReactPointerEvent, onClick: () => void): void {
  if (!isTauri) { onClick(); return; }
  const startSX = e.screenX;
  const startSY = e.screenY;
  const target = e.currentTarget as HTMLElement;
  const pointerId = e.pointerId;
  try { target.setPointerCapture(pointerId); } catch { /* ignore */ }

  let moved = false;
  let setPos: ((x: number, y: number) => void) | null = null;
  let baseX = 0, baseY = 0, scale = 1;
  let last: { x: number; y: number } | null = null;

  (async () => {
    const { PhysicalPosition, getCurrentWindow } = await winMod();
    const w = getCurrentWindow();
    scale = await w.scaleFactor();
    const p = await w.outerPosition();
    baseX = p.x; baseY = p.y;
    setPos = (x, y) => { void w.setPosition(new PhysicalPosition(x, y)); };
  })();

  const move = (ev: PointerEvent): void => {
    const dist = Math.hypot(ev.screenX - startSX, ev.screenY - startSY);
    if (!moved && dist > 3) moved = true;
    if (!moved || !setPos) return;
    last = {
      x: Math.round(baseX + (ev.screenX - startSX) * scale),
      y: Math.round(baseY + (ev.screenY - startSY) * scale),
    };
    setPos(last.x, last.y);
  };

  const up = (): void => {
    target.removeEventListener("pointermove", move);
    target.removeEventListener("pointerup", up);
    try { target.releasePointerCapture(pointerId); } catch { /* ignore */ }
    if (!moved) onClick();
    else if (last) api.saveWindow(last.x, last.y).catch(() => {});
  };

  target.addEventListener("pointermove", move);
  target.addEventListener("pointerup", up);
}
