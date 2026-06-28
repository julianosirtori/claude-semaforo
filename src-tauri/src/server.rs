// HTTP listener reachable from the host and from containers.
//
//   POST /events      state updates from the lifecycle hooks
//   POST /permission   a held PreToolUse request — the response carries the
//                      user's allow/deny decision back to Claude Code
//
// Every request must carry `Authorization: Bearer <token>`.

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tiny_http::{Header, Request, Response, Server};

use crate::state::{basename, now_ms, Decision, Inner, ReqKind, Session, SessionState};

const PERMISSION_TIMEOUT: Duration = Duration::from_secs(600);

pub struct ServerHandle {
    running: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    pub bind: String,
}

impl ServerHandle {
    pub fn stop(mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Bind and start serving. Returns `None` if the address can't be bound
/// (e.g. the port is taken) — the app keeps running without a listener.
pub fn start(bind: &str, inner: Arc<Mutex<Inner>>, app: AppHandle) -> Option<ServerHandle> {
    let server = match Server::http(bind) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("[semaforo] could not bind {bind}: {e}");
            return None;
        }
    };
    let running = Arc::new(AtomicBool::new(true));
    let bind_owned = bind.to_string();

    let join = thread::spawn({
        let running = running.clone();
        move || {
            while running.load(Ordering::SeqCst) {
                match server.recv_timeout(Duration::from_millis(400)) {
                    Ok(Some(req)) => {
                        let inner = inner.clone();
                        let app = app.clone();
                        thread::spawn(move || handle(req, inner, app));
                    }
                    Ok(None) => {} // timeout — re-check the running flag
                    Err(_) => break,
                }
            }
        }
    });

    Some(ServerHandle {
        running,
        join: Some(join),
        bind: bind_owned,
    })
}

fn header<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    for h in req.headers() {
        if h.field.as_str().as_str().eq_ignore_ascii_case(name) {
            return Some(h.value.as_str());
        }
    }
    None
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn authorized(req: &Request, token: &str) -> bool {
    let Some(auth) = header(req, "Authorization") else { return false };
    let presented = auth.strip_prefix("Bearer ").unwrap_or(auth).trim();
    !token.is_empty() && constant_time_eq(presented.as_bytes(), token.as_bytes())
}

fn is_container(req: &Request, body: &Value) -> bool {
    if header(req, "X-Semaforo-Container").map(|v| v == "1").unwrap_or(false) {
        return true;
    }
    if body.get("container").and_then(Value::as_bool).unwrap_or(false) {
        return true;
    }
    match req.remote_addr() {
        Some(addr) => !addr.ip().is_loopback(),
        None => false,
    }
}

fn json_response(code: u16, body: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    Response::from_string(body.to_string())
        .with_status_code(code)
        .with_header(header)
}

fn emit(app: &AppHandle, inner: &Arc<Mutex<Inner>>) {
    if let Ok(g) = inner.lock() {
        let _ = app.emit("snapshot", g.snapshot());
    }
}

fn notify_if_enabled(app: &AppHandle, inner: &Arc<Mutex<Inner>>, folder: &str) {
    let enabled = inner.lock().map(|g| g.config.notify).unwrap_or(false);
    if enabled {
        let _ = app
            .notification()
            .builder()
            .title(format!("{folder} te esperando"))
            .body("Claude Semáforo")
            .show();
    }
}

