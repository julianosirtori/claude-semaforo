// One-click Claude Code wiring: drop the hook scripts and the token in
// ~/.claude/, and merge the hook registrations into ~/.claude/settings.json.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;

// Bundled at compile time so the installed app needs no repo on disk.
const NOTIFY_SH: &str = include_str!("../../hooks/notify.sh");
const NOTIFY_PS1: &str = include_str!("../../hooks/notify.ps1");

const STATE_EVENTS: [&str; 4] = ["UserPromptSubmit", "Notification", "Stop", "SessionEnd"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub claude_dir: String,
    pub settings_path: String,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn claude_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".claude"))
}

fn hook_command(dir: &Path) -> String {
    if cfg!(windows) {
        format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -File \"{}\"",
            dir.join("notify.ps1").display()
        )
    } else {
        format!("bash \"{}\"", dir.join("notify.sh").display())
    }
}

/// A hook group is "ours" if any of its commands references our scripts.
fn is_our_group(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hooks| {
            hooks.iter().any(|h| {
                h.get("command")
                    .and_then(Value::as_str)
                    .map(|c| c.contains("notify.sh") || c.contains("notify.ps1"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn our_group(event: &str, command: &str) -> Value {
    if event == "PreToolUse" {
        json!({ "matcher": "Bash", "hooks": [{ "type": "command", "command": command, "timeout": 620 }] })
    } else {
        json!({ "hooks": [{ "type": "command", "command": command }] })
    }
}

/// Merge our five hooks into an existing settings object, replacing any prior
/// Semáforo entries and preserving everything else. Pure and idempotent.
pub fn build_settings(mut existing: Value, command: &str) -> Value {
    if !existing.is_object() {
        existing = json!({});
    }
    let root = existing.as_object_mut().unwrap();
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks = hooks.as_object_mut().unwrap();

    for event in STATE_EVENTS.iter().copied().chain(std::iter::once("PreToolUse")) {
        let mut groups: Vec<Value> = hooks
            .get(event)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|g| !is_our_group(g))
            .collect();
        groups.push(our_group(event, command));
        hooks.insert(event.to_string(), Value::Array(groups));
    }
    existing
}

pub fn install(_app: &AppHandle, token: &str) -> Result<InstallReport, String> {
    let dir = claude_dir().ok_or("não encontrei o diretório home")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    fs::write(dir.join("notify.sh"), NOTIFY_SH).map_err(|e| e.to_string())?;
    fs::write(dir.join("notify.ps1"), NOTIFY_PS1).map_err(|e| e.to_string())?;
    fs::write(dir.join("semaforo.token"), token).map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir.join("notify.sh"), fs::Permissions::from_mode(0o755));
    }

    let settings_path = dir.join("settings.json");
    let existing = fs::read_to_string(&settings_path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(|| json!({}));
    let merged = build_settings(existing, &hook_command(&dir));
    let pretty = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
    fs::write(&settings_path, pretty).map_err(|e| e.to_string())?;

    Ok(InstallReport {
        claude_dir: dir.display().to_string(),
        settings_path: settings_path.display().to_string(),
    })
}

/// True when settings.json already references our hook scripts.
pub fn is_installed() -> bool {
    claude_dir()
        .map(|d| {
            fs::read_to_string(d.join("settings.json"))
                .map(|c| c.contains("notify.sh") || c.contains("notify.ps1"))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Keep an installed setup's token in sync after a regenerate.
pub fn sync_token(token: &str) {
    if let Some(dir) = claude_dir() {
        let path = dir.join("semaforo.token");
        if path.exists() {
            let _ = fs::write(path, token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_all_five_hooks_from_empty() {
        let out = build_settings(json!({}), "bash notify.sh");
        let hooks = out["hooks"].as_object().unwrap();
        for event in ["UserPromptSubmit", "Notification", "Stop", "SessionEnd", "PreToolUse"] {
            assert!(hooks.contains_key(event), "missing {event}");
        }
        assert_eq!(hooks["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(hooks["PreToolUse"][0]["hooks"][0]["timeout"], 620);
        assert_eq!(hooks["Stop"][0]["hooks"][0]["command"], "bash notify.sh");
    }

    #[test]
    fn preserves_unrelated_user_hooks() {
        let existing = json!({
            "model": "opus",
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "echo done" }] }] }
        });
        let out = build_settings(existing, "bash notify.sh");
        assert_eq!(out["model"], "opus");
        let stop = out["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2); // user's echo + ours
        assert_eq!(stop[0]["hooks"][0]["command"], "echo done");
    }

    #[test]
    fn is_idempotent() {
        let once = build_settings(json!({}), "bash notify.sh");
        let twice = build_settings(once.clone(), "bash notify.sh");
        assert_eq!(once, twice);
        assert_eq!(twice["hooks"]["Stop"].as_array().unwrap().len(), 1);
    }
}
