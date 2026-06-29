// Bridge to the Rust backend. In a browser (vite dev, no Tauri) it falls back to
// an in-memory mock that mirrors the design prototype so the UI can be previewed
// and developed without the desktop shell.

import type { Snapshot, Session, Decision, AppConfig } from "./types";

type Listener = (snap: Snapshot) => void;

export interface Api {
  isTauri: boolean;
  getSnapshot(): Promise<Snapshot>;
  subscribe(cb: Listener): () => void;
  respond(id: string, decision: Decision): Promise<void>;
  setConfig(patch: Partial<AppConfig>): Promise<AppConfig>;
  regenerateToken(): Promise<string>;
  revealToken(): Promise<string>;
  saveWindow(x: number, y: number): Promise<void>;
  setPanelOpen(open: boolean): Promise<void>;
  installHooks(): Promise<InstallReport>;
  hooksInstalled(): Promise<boolean>;
  quit(): Promise<void>;
  onToggle(cb: () => void): () => void;
}

export interface InstallReport {
  claudeDir: string;
  settingsPath: string;
}

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function tauriApi(): Api {
  // Imported lazily so the browser bundle never touches Tauri internals.
  const core = () => import("@tauri-apps/api/core");
  const event = () => import("@tauri-apps/api/event");
  return {
    isTauri: true,
    async getSnapshot() {
      const { invoke } = await core();
      return invoke<Snapshot>("get_state");
    },
    subscribe(cb) {
      let unlisten: (() => void) | undefined;
      event().then(async ({ listen }) => {
        unlisten = await listen<Snapshot>("snapshot", (e) => cb(e.payload));
      });
      // Prime with the current state.
      this.getSnapshot().then(cb).catch(() => {});
      return () => unlisten?.();
    },
    async respond(id, decision) {
      const { invoke } = await core();
      await invoke("respond", { sessionId: id, decision });
    },
    async setConfig(patch) {
      const { invoke } = await core();
      return invoke<AppConfig>("set_config", { patch });
    },
    async regenerateToken() {
      const { invoke } = await core();
      return invoke<string>("regenerate_token");
    },
    async revealToken() {
      const { invoke } = await core();
      return invoke<string>("reveal_token");
    },
    async saveWindow(x, y) {
      const { invoke } = await core();
      await invoke("save_window", { x, y });
    },
    async setPanelOpen(open) {
      const { invoke } = await core();
      await invoke("set_panel_open", { open });
    },
    async installHooks() {
      const { invoke } = await core();
      return invoke<InstallReport>("install_hooks");
    },
    async hooksInstalled() {
      const { invoke } = await core();
      return invoke<boolean>("hooks_installed");
    },
    async quit() {
      const { invoke } = await core();
      await invoke("quit_app");
    },
    onToggle(cb) {
      let unlisten: (() => void) | undefined;
      event().then(async ({ listen }) => {
        unlisten = await listen("toggle-panel", () => cb());
      });
      return () => unlisten?.();
    },
  };
}

// ----------------------------- Browser mock -----------------------------

