import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "./api";
import { applyOpen, beginDrag, restorePosition } from "./window";
import { derive, type Decision, type AppConfig, type Snapshot, type SessionState } from "./types";
import { Pill } from "./components/Pill";
import { Panel } from "./components/Panel";

const BADGE_VAR: Record<SessionState, string> = {
  waiting: "var(--wait)",
  working: "var(--work)",
  ready: "var(--ready)",
};

function effectiveTheme(pref: AppConfig["theme"]): "light" | "dark" {
  if (pref === "auto") return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  return pref;
}

export default function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [open, setOpen] = useState(false);
  const [view, setView] = useState<"list" | "config">("list");
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [toast, setToast] = useState<string | null>(null);
  const [flashId, setFlashId] = useState<string | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [regenSpinning, setRegenSpinning] = useState(false);

  const prevUpdated = useRef<Map<string, number>>(new Map());
  const restored = useRef(false);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const flashTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  // Subscribe to backend snapshots; flash whichever session just changed.
  useEffect(() => {
    return api.subscribe((snap) => {
      let changed: { id: string; at: number } | null = null;
      for (const s of snap.sessions) {
        const prev = prevUpdated.current.get(s.id);
        if (prev !== undefined && s.updatedAt > prev && (!changed || s.updatedAt > changed.at)) {
          changed = { id: s.id, at: s.updatedAt };
        }
      }
      prevUpdated.current = new Map(snap.sessions.map((s) => [s.id, s.updatedAt]));
      if (changed) {
        setFlashId(changed.id);
        clearTimeout(flashTimer.current);
        flashTimer.current = setTimeout(() => setFlashId(null), 780);
      }
      setSnapshot(snap);
    });
  }, []);

  // Restore the saved window corner once, after the first snapshot.
  useEffect(() => {
    if (snapshot && !restored.current) {
      restored.current = true;
      restorePosition(snapshot.config.winX, snapshot.config.winY);
    }
  }, [snapshot]);

  // Theme + accent follow config (and the OS when set to "auto").
  useEffect(() => {
    if (!snapshot) return;
    const { theme, accent } = snapshot.config;
    const root = document.documentElement;
    const apply = () => { root.dataset.theme = effectiveTheme(theme); };
    apply();
    root.style.setProperty("--accent", accent);
    if (theme === "auto") {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      mq.addEventListener("change", apply);
      return () => mq.removeEventListener("change", apply);
    }
  }, [snapshot?.config.theme, snapshot?.config.accent]);

  // Resize the window between pill and panel.
  useEffect(() => { if (restored.current) applyOpen(open); }, [open]);

  // Tick relative timestamps.
  useEffect(() => {
    const iv = setInterval(() => setNowMs(Date.now()), 10_000);
    return () => clearInterval(iv);
  }, []);

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    clearTimeout(toastTimer.current);
    toastTimer.current = setTimeout(() => setToast(null), 1700);
  }, []);

  const toggle = useCallback(() => setOpen((o) => !o), []);
  const onPatch = useCallback((patch: Partial<AppConfig>) => { api.setConfig(patch); }, []);

  const respond = useCallback((id: string, decision: Decision) => { api.respond(id, decision); }, []);
  const onAlways = useCallback((id: string) => { api.respond(id, "always"); showToast("Regra criada · sempre permitir"); }, [showToast]);
  const onSend = useCallback((id: string) => {
    const text = (drafts[id] ?? "").trim();
    api.replyText(id, text);
    setDrafts((dr) => ({ ...dr, [id]: "" }));
  }, [drafts]);

  const onCopyToken = useCallback(async () => {
    try {
      const token = await api.revealToken();
      await navigator.clipboard.writeText(token);
      showToast("Token copiado");
    } catch { showToast("Não consegui copiar"); }
  }, [showToast]);

  const onRegenToken = useCallback(async () => {
    setRegenSpinning(true);
    try { await api.regenerateToken(); showToast("Token novo gerado"); }
    finally { setTimeout(() => setRegenSpinning(false), 600); }
  }, [showToast]);

  if (!snapshot) return <div className="app" />;

  const d = derive(snapshot.sessions);

  return (
    <div className="app" onClick={() => open && setOpen(false)}>
      <Panel
        open={open}
        view={view}
        snapshot={snapshot}
        derived={d}
        flashId={flashId}
        nowMs={nowMs}
        drafts={drafts}
        regenSpinning={regenSpinning}
        onShowConfig={() => { setView("config"); setOpen(true); }}
        onShowList={() => setView("list")}
        onClose={() => setOpen(false)}
        onPatch={onPatch}
        onCopyToken={onCopyToken}
        onRegenToken={onRegenToken}
        onAllow={(id) => respond(id, "allow")}
        onAlways={onAlways}
        onDeny={(id) => respond(id, "deny")}
        onSend={onSend}
        onDraft={(id, v) => setDrafts((dr) => ({ ...dr, [id]: v }))}
      />

      {!open && d.hasWait && <div className="nudge">clique pra abrir</div>}

      <Pill
        empty={d.total === 0}
        badgeVar={BADGE_VAR[d.worst]}
        badgeLabel={d.total ? String(d.badgeCount) : "·"}
        hasWait={d.hasWait}
        onPointerDown={(e) => { e.stopPropagation(); beginDrag(e, toggle); }}
      />

      {toast && (
        <div className="toast" onClick={(e) => e.stopPropagation()}>
          <span className="toast__dot" />
          {toast}
        </div>
      )}
    </div>
  );
}
