# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Claude Semáforo is a Tauri 2 always-on-top widget (Rust backend + React/TS frontend) that aggregates the state of every Claude Code session — including ones in containers — into one pill. It is **status-only**: each session is `working`/`waiting`/`ready`, nothing more. It does not answer permission prompts. `ARCHITECTURE.md` is the canonical design doc (written in Portuguese, by author preference); read it before non-trivial backend work.

## Commands

```bash
npm run dev          # frontend only against the in-memory mock, http://localhost:1420
                     # append ?static to the URL to freeze the seeded state
npm run tauri dev    # full desktop app (Rust + frontend)
npm run build        # tsc typecheck + production bundle
npm run tauri build  # installers in src-tauri/target/release/bundle
npm test             # frontend unit tests (vitest)

# single frontend test
npx vitest run src/types.test.ts
npx vitest run -t "derive"

# backend tests
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml merge   # single test by name
```

## Architecture

**Two runtimes, one shared state model.** The Rust backend runs an HTTP listener (`tiny_http`, sync, one thread per request) on `0.0.0.0:7337`; the frontend renders in a transparent, decorationless, always-on-top window. All backend state lives in memory behind a single `Arc<Mutex<Inner>>` shared between the Tauri command handlers and the HTTP server thread.

**Lifecycle events are the only mechanism.** State events go to `/events`: `UserPromptSubmit`→working, `Notification`→waiting, `Stop`→ready, `SessionEnd`→remove, and `PostToolUse` un-sticks a session left waiting. The widget never gates tool calls — there is no `PreToolUse`/`/permission` hook, so a session in `auto` mode is never prompted on its behalf. `tiny_http`'s one-thread-per-request model still keeps a slow `/events` POST from stalling others. Every request requires `Authorization: Bearer <token>`, compared in constant time.

**`0.0.0.0` bind is deliberate** so containers reach the host (`host.docker.internal`). Container sessions are detected by the `X-Semaforo-Container` header (reliable) with source-IP-not-loopback as fallback.

**Frontend data flow.** `App.tsx` calls `api.subscribe(cb)` and holds the latest `Snapshot` (`{ sessions, config }`); everything else is derived at render time. `derive()` in `src/types.ts` is the single source of truth for the badge, chips, and subtitle (worst-state order: `waiting > working > ready`). The row "flash" on change is computed client-side by diffing each session's `updatedAt`, so the backend never signals it.

**The browser mock is a first-class path.** `api.ts` detects Tauri via `"__TAURI_INTERNALS__" in window`; outside Tauri it serves an in-memory mock that mirrors the design prototype (same sessions, an auto-driver cycling states every 2.4s, same actions). This lets the UI be developed in a plain browser without the desktop shell. Window-choreography calls in `window.ts` become no-ops outside Tauri.

**Theming is variable-swap only.** `styles.css` defines the full palette in CSS variables under `:root` (light) and `[data-theme="dark"]`; state colors and the accent are variables too. `App` resolves the effective theme (Auto follows `prefers-color-scheme`) and sets `document.documentElement.dataset.theme` + `--accent`. No Tailwind, no UI framework.

## Conventions

- The widget is desktop-only. `src-tauri/icons/` holds desktop targets (PNG sizes, `icon.ico`, `icon.icns`, Windows `Square*Logo`); do not commit the `android/`/`ios/` dirs or `64x64.png` that `tauri icon` also emits.
- `set_config` owns side effects: changing the bind restarts the server, toggling always-on-top calls the window, toggling autostart calls the plugin.
- Hooks (`hooks/notify.sh`, `hooks/notify.ps1`) are installed into `~/.claude/` either one-click (pill → gear → Claude Code → Instalar, via `setup.rs`) or by copying `.claude/settings.local.example.json`. The merge logic in `setup.rs` is pure and idempotent — keep it testable.
- The token lives in `~/.claude/semaforo.token` so regenerating it never requires reinstalling hooks. `SEMAFORO_TOKEN` and `SEMAFORO_BIND` env vars override runtime config.