function mockApi(): Api {
  const now = Date.now();
  const listeners = new Set<Listener>();

  const config: AppConfig = {
    bind: "0.0.0.0:7337",
    theme: "light",
    accent: "#C96442",
    alwaysOnTop: true,
    autostart: false,
    notify: true,
    sound: true,
    replyPerm: true,
    token: "csf_demo_3f9a1c84e07b25d6",
  };

  let sessions: Session[] = [
    { id: "api", folder: "api-gateway", cwd: "~/dev/api-gateway", container: true, state: "waiting", reqKind: "perm", cmd: "npm run migrate:prod", lastMsg: "Quer rodar a migração no banco de produção?", updatedAt: now - 2_000 },
    { id: "sem", folder: "claude-semaforo", cwd: "~/dev/claude-semaforo", container: false, state: "waiting", reqKind: "ask", cmd: null, lastMsg: "Uso NSIS ou MSI pro instalador do Windows?", updatedAt: now - 60_000 },
    { id: "mob", folder: "mobile-app", cwd: "~/work/mobile-app", container: true, state: "working", reqKind: null, cmd: null, lastMsg: "Refatorando o módulo de autenticação…", updatedAt: now - 40_000 },
    { id: "mkt", folder: "marketing-site", cwd: "~/dev/marketing-site", container: false, state: "ready", reqKind: null, cmd: null, lastMsg: "Refiz a hero e os três cards de preço.", updatedAt: now - 180_000 },
    { id: "dat", folder: "data-pipeline", cwd: "~/dev/data-pipeline", container: false, state: "ready", reqKind: null, cmd: null, lastMsg: "Tudo verde: 42/42 testes passando.", updatedAt: now - 360_000 },
  ];

  const PERM = [
    { m: "Quer rodar a migração no banco de produção?", c: "npm run migrate:prod" },
    { m: "Posso apagar a branch antiga?", c: "git branch -D legacy" },
    { m: "Instalo a dependência nova?", c: "npm i zod" },
    { m: "Posso sobrescrever o .env?", c: "write .env" },
    { m: "Rodo a suíte E2E inteira?", c: "npm run e2e" },
  ];
  const Q = ["Uso NSIS ou MSI pro instalador do Windows?", "Quer dark mode também?", "Mantenho compatibilidade com a v1?", "Qual nome dou pro endpoint novo?", "Componente separado ou inline?"];
  const DONE = ["Pronto — terminei o refactor.", "Feito. 18 arquivos alterados.", "Build passou, tudo verde.", "Concluído, dá uma revisada.", "Pronto pra revisar o PR."];
  const WORK = ["Pensando…", "Lendo os arquivos do projeto…", "Rodando os testes…", "Escrevendo o código…", "Refatorando o módulo…"];

  const rnd = <T,>(a: T[]) => a[Math.floor(Math.random() * a.length)];
  const snap = (): Snapshot => ({ sessions: sessions.map((s) => ({ ...s })), config: { ...config } });
  const emit = () => listeners.forEach((l) => l(snap()));

  const patch = (id: string, p: Partial<Session>) => {
    sessions = sessions.map((s) => (s.id === id ? { ...s, ...p, updatedAt: Date.now() } : s));
    emit();
  };
  const pick = (st: Session["state"]) => { const a = sessions.filter((x) => x.state === st); return a.length ? rnd(a) : null; };
  const pickNot = (st: Session["state"]) => { const a = sessions.filter((x) => x.state !== st); return a.length ? rnd(a) : null; };

  const doPerm = () => { const s = pick("working") || pick("ready") || pickNot("waiting"); if (s) { const p = rnd(PERM); patch(s.id, { state: "waiting", reqKind: "perm", cmd: p.c, lastMsg: p.m }); } };
  const doAsk = () => { const s = pick("working") || pick("ready") || pickNot("waiting"); if (s) patch(s.id, { state: "waiting", reqKind: "ask", cmd: null, lastMsg: rnd(Q) }); };
  const doFinish = () => { const s = pick("working") || pickNot("ready"); if (s) patch(s.id, { state: "ready", reqKind: null, cmd: null, lastMsg: rnd(DONE) }); };
  const doPrompt = () => { const s = pick("ready") || pickNot("working"); if (s) patch(s.id, { state: "working", reqKind: null, cmd: null, lastMsg: rnd(WORK) }); };

  const allow = (id: string) => { const s = sessions.find((x) => x.id === id); patch(id, { state: "working", reqKind: null, cmd: null, lastMsg: s?.cmd ? `Rodando ${s.cmd}…` : rnd(WORK) }); };
  const deny = (id: string) => patch(id, { state: "working", reqKind: null, cmd: null, lastMsg: "Ok — vou por outro caminho." });

  const tick = () => {
    const w = sessions.filter((x) => x.state === "waiting");
    const k = sessions.filter((x) => x.state === "working").length;
    const r = sessions.filter((x) => x.state === "ready").length;
    if (w.length >= 2) { const ws = w[0]; return ws.reqKind === "perm" ? allow(ws.id) : deny(ws.id); }
    if (k === 0 && w.length === 0) return doPrompt();
    const d = Math.random();
    if (d < 0.3 && k > 0) return doAsk();
    if (d < 0.55 && k > 0) return doPerm();
    if (d < 0.8 && k > 0) return doFinish();
    if (w.length > 0) { const ws = w[0]; return ws.reqKind === "perm" ? allow(ws.id) : deny(ws.id); }
    if (r > 0) return doPrompt();
    return doPerm();
  };
  // `?static` freezes the seed state for inspecting the waiting-state UI.
  if (!new URLSearchParams(location.search).has("static")) setInterval(tick, 2400);

  return {
    isTauri: false,
    async getSnapshot() { return snap(); },
    subscribe(cb) { listeners.add(cb); cb(snap()); return () => listeners.delete(cb); },
    async respond(id, decision) { if (decision === "deny") deny(id); else allow(id); },
    async setConfig(p) { Object.assign(config, p); emit(); return { ...config }; },
    async regenerateToken() { config.token = "csf_" + Math.random().toString(16).slice(2).padEnd(16, "0").slice(0, 16); emit(); return config.token; },
    async revealToken() { return config.token; },
    async saveWindow() { /* no-op in the browser */ },
    async setPanelOpen() { /* no-op in the browser */ },
    async installHooks() { return { claudeDir: "~/.claude", settingsPath: "~/.claude/settings.json" }; },
    async hooksInstalled() { return false; },
    async quit() { /* no-op in the browser */ },
    onToggle() { return () => {}; },
  };
}

export const api: Api = isTauri ? tauriApi() : mockApi();
