// Window choreography for the always-on-top widget: collapse to the pill,
// expand to the panel while keeping the bottom-right corner anchored, and
// drag the whole window by the pill. All functions are no-ops outside Tauri.

import type { PointerEvent as ReactPointerEvent } from "react";
import { api } from "./api";

const COLLAPSED = { w: 96, h: 96 };
const OPEN = { w: 398, h: 600 };

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function winMod() {
  return import("@tauri-apps/api/window");
}

function clamp(v: number, min: number, max: number) {
  return Math.max(min, Math.min(max, v));
}

/** Resize between pill and panel, anchored to the current bottom-right corner. */
export async function applyOpen(open: boolean): Promise<void> {
  if (!isTauri) return;
  const { PhysicalPosition, PhysicalSize, getCurrentWindow, currentMonitor } = await winMod();
  const w = getCurrentWindow();
  const scale = await w.scaleFactor();
  const pos = await w.outerPosition();
  const size = await w.outerSize();
  const right = pos.x + size.width;
  const bottom = pos.y + size.height;

  const target = open ? OPEN : COLLAPSED;
  const nw = Math.round(target.w * scale);
  const nh = Math.round(target.h * scale);

  let x = right - nw;
  let y = bottom - nh;

  const mon = await currentMonitor();
  if (mon) {
    x = clamp(x, mon.position.x, Math.max(mon.position.x, mon.position.x + mon.size.width - nw));
    y = clamp(y, mon.position.y, Math.max(mon.position.y, mon.position.y + mon.size.height - nh));
  }

  await w.setSize(new PhysicalSize(nw, nh));
  await w.setPosition(new PhysicalPosition(x, y));
}

/** Restore the saved corner on launch, then settle into the collapsed pill. */
export async function restorePosition(winX?: number, winY?: number): Promise<void> {
  if (!isTauri) return;
  const { PhysicalPosition, getCurrentWindow } = await winMod();
  const w = getCurrentWindow();
  if (typeof winX === "number" && typeof winY === "number") {
    await w.setPosition(new PhysicalPosition(Math.round(winX), Math.round(winY)));
  }
  await applyOpen(false);
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
