// Config persistence (a JSON file in the app config dir) and token generation.

use std::fs;
use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::state::Config;

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    Some(dir.join("config.json"))
}

/// Load persisted config, applying env overrides and ensuring a token exists.
pub fn load(app: &AppHandle) -> Config {
    let existing = config_path(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Config>(&s).ok());

    let mut cfg = existing.clone().unwrap_or_default();
    let mut needs_save = existing.is_none();
    if cfg.token.is_empty() {
        cfg.token = generate_token();
        needs_save = true;
    }
    // Persist the generated token so it stays stable across launches.
    // Env overrides below are runtime-only and intentionally not persisted.
    if needs_save {
        save(app, &cfg);
    }

    if let Ok(token) = std::env::var("SEMAFORO_TOKEN") {
        if !token.is_empty() {
            cfg.token = token;
        }
    }
    if let Ok(bind) = std::env::var("SEMAFORO_BIND") {
        if !bind.is_empty() {
            cfg.bind = bind;
        }
    }
    cfg
}

pub fn save(app: &AppHandle, cfg: &Config) {
    let Some(path) = config_path(app) else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(path, json);
    }
}

pub fn generate_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("system RNG");
    let mut s = String::from("csf_");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
