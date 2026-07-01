// Commands exposed to the frontend.

use tauri::{AppHandle, Emitter, Manager, State};

use crate::config;
use crate::server;
use crate::setup::{self, InstallReport};
use crate::state::{AppState, Config, ConfigPatch, Snapshot};

fn push(app: &AppHandle, state: &AppState) {
    if let Ok(g) = state.inner.lock() {
        let _ = app.emit("snapshot", g.snapshot());
    }
}

#[tauri::command]
pub fn get_state(state: State<AppState>) -> Snapshot {
    state.inner.lock().unwrap().snapshot()
}

/// The frontend reports panel open/close so the click-through poller knows
/// whether the whole window is interactive or only the collapsed pill corner.
#[tauri::command]
pub fn set_panel_open(open: bool, state: State<AppState>) {
    state.panel_open.store(open, std::sync::atomic::Ordering::Relaxed);
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
        if let Some(v) = patch.sound { c.sound = v; }

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