fn handle(mut req: Request, inner: Arc<Mutex<Inner>>, app: AppHandle) {
    let token = inner.lock().map(|g| g.config.token.clone()).unwrap_or_default();
    if !authorized(&req, &token) {
        let _ = req.respond(json_response(401, json!({ "error": "unauthorized" })));
        return;
    }

    let url = req.url().to_string();
    let mut body = String::new();
    let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
    let payload: Value = serde_json::from_str(&body).unwrap_or(Value::Null);

    if url.starts_with("/events") {
        handle_events(req, &inner, &app, &payload);
    } else if url.starts_with("/permission") {
        handle_permission(req, &inner, &app, &payload);
    } else {
        let _ = req.respond(json_response(404, json!({ "error": "not found" })));
    }
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn handle_events(req: Request, inner: &Arc<Mutex<Inner>>, app: &AppHandle, payload: &Value) {
    let event = str_field(payload, "hook_event_name").unwrap_or("").to_string();
    let session_id = str_field(payload, "session_id").unwrap_or("unknown").to_string();
    let cwd = str_field(payload, "cwd").unwrap_or("").to_string();
    let container = is_container(&req, payload);
    let provided_msg = str_field(payload, "message").map(|s| s.to_string());

    let became_waiting = {
        let mut g = match inner.lock() {
            Ok(g) => g,
            Err(_) => {
                let _ = req.respond(json_response(500, json!({ "error": "state" })));
                return;
            }
        };
        apply_event(&mut g, &event, &session_id, &cwd, container, provided_msg, payload)
    };

    emit(app, inner);
    if let Some(folder) = became_waiting {
        notify_if_enabled(app, inner, &folder);
    }
    let _ = req.respond(json_response(200, json!({ "ok": true })));
}

/// Apply a lifecycle event to the session map. Returns the folder name when the
/// session transitions into the waiting state, so the caller can notify.
fn apply_event(
    g: &mut Inner,
    event: &str,
    session_id: &str,
    cwd: &str,
    container: bool,
    provided_msg: Option<String>,
    payload: &Value,
) -> Option<String> {
    if event == "SessionEnd" {
        g.sessions.remove(session_id);
        g.pending.remove(session_id);
        return None;
    }

    let folder: String = if cwd.is_empty() {
        session_id.chars().take(8).collect()
    } else {
        basename(cwd)
    };
    let entry = g.sessions.entry(session_id.to_string()).or_insert_with(|| Session {
        id: session_id.to_string(),
        folder: folder.clone(),
        cwd: cwd.to_string(),
        container,
        state: SessionState::Working,
        req_kind: None,
        cmd: None,
        last_msg: String::new(),
        updated_at: now_ms(),
    });
    if !cwd.is_empty() {
        entry.folder = folder.clone();
        entry.cwd = cwd.to_string();
    }
    entry.container = container;
    entry.updated_at = now_ms();

    let mut became_waiting = None;
    match event {
        "UserPromptSubmit" => {
            entry.state = SessionState::Working;
            entry.req_kind = None;
            entry.cmd = None;
            entry.last_msg = provided_msg.unwrap_or_else(|| "Pensando…".into());
        }
        "Notification" => {
            entry.state = SessionState::Waiting;
            entry.req_kind = Some(ReqKind::Ask);
            entry.last_msg = provided_msg.unwrap_or_else(|| "Esperando você.".into());
            became_waiting = Some(entry.folder.clone());
        }
        "Stop" | "SubagentStop" => {
            entry.state = SessionState::Ready;
            entry.req_kind = None;
            entry.cmd = None;
            entry.last_msg = provided_msg
                .or_else(|| last_assistant_message(payload))
                .unwrap_or_else(|| "Terminei o turno.".into());
        }
        _ => {
            if let Some(msg) = provided_msg {
                entry.last_msg = msg;
            }
        }
    }
    became_waiting
}

fn handle_permission(req: Request, inner: &Arc<Mutex<Inner>>, app: &AppHandle, payload: &Value) {
    let session_id = str_field(payload, "session_id").unwrap_or("unknown").to_string();
    let cwd = str_field(payload, "cwd").unwrap_or("").to_string();
    let container = is_container(&req, payload);
    let tool_name = str_field(payload, "tool_name").unwrap_or("Tool");
    let cmd = describe_tool(tool_name, payload.get("tool_input"));

    let rx = {
        let mut g = match inner.lock() {
            Ok(g) => g,
            Err(_) => {
                let _ = req.respond(json_response(500, json!({ "error": "state" })));
                return;
            }
        };

        // Honor an existing "always allow" rule without bothering the user.
        if g.allow_rules.contains(&cmd) {
            upsert_working(&mut g, &session_id, &cwd, container, format!("Rodando {cmd}…"));
            drop(g);
            emit(app, inner);
            let _ = req.respond(permission_response(Decision::Allow));
            return;
        }

        // Pill answering disabled → let the native prompt handle it.
        if !g.config.reply_perm {
            let _ = req.respond(permission_response(Decision::Ask));
            return;
        }

        let folder = if cwd.is_empty() { basename(&session_id) } else { basename(&cwd) };
        let entry = g.sessions.entry(session_id.clone()).or_insert_with(|| Session {
            id: session_id.clone(),
            folder: folder.clone(),
            cwd: cwd.clone(),
            container,
            state: SessionState::Waiting,
            req_kind: Some(ReqKind::Perm),
            cmd: Some(cmd.clone()),
            last_msg: String::new(),
            updated_at: now_ms(),
        });
        entry.folder = folder.clone();
        if !cwd.is_empty() {
            entry.cwd = cwd.clone();
        }
        entry.container = container;
        entry.state = SessionState::Waiting;
        entry.req_kind = Some(ReqKind::Perm);
        entry.cmd = Some(cmd.clone());
        entry.last_msg = format!("Permissão: {tool_name}");
        entry.updated_at = now_ms();

        let (tx, rx) = channel::<Decision>();
        g.pending.insert(session_id.clone(), tx); // replaces any prior responder
        rx
    };

    emit(app, inner);
    {
        let folder = inner
            .lock()
            .ok()
            .and_then(|g| g.sessions.get(&session_id).map(|s| s.folder.clone()))
            .unwrap_or_default();
        notify_if_enabled(app, inner, &folder);
    }

    let decision = rx.recv_timeout(PERMISSION_TIMEOUT).unwrap_or(Decision::Ask);
    if let Decision::Ask = decision {
        // Timed out: drop our responder slot so it doesn't linger.
        if let Ok(mut g) = inner.lock() {
            g.pending.remove(&session_id);
        }
    }
    let _ = req.respond(permission_response(decision));
}

fn upsert_working(g: &mut Inner, session_id: &str, cwd: &str, container: bool, msg: String) {
    let folder = if cwd.is_empty() { basename(session_id) } else { basename(cwd) };
    let entry = g.sessions.entry(session_id.to_string()).or_insert_with(|| Session {
        id: session_id.to_string(),
        folder: folder.clone(),
        cwd: cwd.to_string(),
        container,
        state: SessionState::Working,
        req_kind: None,
        cmd: None,
        last_msg: String::new(),
        updated_at: now_ms(),
    });
    entry.state = SessionState::Working;
    entry.req_kind = None;
    entry.cmd = None;
    entry.last_msg = msg;
    entry.container = container;
    entry.updated_at = now_ms();
}

fn permission_response(decision: Decision) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(
        200,
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": decision.as_str(),
                "permissionDecisionReason": "respondido pelo Claude Semáforo",
            }
        }),
    )
}

