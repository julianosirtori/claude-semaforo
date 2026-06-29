// Notification tones synthesized via Web Audio — no audio files to ship.
// Browsers (and the Tauri webview) start the AudioContext "suspended" until a
// user gesture. We resume on demand and, while still blocked, arm a one-shot
// unlock on the next pointer/key event so later transitions do sound.

import type { SessionState } from "./types";

type Cue = "waiting" | "ready";
type Note = [freq: number, at: number, dur: number];

// waiting → insistent two-note nudge; ready → bright rising chime.
const TONES: Record<Cue, Note[]> = {
  waiting: [[784, 0, 0.16], [784, 0.2, 0.18]],
  ready: [[659.25, 0, 0.14], [987.77, 0.12, 0.22]],
};

const PEAK = 0.18;

let ctx: AudioContext | null = null;
let unlockArmed = false;

function audioContext(): AudioContext | null {
  if (typeof window === "undefined") return null;
  const Ctor = window.AudioContext ?? (window as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!Ctor) return null;
  if (!ctx) ctx = new Ctor();
  return ctx;
}

function armUnlock(c: AudioContext) {
  if (unlockArmed) return;
  unlockArmed = true;
  const unlock = () => { c.resume().catch(() => {}); };
  window.addEventListener("pointerdown", unlock, { once: true });
  window.addEventListener("keydown", unlock, { once: true });
}

function tone(c: AudioContext, [freq, at, dur]: Note) {
  const osc = c.createOscillator();
  const gain = c.createGain();
  osc.type = "sine";
  osc.frequency.value = freq;
  osc.connect(gain).connect(c.destination);
  const start = c.currentTime + at;
  // exponential ramps can't touch zero, so floor the envelope at a hair above it.
  gain.gain.setValueAtTime(0.0001, start);
  gain.gain.exponentialRampToValueAtTime(PEAK, start + 0.012);
  gain.gain.exponentialRampToValueAtTime(0.0001, start + dur);
  osc.start(start);
  osc.stop(start + dur + 0.02);
}

export function playState(state: SessionState) {
  if (state !== "waiting" && state !== "ready") return;
  const c = audioContext();
  if (!c) return;
  c.resume().catch(() => {});
  if (c.state === "suspended") armUnlock(c);
  for (const note of TONES[state]) tone(c, note);
}
