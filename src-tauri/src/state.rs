// Domain model and shared application state.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Working,
    Waiting,
    Ready,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReqKind {
    Perm,
    Ask,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub folder: String,
    pub cwd: String,
    pub container: bool,
    pub state: SessionState,
    pub req_kind: Option<ReqKind>,
    pub cmd: Option<String>,
    pub last_msg: String,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub bind: String,
    pub theme: String,  // "auto" | "light" | "dark"
    pub accent: String, // hex
    pub always_on_top: bool,
    pub autostart: bool,
    pub notify: bool,
    pub reply_perm: bool,
    pub reply_text: bool,
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub win_x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub win_y: Option<i32>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind: "0.0.0.0:7337".into(),
            theme: "light".into(),
            accent: "#C96442".into(),
            always_on_top: true,
            autostart: false,
            notify: true,
            reply_perm: true,
            reply_text: false,
            token: String::new(),
            win_x: None,
            win_y: None,
        }
    }
}

/// Patch sent from the Config UI (only user-editable fields).
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPatch {
    pub bind: Option<String>,
    pub theme: Option<String>,
    pub accent: Option<String>,
    pub always_on_top: Option<bool>,
    pub autostart: Option<bool>,
    pub notify: Option<bool>,
    pub reply_perm: Option<bool>,
    pub reply_text: Option<bool>,
}

#[derive(Serialize, Clone)]
pub struct Snapshot {
    pub sessions: Vec<Session>,
    pub config: Config,
}

/// What the user (or a timeout) decided for a held permission request.
#[derive(Clone, Copy)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Deny => "deny",
            Decision::Ask => "ask",
        }
    }
}

pub struct Inner {
    pub sessions: HashMap<String, Session>,
    pub config: Config,
    /// session_id -> responder for a currently-held PreToolUse request.
    pub pending: HashMap<String, Sender<Decision>>,
    /// Commands the user chose to always allow.
    pub allow_rules: HashSet<String>,
}

impl Inner {
    pub fn snapshot(&self) -> Snapshot {
        let mut sessions: Vec<Session> = self.sessions.values().cloned().collect();
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Snapshot {
            sessions,
            config: self.config.clone(),
        }
    }
}

/// Tauri-managed state. The server thread shares the same `Arc<Mutex<Inner>>`.
pub struct AppState {
    pub inner: std::sync::Arc<Mutex<Inner>>,
    pub server: Mutex<Option<crate::server::ServerHandle>>,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn basename_handles_unix_and_windows() {
        assert_eq!(basename("/home/me/proj"), "proj");
        assert_eq!(basename("/home/me/proj/"), "proj");
        assert_eq!(basename("C:\\dev\\app"), "app");
        assert_eq!(basename("solo"), "solo");
    }

    #[test]
    fn decision_serializes_to_lowercase() {
        assert_eq!(Decision::Allow.as_str(), "allow");
        assert_eq!(Decision::Deny.as_str(), "deny");
        assert_eq!(Decision::Ask.as_str(), "ask");
    }

    #[test]
    fn config_serializes_camel_case() {
        let value = serde_json::to_value(Config::default()).unwrap();
        assert!(value.get("alwaysOnTop").is_some());
        assert!(value.get("replyPerm").is_some());
        assert!(value.get("replyText").is_some());
    }

    #[test]
    fn config_patch_deserializes_camel_case() {
        let patch: ConfigPatch =
            serde_json::from_str(r#"{ "replyText": true, "alwaysOnTop": false }"#).unwrap();
        assert_eq!(patch.reply_text, Some(true));
        assert_eq!(patch.always_on_top, Some(false));
        assert_eq!(patch.bind, None);
    }

    #[test]
    fn snapshot_orders_newest_first() {
        let mut sessions = HashMap::new();
        for (id, at) in [("old", 100i64), ("new", 300), ("mid", 200)] {
            sessions.insert(
                id.to_string(),
                Session {
                    id: id.to_string(),
                    folder: id.to_string(),
                    cwd: String::new(),
                    container: false,
                    state: SessionState::Ready,
                    req_kind: None,
                    cmd: None,
                    last_msg: String::new(),
                    updated_at: at,
                },
            );
        }
        let inner = Inner {
            sessions,
            config: Config::default(),
            pending: HashMap::new(),
            allow_rules: HashSet::new(),
        };
        let ids: Vec<_> = inner.snapshot().sessions.iter().map(|s| s.id.clone()).collect();
        assert_eq!(ids, vec!["new", "mid", "old"]);
    }
}
