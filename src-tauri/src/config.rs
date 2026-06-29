// Config persistence (a JSON file in the app config dir) and token generation.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::state::Config;

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    Some(dir.join("config.json"))
}

fn allow_rules_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    Some(dir.join("allow_rules.json"))
}

/// Tighten a file holding a secret (the token, or the config that embeds it) to
/// owner read/write on Unix. Inherited umask is typically world-readable, which
/// leaks the token on a shared host. A no-op on Windows.
#[cfg(unix)]
pub fn restrict_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
pub fn restrict_owner_only(_path: &Path) {}

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
        if fs::write(&path, json).is_ok() {
            restrict_owner_only(&path); // config.json embeds the token
        }
    }
}

/// Load the persisted "always allow" rules. Missing or corrupt file → empty set,
/// so a bad rules file never blocks startup.
pub fn load_allow_rules(app: &AppHandle) -> HashSet<String> {
    allow_rules_path(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|s| parse_rules(&s))
        .unwrap_or_default()
}

/// Persist the rules to their own sidecar, kept separate from `Config` so token
/// rotation never churns the rules file (and vice versa).
pub fn save_allow_rules(app: &AppHandle, rules: &HashSet<String>) {
    let Some(path) = allow_rules_path(app) else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(rules) {
        let _ = fs::write(path, json);
    }
}

fn parse_rules(s: &str) -> HashSet<String> {
    serde_json::from_str(s).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rules_round_trips_and_tolerates_garbage() {
        let json = serde_json::to_string(&HashSet::from([
            "Write /a/.env".to_string(),
            "Bash npm run migrate:prod".to_string(),
        ]))
        .unwrap();
        let set = parse_rules(&json);
        assert!(set.contains("Write /a/.env"));
        assert!(set.contains("Bash npm run migrate:prod"));
        assert_eq!(parse_rules("not json at all"), HashSet::new());
        assert_eq!(parse_rules(""), HashSet::new());
    }

    #[cfg(unix)]
    #[test]
    fn restrict_owner_only_sets_0600() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("semaforo_perm_{}.tmp", std::process::id()));
        fs::write(&path, "secret").unwrap();
        restrict_owner_only(&path);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_file(&path);
    }
}
