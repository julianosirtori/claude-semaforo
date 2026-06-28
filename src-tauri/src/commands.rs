// Commands exposed to the frontend.

use tauri::{AppHandle, Emitter, Manager, State};

use crate::config;
use crate::server;
use crate::setup::{self, InstallReport};
use crate::state::{now_ms, AppState, Config, ConfigPatch, Decision, SessionState, Snapshot};

fn push(app: &AppHandle, state: &AppState) {
    if let Ok(g) = state.inner.lock() {
        let _ = app.emit("snapshot", g.snapshot());
    }
}

#[tauri::command]
pub fn get_state(state: State<AppState>) -> Snapshot {
    state.inner.lock().unwrap().snapshot()
}

#[tauri::command]
pub fn respond(session_id: String, decision: String, state: State<AppState>, app: AppHandle) {
    {
        let mut g = state.inner.lock().unwrap();

        let rule = g.sessions.get(&session_id).and_then(|s| s.cmd.clone());
        if decision == "always" {
            if let Some(cmd) = rule {
                g.allow_rules.insert(cmd);
            }
        }

        let dec = if decision == "deny" { Decision::Deny } else { Decision::Allow };

        if let Some(s) = g.sessions.get_mut(&session_id) {
            let cmd = s.cmd.take();
            s.state = SessionState::Working;
            s.req_kind = None;
            s.last_msg = if decision == "deny" {
                "Ok — vou por outro caminho.".into()
            } else {
                cmd.map(|c| format!("Rodando {c}…")).unwrap_or_else(|| "Voltando ao trabalho…".into())
            };
            s.updated_at = now_ms();
        }

        if let Some(tx) = g.pending.remove(&session_id) {
            let _ = tx.send(dec);
        }
    }
    push(&app, &state);
}

#[tauri::command]
pub fn reply_text(session_id: String, text: String, state: State<AppState>, app: AppHandle) {
    {
        let mut g = state.inner.lock().unwrap();
        if let Some(s) = g.sessions.get_mut(&session_id) {
            s.state = SessionState::Working;
            s.req_kind = None;
            s.cmd = None;
            s.last_msg = if text.trim().is_empty() {
                "Voltando ao trabalho…".into()
            } else {
                "Voltando ao trabalho…".into()
            };
            s.updated_at = now_ms();
        }
        if let Some(tx) = g.pending.remove(&session_id) {
            let _ = tx.send(Decision::Ask);
        }
    }
    push(&app, &state);
}

#[tauri::command]
pub fn get_config(state: State<AppState>) -> Config {
    state.inner.lock().unwrap().config.clone()
}

#[tauri::command]
pub fn set_config(patch: ConfigPatch, state: State<AppState>, app: AppHandle) -> Config {
    let mut bind_changed = false;
    let mut aot_changed = false;
    let mut autostart_changed = false;

    let new_cfg = {
        let mut g = state.inner.lock().unwrap();
        let c = &mut g.config;

        if let Some(v) = patch.bind {
            bind_changed = v != c.bind;
            c.bind = v;
        }
        if let Some(v) = patch.theme { c.theme = v; }
        if let Some(v) = patch.accent { c.accent = v; }
        if let Some(v) = patch.always_on_top {
            aot_changed = v != c.always_on_top;
            c.always_on_top = v;
        }
        if let Some(v) = patch.autostart {
            autostart_changed = v != c.autostart;
            c.autostart = v;
        }
        if let Some(v) = patch.notify { c.notify = v; }
        if let Some(v) = patch.reply_perm { c.reply_perm = v; }
        if let Some(v) = patch.reply_text { c.reply_text = v; }

        config::save(&app, c);
        c.clone()
    };

    if bind_changed {
        let mut slot = state.server.lock().unwrap();
        if let Some(old) = slot.take() {
            old.stop();
        }
        *slot = server::start(&new_cfg.bind, state.inner.clone(), app.clone());
    }
    if aot_changed {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.set_always_on_top(new_cfg.always_on_top);
        }
    }
    if autostart_changed {
        apply_autostart(&app, new_cfg.autostart);
    }

    push(&app, &state);
    new_cfg
}

#[tauri::command]
pub fn regenerate_token(state: State<AppState>, app: AppHandle) -> String {
    let token = config::generate_token();
    {
        let mut g = state.inner.lock().unwrap();
        g.config.token = token.clone();
        config::save(&app, &g.config);
    }
    setup::sync_token(&token); // keep an installed hook setup working
    push(&app, &state);
    token
}

/// Write the hook scripts + token to ~/.claude and merge the hooks into
/// ~/.claude/settings.json so every Claude Code session reports here.
#[tauri::command]
pub fn install_hooks(state: State<AppState>, app: AppHandle) -> Result<InstallReport, String> {
    let token = state.inner.lock().unwrap().config.token.clone();
    setup::install(&app, &token)
}

#[tauri::command]
pub fn hooks_installed() -> bool {
    setup::is_installed()
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn reveal_token(state: State<AppState>) -> String {
    state.inner.lock().unwrap().config.token.clone()
}

#[tauri::command]
pub fn save_window(x: i32, y: i32, state: State<AppState>, app: AppHandle) {
    let mut g = state.inner.lock().unwrap();
    g.config.win_x = Some(x);
    g.config.win_y = Some(y);
    config::save(&app, &g.config);
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn apply_autostart(app: &AppHandle, enable: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let _ = if enable { manager.enable() } else { manager.disable() };
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn apply_autostart(_app: &AppHandle, _enable: bool) {}