/// A short, human label for the tool/command awaiting permission.
fn describe_tool(tool_name: &str, tool_input: Option<&Value>) -> String {
    let input = tool_input.unwrap_or(&Value::Null);
    let raw = match tool_name {
        "Bash" => input.get("command").and_then(Value::as_str).map(str::to_string),
        _ => input
            .get("file_path")
            .or_else(|| input.get("path"))
            .and_then(Value::as_str)
            .map(|p| format!("{tool_name} {}", basename(p))),
    }
    .unwrap_or_else(|| tool_name.to_string());

    if raw.chars().count() > 120 {
        let truncated: String = raw.chars().take(117).collect();
        format!("{truncated}…")
    } else {
        raw
    }
}

/// Best-effort: the last assistant text from a host-readable transcript.
fn last_assistant_message(payload: &Value) -> Option<String> {
    let path = str_field(payload, "transcript_path")?;
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines().rev().take(80) {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let blocks = v.get("message").and_then(|m| m.get("content")).and_then(Value::as_array);
        if let Some(blocks) = blocks {
            for b in blocks {
                if b.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(text) = b.get("text").and_then(Value::as_str) {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            let one_line: String = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
                            return Some(clip(&one_line, 140));
                        }
                    }
                }
            }
        }
    }
    None
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let head: String = s.chars().take(max - 1).collect();
        format!("{head}…")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Config;
    use std::collections::{HashMap, HashSet};

    fn inner() -> Inner {
        Inner {
            sessions: HashMap::new(),
            config: Config::default(),
            pending: HashMap::new(),
            allow_rules: HashSet::new(),
        }
    }

    #[test]
    fn describe_bash_uses_command() {
        let input = json!({ "command": "npm run migrate:prod" });
        assert_eq!(describe_tool("Bash", Some(&input)), "npm run migrate:prod");
    }

    #[test]
    fn describe_file_tool_uses_basename() {
        let input = json!({ "file_path": "/home/me/proj/.env" });
        assert_eq!(describe_tool("Write", Some(&input)), "Write .env");
    }

    #[test]
    fn describe_falls_back_to_tool_name() {
        assert_eq!(describe_tool("Glob", Some(&json!({}))), "Glob");
        assert_eq!(describe_tool("Glob", None), "Glob");
    }

    #[test]
    fn describe_truncates_long_commands() {
        let long = "x".repeat(300);
        let described = describe_tool("Bash", Some(&json!({ "command": long })));
        assert!(described.chars().count() <= 120);
        assert!(described.ends_with('…'));
    }

    #[test]
    fn constant_time_eq_matches_and_rejects() {
        assert!(constant_time_eq(b"csf_secret", b"csf_secret"));
        assert!(!constant_time_eq(b"csf_secret", b"csf_secrxt"));
        assert!(!constant_time_eq(b"short", b"longer-token"));
    }

    #[test]
    fn user_prompt_marks_working() {
        let mut g = inner();
        apply_event(&mut g, "UserPromptSubmit", "s1", "/home/me/proj", false, None, &Value::Null);
        let s = g.sessions.get("s1").unwrap();
        assert!(matches!(s.state, SessionState::Working));
        assert_eq!(s.folder, "proj");
        assert_eq!(s.last_msg, "Pensando…");
    }

    #[test]
    fn notification_marks_waiting_and_returns_folder() {
        let mut g = inner();
        let folder = apply_event(&mut g, "Notification", "s1", "/x/api-gateway", false, Some("posso?".into()), &Value::Null);
        assert_eq!(folder.as_deref(), Some("api-gateway"));
        let s = g.sessions.get("s1").unwrap();
        assert!(matches!(s.state, SessionState::Waiting));
        assert!(matches!(s.req_kind, Some(ReqKind::Ask)));
        assert_eq!(s.last_msg, "posso?");
    }

    #[test]
    fn stop_marks_ready_with_message() {
        let mut g = inner();
        apply_event(&mut g, "UserPromptSubmit", "s1", "/x/api", false, None, &Value::Null);
        apply_event(&mut g, "Stop", "s1", "/x/api", false, Some("feito".into()), &Value::Null);
        let s = g.sessions.get("s1").unwrap();
        assert!(matches!(s.state, SessionState::Ready));
        assert_eq!(s.last_msg, "feito");
    }

    #[test]
    fn session_end_removes_session() {
        let mut g = inner();
        apply_event(&mut g, "UserPromptSubmit", "s1", "/x", false, None, &Value::Null);
        apply_event(&mut g, "SessionEnd", "s1", "", false, None, &Value::Null);
        assert!(g.sessions.get("s1").is_none());
    }

    #[test]
    fn container_flag_is_kept() {
        let mut g = inner();
        apply_event(&mut g, "UserPromptSubmit", "s1", "/x/api", true, None, &Value::Null);
        assert!(g.sessions.get("s1").unwrap().container);
    }

    #[test]
    fn reads_last_assistant_message_from_transcript() {
        let path = std::env::temp_dir().join(format!("semaforo_test_{}.jsonl", std::process::id()));
        let lines = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"oi\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Refiz a hero.\"}]}}\n",
        );
        fs::write(&path, lines).unwrap();
        let payload = json!({ "transcript_path": path.to_string_lossy() });
        assert_eq!(last_assistant_message(&payload).as_deref(), Some("Refiz a hero."));
        let _ = fs::remove_file(&path);
    }
}
