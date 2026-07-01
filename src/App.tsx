import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "./api";
import { applyOpen, beginDrag, restorePosition } from "./window";
import { derive, nextCue, type AppConfig, type Snapshot, type SessionState } from "./types";
import { Pill } from "./components/Pill";
import { Panel } from "./components/Panel";
import { playState } from "./sound";
import notifyScript from "../hooks/notify.sh?raw";

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
  const [toast, setToast] = useState<string | null>(null);
  const [flashId, setFlashId] = useState<string | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [regenSpinning, setRegenSpinning] = useState(false);
  const [hooksInstalled, setHooksInstalled] = useState(false);
  const [installing, setInstalling] = useState(false);

  const prevUpdated = useRef<Map<string, number>>(new Map());
  const prevState = useRef<Map<string, SessionState>>(new Map());
  const restored = useRef(false);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const flashTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  // Subscribe to backend snapshots; flash whichever session just changed and
  // sound a cue for sessions entering waiting/ready (waiting wins if both happen).
  useEffect(() => {
    return api.subscribe((snap) => {
      let changed: { id: string; at: number } | null = null;
      for (const s of snap.sessions) {
        const prevAt = prevUpdated.current.get(s.id);
        if (prevAt !== undefined && s.updatedAt > prevAt && (!changed || s.updatedAt > changed.at)) {
          changed = { id: s.id, at: s.updatedAt };
        }
      }
      const cue = nextCue(prevState.current, snap.sessions);
      prevUpdated.current = new Map(snap.sessions.map((s) => [s.id, s.updatedAt]));
      prevState.current = new Map(snap.sessions.map((s) => [s.id, s.state]));
      if (changed) {
        setFlashId(changed.id);
        clearTimeout(flashTimer.current);
        flashTimer.current = setTimeout(() => setFlashId(null), 780);
      }
      if (cue && snap.config.sound) playState(cue);
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

  // The window is a fixed size and never resizes (no transparent-resize
  // flicker); opening just informs the backend (for click-through) while the
  // panel animates in via CSS.
  useEffect(() => {
    if (!restored.current) return;
    applyOpen(open);
  }, [open]);

  // Tick relative timestamps.
  useEffect(() => {
    const iv = setInterval(() => setNowMs(Date.now()), 10_000);
    return () => clearInterval(iv);
  }, []);

  // Tray (or its menu) toggles the panel.
  useEffect(() => api.onToggle(() => setOpen((o) => !o)), []);

  // Reflect whether the Claude Code hooks are already wired up.
  useEffect(() => { api.hooksInstalled().then(setHooksInstalled).catch(() => {}); }, []);

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    clearTimeout(toastTimer.current);
    toastTimer.current = setTimeout(() => setToast(null), 1700);
  }, []);

  const toggle = useCallback(() => setOpen((o) => !o), []);
  const onPatch = useCallback((patch: Partial<AppConfig>) => { api.setConfig(patch); }, []);

  const onCopyToken = useCallback(async () => {
    try {
      const token = await api.revealToken();
      await navigator.clipboard.writeText(token);
      showToast("Token copiado");
    } catch { showToast("Não consegui copiar"); }
  }, [showToast]);

  // Copy a self-contained bootstrap to wire a devcontainer to the host: it
  // writes the token, the notify.sh hook, and the hooks settings — each under
  // its ~/.claude path. The container reaches the host via host.docker.internal.
  const onCopyContainer = useCallback(async () => {
    try {
      const token = await api.revealToken();
      const cmd = 'bash "$HOME/.claude/notify.sh"';
      const settings = {
        hooks: {
          UserPromptSubmit: [{ hooks: [{ type: "command", command: cmd }] }],
          Notification: [{ hooks: [{ type: "command", command: cmd }] }],
          PostToolUse: [{ hooks: [{ type: "command", command: cmd }] }],
          Stop: [{ hooks: [{ type: "command", command: cmd }] }],
          SessionEnd: [{ hooks: [{ type: "command", command: cmd }] }],
        },
      };
      const bootstrap = [
        "# Claude Semáforo — rode dentro do devcontainer pra reportar pro host.",
        "# O container alcança o host em host.docker.internal:7337.",
        "mkdir -p ~/.claude",
        "",
        "# ~/.claude/semaforo.token",
        `printf '%s' '${token}' > ~/.claude/semaforo.token`,
        "",
        "# ~/.claude/notify.sh",
        "cat > ~/.claude/notify.sh <<'NOTIFY_EOF'",
        notifyScript.replace(/\r?\n$/, ""),
        "NOTIFY_EOF",
        "chmod +x ~/.claude/notify.sh",
        "",
        "# ~/.claude/settings.json (hooks)",
        "cat > ~/.claude/settings.json <<'SETTINGS_EOF'",
        JSON.stringify(settings, null, 2),
        "SETTINGS_EOF",
        "",
      ].join("\n");
      await navigator.clipboard.writeText(bootstrap);
      showToast("Setup do container copiado");
    } catch { showToast("Não consegui copiar"); }
  }, [showToast]);

  const onRegenToken = useCallback(async () => {
    setRegenSpinning(true);
    try { await api.regenerateToken(); showToast("Token novo gerado"); }
    finally { setTimeout(() => setRegenSpinning(false), 600); }
  }, [showToast]);

  const onInstallHooks = useCallback(async () => {
    setInstalling(true);
    try {
      const report = await api.installHooks();
      setHooksInstalled(true);
      showToast(`Hooks instalados em ${report.claudeDir}`);
    } catch {
      showToast("Não consegui instalar os hooks");
    } finally {
      setInstalling(false);
    }
  }, [showToast]);

  const onQuit = useCallback(() => { api.quit(); }, []);

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
        regenSpinning={regenSpinning}
        onShowConfig={() => { setView("config"); setOpen(true); }}
        onShowList={() => setView("list")}
        onClose={() => setOpen(false)}
        onPatch={onPatch}
        onCopyToken={onCopyToken}
        onRegenToken={onRegenToken}
        hooksInstalled={hooksInstalled}
        installing={installing}
        onInstallHooks={onInstallHooks}
        onCopyContainer={onCopyContainer}
        onQuit={onQuit}
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
