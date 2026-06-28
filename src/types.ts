// Domain model shared with the Rust backend (serde renames keep these in sync).

export type SessionState = "working" | "waiting" | "ready";
export type ReqKind = "perm" | "ask" | null;

export interface Session {
  id: string;
  folder: string; // basename of cwd
  cwd: string; // full path shown in the row
  container: boolean;
  state: SessionState;
  reqKind: ReqKind;
  cmd: string | null; // pending tool/command for a permission request
  lastMsg: string;
  updatedAt: number; // epoch ms
}

export type ThemePref = "auto" | "light" | "dark";
export type BindAddr = "0.0.0.0:7337" | "127.0.0.1:7337" | string;

export interface AppConfig {
  bind: BindAddr;
  theme: ThemePref;
  accent: string;
  alwaysOnTop: boolean;
  autostart: boolean;
  notify: boolean;
  replyPerm: boolean; // allow/deny from the pill via the HTTP PreToolUse hook
  replyText: boolean; // free-text reply (requires SDK-mode sessions)
  token: string; // masked in the UI; revealed only on copy
  winX?: number; // persisted window corner (not shown in UI)
  winY?: number;
}

export interface Snapshot {
  sessions: Session[];
  config: AppConfig;
}

// "always" = allow now and remember the rule.
export type Decision = "allow" | "deny" | "always";

export const STATE_LABEL: Record<SessionState, string> = {
  waiting: "Te esperando",
  working: "Trabalhando",
  ready: "Pronto",
};

export const COUNT_WORD: Record<SessionState, string> = {
  waiting: "esperando",
  working: "trabalhando",
  ready: "prontas",
};

export const STATE_CLASS: Record<SessionState, string> = {
  waiting: "s-waiting",
  working: "s-working",
  ready: "s-ready",
};

// Worst-state ordering: waiting (🔴) > working (🟡) > ready (🟢).
const WORST_ORDER: SessionState[] = ["waiting", "working", "ready"];

export interface Derived {
  total: number;
  counts: Record<SessionState, number>;
  worst: SessionState;
  badgeCount: number;
  hasWait: boolean;
  headerSub: string;
}

export function derive(sessions: Session[]): Derived {
  const counts: Record<SessionState, number> = { waiting: 0, working: 0, ready: 0 };
  for (const s of sessions) counts[s.state]++;
  const worst = WORST_ORDER.find((st) => counts[st] > 0) ?? "ready";
  const badgeCount = counts[worst];
  const total = sessions.length;
  return {
    total,
    counts,
    worst,
    badgeCount,
    hasWait: counts.waiting > 0,
    headerSub: `${total} ${total === 1 ? "sessão" : "sessões"} · ${badgeCount} ${COUNT_WORD[worst]}`,
  };
}

// Relative time in the design's clipped style: "agora", "40 s", "3 min", "2 h".
export function relTime(updatedAt: number, nowMs: number): string {
  const sec = Math.max(0, Math.round((nowMs - updatedAt) / 1000));
  if (sec < 5) return "agora";
  if (sec < 60) return `${sec} s`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min} min`;
  const h = Math.round(min / 60);
  if (h < 24) return `${h} h`;
  return `${Math.round(h / 24)} d`;
}
