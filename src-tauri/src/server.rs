// HTTP listener reachable from the host and from containers.
//
//   POST /events      state updates from the lifecycle hooks
//   POST /permission   a held PreToolUse request — the response carries the
//                      user's allow/deny decision back to Claude Code
//
// Every request must carry `Authorization: Bearer <token>`.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tiny_http::{Header, Request, Response, Server};

use crate::state::{basename, now_ms, Decision, Inner, Pending, ReqKind, Session, SessionState};

const PERMISSION_TIMEOUT: Duration = Duration::from_secs(600);

/// Per-route cap on request body size. The server is one-thread-per-request, so
/// an authenticated-but-hostile container could otherwise OOM the widget with a
/// few huge concurrent POSTs.
const MAX_PERMISSION_BODY: u64 = 256 * 1024;
const MAX_EVENTS_BODY: u64 = 64 * 1024;

/// Cap on how much of a transcript tail we read for the last assistant message.
const MAX_TRANSCRIPT_TAIL: u64 = 256 * 1024;

pub struct ServerHandle {
    running: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
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
    let _ = req.as_reader().take(max_body_for(&url)).read_to_string(&mut body);
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
    let Some(session_id) = str_field(payload, "session_id").map(str::to_string) else {
        eprintln!("[semaforo] /events without session_id (event={event}) — ignoring");
        let _ = req.respond(json_response(200, json!({ "ok": true })));
        return;
    };
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

    if event == "PostToolUse" {
        // A tool finished. Only un-stick a session stranded in waiting (e.g. a
        // permission answered in the terminal, which has no channel back to the
        // pill). Leave working/ready sessions alone so we don't bump updated_at
        // and flash the row on every tool call.
        if let Some(s) = g.sessions.get_mut(session_id) {
            if s.state == SessionState::Waiting {
                s.state = SessionState::Working;
                s.req_kind = None;
                s.cmd = None;
                s.last_msg = "Voltando ao trabalho…".into();
                s.updated_at = now_ms();
            }
        }
        return None;
    }

    // A held /permission owns this session's prompt; a later Notification must
    // not downgrade its allow/deny buttons into a generic text Ask.
    let has_pending = g.pending.contains_key(session_id);

    let folder: String = if cwd.is_empty() {
        session_id.chars().take(8).collect()
    } else {
        basename(cwd)
    };
    let is_new = !g.sessions.contains_key(session_id);
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
    // Latch the container flag: once a session is known to be in a container, a
    // later host-origin event must not flip the badge back and forth.
    entry.container = entry.container || container;

    let mut became_waiting = None;
    let mut changed = is_new;
    match event {
        "UserPromptSubmit" => {
            entry.state = SessionState::Working;
            entry.req_kind = None;
            entry.cmd = None;
            entry.last_msg = provided_msg.unwrap_or_else(|| "Pensando…".into());
            changed = true;
        }
        "Notification" => {
            if !has_pending {
                entry.state = SessionState::Waiting;
                entry.req_kind = Some(ReqKind::Ask);
                entry.last_msg = provided_msg.unwrap_or_else(|| "Esperando você.".into());
                became_waiting = Some(entry.folder.clone());
                changed = true;
            }
        }
        "Stop" => {
            entry.state = SessionState::Ready;
            entry.req_kind = None;
            entry.cmd = None;
            entry.last_msg = provided_msg
                .or_else(|| last_assistant_message(payload))
                .unwrap_or_else(|| "Terminei o turno.".into());
            changed = true;
        }
        _ => {
            if let Some(msg) = provided_msg {
                entry.last_msg = msg;
                changed = true;
            }
        }
    }
    // Only bump the timestamp on a real change, so the row doesn't flash when a
    // held permission swallows a Notification (or any no-op event arrives).
    if changed {
        entry.updated_at = now_ms();
    }
    became_waiting
}

/// Body cap by route: permission payloads (with tool input) get more headroom
/// than the small lifecycle events.
fn max_body_for(url: &str) -> u64 {
    if url.starts_with("/permission") {
        MAX_PERMISSION_BODY
    } else {
        MAX_EVENTS_BODY
    }
}

fn handle_permission(req: Request, inner: &Arc<Mutex<Inner>>, app: &AppHandle, payload: &Value) {
    let Some(session_id) = str_field(payload, "session_id").map(str::to_string) else {
        eprintln!("[semaforo] /permission without session_id — deferring to Claude Code");
        let _ = req.respond(permission_response(Decision::Ask));
        return;
    };

    // Only sessions in "default" mode are actually prompted by Claude Code. In
    // auto / acceptEdits / bypassPermissions / plan / dontAsk modes it wouldn't
    // ask, so defer instead of gating — otherwise an auto-mode session that
    // never wanted a prompt shows a false 🔴 "te esperando".
    match str_field(payload, "permission_mode") {
        Some("default") | None => {}
        Some(_) => {
            let _ = req.respond(permission_response(Decision::Ask));
            return;
        }
    }

    let cwd = str_field(payload, "cwd").unwrap_or("").to_string();
    let container = is_container(&req, payload);
    let tool_name = str_field(payload, "tool_name").unwrap_or("Tool");
    let tool_input = payload.get("tool_input");
    // `key` is the precise rule identity (stored/matched); `label` is the short
    // friendly text shown on the pill. They differ on purpose: the key keeps the
    // full path / arguments so "always" can't over-grant, the label truncates.
    let key = describe_key(tool_name, tool_input);
    let label = describe_label(tool_name, tool_input);

    let rx = {
        let mut g = match inner.lock() {
            Ok(g) => g,
            Err(_) => {
                let _ = req.respond(json_response(500, json!({ "error": "state" })));
                return;
            }
        };

        // Honor an existing "always allow" rule without bothering the user.
        if g.allow_rules.contains(&key) {
            upsert_working(&mut g, &session_id, &cwd, container, format!("Rodando {label}…"));
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
            cmd: Some(label.clone()),
            last_msg: String::new(),
            updated_at: now_ms(),
        });
        entry.folder = folder.clone();
        if !cwd.is_empty() {
            entry.cwd = cwd.clone();
        }
        entry.container = entry.container || container;
        entry.state = SessionState::Waiting;
        entry.req_kind = Some(ReqKind::Perm);
        entry.cmd = Some(label.clone());
        entry.last_msg = if tool_name == "Bash" {
            "Quer rodar um comando".to_string()
        } else {
            format!("Quer usar {tool_name}")
        };
        entry.updated_at = now_ms();

        let (tx, rx) = channel::<Decision>();
        // Replaces any prior responder; the displaced one wakes with Disconnected.
        g.pending.insert(session_id.clone(), Pending { tx, rule_key: key });
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

    let decision = match rx.recv_timeout(PERMISSION_TIMEOUT) {
        Ok(decision) => decision,
        Err(RecvTimeoutError::Timeout) => {
            // Nobody answered in time. Reset the session so the pill stops showing
            // a phantom 🔴, and tell the user to answer in the terminal instead.
            let reset = inner.lock().map(|mut g| reset_timed_out(&mut g, &session_id)).unwrap_or(false);
            if reset {
                emit(app, inner);
            }
            Decision::Ask
        }
        // Displaced by a newer /permission for the same session (it dropped our
        // sender). The replacement now owns the session — don't touch it.
        Err(RecvTimeoutError::Disconnected) => Decision::Ask,
    };
    let _ = req.respond(permission_response(decision));
}

/// On a permission timeout, drop our held slot and un-stick the session — but
/// only if it's still showing *our* held permission (nobody answered, no newer
/// request replaced it). Returns whether anything changed (worth an emit).
fn reset_timed_out(g: &mut Inner, session_id: &str) -> bool {
    if g.pending.remove(session_id).is_none() {
        return false; // already answered or replaced
    }
    let Some(s) = g.sessions.get_mut(session_id) else { return false };
    if s.state == SessionState::Waiting && s.req_kind == Some(ReqKind::Perm) {
        s.state = SessionState::Working;
        s.req_kind = None;
        s.cmd = None;
        s.last_msg = "Tempo esgotado — responda no terminal.".into();
        s.updated_at = now_ms();
        true
    } else {
        false
    }
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

/// A short, human label for the tool/command awaiting permission. Truncated for
/// display only — never key an allow-rule off this (use `describe_key`).
fn describe_label(tool_name: &str, tool_input: Option<&Value>) -> String {
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

/// The precise identity of a tool call, used to store and match "always allow"
/// rules. Unlike the label it is never truncated and keeps the full path (file
/// tools) or the exact, canonically-ordered arguments (Bash, MCP, everything
/// else), so approving one call can't blanket-approve a different file or payload.
fn describe_key(tool_name: &str, tool_input: Option<&Value>) -> String {
    let input = tool_input.unwrap_or(&Value::Null);
    match tool_name {
        "Bash" => input
            .get("command")
            .and_then(Value::as_str)
            .map(|c| format!("Bash {c}")),
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "Read" => input
            .get("file_path")
            .or_else(|| input.get("path"))
            .and_then(Value::as_str)
            .map(|p| format!("{tool_name} {p}")),
        _ => match input {
            Value::Null => None,
            other => Some(format!("{tool_name} {other}")),
        },
    }
    .unwrap_or_else(|| tool_name.to_string())
}

/// Best-effort: the last assistant text from a host-readable transcript. The
/// path comes from the (authenticated) hook body, so it's confined to
/// `~/.claude/projects/` to avoid reading arbitrary files, and only its tail is
/// read so a long transcript can't blow up memory.
fn last_assistant_message(payload: &Value) -> Option<String> {
    let path = str_field(payload, "transcript_path")?;
    let home = crate::setup::home_dir()?;
    if !transcript_allowed(&home, path) {
        return None;
    }
    let content = read_tail(path, MAX_TRANSCRIPT_TAIL)?;
    parse_last_assistant(&content)
}

/// Confine transcript reads to `~/.claude/projects/`, rejecting any `..` so the
/// path can't escape the directory.
fn transcript_allowed(home: &Path, path: &str) -> bool {
    let base = home.join(".claude").join("projects");
    !path.contains("..") && Path::new(path).starts_with(&base)
}

/// Read at most the last `max_bytes` of a file. UTF-8 is decoded lossily so a
/// byte-boundary cut in the middle of a character can't fail the read; a partial
/// leading line simply won't parse as JSON and is skipped downstream.
fn read_tail(path: &str, max_bytes: u64) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if len > max_bytes {
        file.seek(SeekFrom::Start(len - max_bytes)).ok()?;
    }
    let mut bytes = Vec::new();
    file.take(max_bytes).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The last non-empty assistant text block from JSONL transcript content.
fn parse_last_assistant(content: &str) -> Option<String> {
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
    fn label_bash_uses_command() {
        let input = json!({ "command": "npm run migrate:prod" });
        assert_eq!(describe_label("Bash", Some(&input)), "npm run migrate:prod");
    }

    #[test]
    fn label_file_tool_uses_basename() {
        let input = json!({ "file_path": "/home/me/proj/.env" });
        assert_eq!(describe_label("Write", Some(&input)), "Write .env");
    }

    #[test]
    fn label_falls_back_to_tool_name() {
        assert_eq!(describe_label("Glob", Some(&json!({}))), "Glob");
        assert_eq!(describe_label("Glob", None), "Glob");
    }

    #[test]
    fn label_truncates_long_commands() {
        let long = "x".repeat(300);
        let described = describe_label("Bash", Some(&json!({ "command": long })));
        assert!(described.chars().count() <= 120);
        assert!(described.ends_with('…'));
    }

    #[test]
    fn key_keeps_full_path_so_different_dirs_dont_collide() {
        // Approving Write to /a/.env must not auto-allow Write to /b/.env.
        let a = describe_key("Write", Some(&json!({ "file_path": "/a/.env" })));
        let b = describe_key("Write", Some(&json!({ "file_path": "/b/.env" })));
        assert_eq!(a, "Write /a/.env");
        assert_ne!(a, b);
    }

    #[test]
    fn key_binds_mcp_calls_to_their_arguments() {
        // "Always" on an MCP tool must not blanket-approve every future call.
        let a = describe_key("mcp__github__create_pr", Some(&json!({ "title": "x" })));
        let b = describe_key("mcp__github__create_pr", Some(&json!({ "title": "y" })));
        assert_ne!(a, b);
        // Bare tool name only when there are no args at all.
        assert_eq!(describe_key("mcp__github__whoami", None), "mcp__github__whoami");
    }

    #[test]
    fn key_does_not_truncate_bash() {
        // Two commands sharing a 117-char prefix must not collide on one rule.
        let long = "x".repeat(300);
        let key = describe_key("Bash", Some(&json!({ "command": long.clone() })));
        assert!(key.contains(&long));
    }

    #[test]
    fn max_body_caps_permission_higher_than_events() {
        assert_eq!(max_body_for("/permission"), MAX_PERMISSION_BODY);
        assert_eq!(max_body_for("/events"), MAX_EVENTS_BODY);
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
    fn parses_last_assistant_message_from_transcript() {
        let lines = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"oi\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Refiz a hero.\"}]}}\n",
        );
        assert_eq!(parse_last_assistant(lines).as_deref(), Some("Refiz a hero."));
    }

    #[test]
    fn transcript_allowed_only_under_claude_projects() {
        let home = Path::new("/home/me");
        assert!(transcript_allowed(home, "/home/me/.claude/projects/foo/s.jsonl"));
        assert!(!transcript_allowed(home, "/etc/passwd"));
        assert!(!transcript_allowed(home, "/home/me/.ssh/id_rsa"));
        // No escaping the directory with traversal.
        assert!(!transcript_allowed(home, "/home/me/.claude/projects/../../etc/passwd"));
    }

    #[test]
    fn read_tail_returns_only_the_last_bytes() {
        let path = std::env::temp_dir().join(format!("semaforo_tail_{}.txt", std::process::id()));
        fs::write(&path, "0123456789").unwrap();
        assert_eq!(read_tail(path.to_str().unwrap(), 4).as_deref(), Some("6789"));
        assert_eq!(read_tail(path.to_str().unwrap(), 100).as_deref(), Some("0123456789"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn post_tool_use_unsticks_waiting_session() {
        // A permission left the session waiting (answered in the terminal, with
        // no channel back to the pill). Once the tool runs, it's working again.
        let mut g = inner();
        apply_event(&mut g, "Notification", "s1", "/x/api", false, Some("posso?".into()), &Value::Null);
        assert!(matches!(g.sessions.get("s1").unwrap().state, SessionState::Waiting));

        apply_event(&mut g, "PostToolUse", "s1", "/x/api", false, None, &Value::Null);
        let s = g.sessions.get("s1").unwrap();
        assert!(matches!(s.state, SessionState::Working));
        assert!(s.req_kind.is_none());
    }

    #[test]
    fn post_tool_use_leaves_non_waiting_untouched() {
        // A tool finishing mid-turn must not bump updated_at (which would flash
        // the row) when the session isn't stuck waiting.
        let mut g = inner();
        apply_event(&mut g, "Stop", "s1", "/x/api", false, Some("feito".into()), &Value::Null);
        g.sessions.get_mut("s1").unwrap().updated_at = 123;

        apply_event(&mut g, "PostToolUse", "s1", "/x/api", false, None, &Value::Null);
        let s = g.sessions.get("s1").unwrap();
        assert!(matches!(s.state, SessionState::Ready));
        assert_eq!(s.updated_at, 123);
    }

    #[test]
    fn notification_keeps_held_permission() {
        // A /permission is held for the session (rich allow/deny prompt). A
        // Notification arriving for the same session must not clobber it into a
        // generic text Ask, which would replace the buttons with a text field.
        let mut g = inner();
        g.sessions.insert(
            "s1".into(),
            Session {
                id: "s1".into(),
                folder: "api".into(),
                cwd: "/x/api".into(),
                container: false,
                state: SessionState::Waiting,
                req_kind: Some(ReqKind::Perm),
                cmd: Some("rm -rf x".into()),
                last_msg: "Quer rodar um comando".into(),
                updated_at: 1,
            },
        );
        let (tx, _rx) = channel::<Decision>();
        g.pending.insert("s1".into(), Pending { tx, rule_key: "Bash rm -rf x".into() });

        apply_event(&mut g, "Notification", "s1", "/x/api", false, Some("posso?".into()), &Value::Null);
        let s = g.sessions.get("s1").unwrap();
        assert!(matches!(s.req_kind, Some(ReqKind::Perm)));
        assert_eq!(s.cmd.as_deref(), Some("rm -rf x"));
    }

    #[test]
    fn notification_with_held_permission_does_not_flash() {
        // A Notification swallowed by a held permission must not bump updated_at,
        // or the row flashes with no semantic change.
        let mut g = inner();
        g.sessions.insert(
            "s1".into(),
            Session {
                id: "s1".into(),
                folder: "api".into(),
                cwd: "/x/api".into(),
                container: false,
                state: SessionState::Waiting,
                req_kind: Some(ReqKind::Perm),
                cmd: Some("rm -rf x".into()),
                last_msg: "Quer rodar um comando".into(),
                updated_at: 42,
            },
        );
        let (tx, _rx) = channel::<Decision>();
        g.pending.insert("s1".into(), Pending { tx, rule_key: "Bash rm -rf x".into() });

        apply_event(&mut g, "Notification", "s1", "/x/api", false, Some("posso?".into()), &Value::Null);
        assert_eq!(g.sessions.get("s1").unwrap().updated_at, 42);
    }

    #[test]
    fn container_flag_latches_true() {
        // A host-origin event after a container one must not flip the badge back.
        let mut g = inner();
        apply_event(&mut g, "UserPromptSubmit", "s1", "/x/api", true, None, &Value::Null);
        apply_event(&mut g, "Stop", "s1", "/x/api", false, None, &Value::Null);
        assert!(g.sessions.get("s1").unwrap().container);
    }

    #[test]
    fn subagent_stop_does_not_mark_ready() {
        // SubagentStop isn't registered as a hook, so it shouldn't be wired to
        // mark the parent ready (which would lie while subagents still run).
        let mut g = inner();
        apply_event(&mut g, "UserPromptSubmit", "s1", "/x/api", false, None, &Value::Null);
        apply_event(&mut g, "SubagentStop", "s1", "/x/api", false, None, &Value::Null);
        assert!(matches!(g.sessions.get("s1").unwrap().state, SessionState::Working));
    }

    #[test]
    fn timeout_resets_stuck_permission() {
        let mut g = inner();
        g.sessions.insert(
            "s1".into(),
            Session {
                id: "s1".into(),
                folder: "api".into(),
                cwd: "/x/api".into(),
                container: false,
                state: SessionState::Waiting,
                req_kind: Some(ReqKind::Perm),
                cmd: Some("rm -rf x".into()),
                last_msg: "Quer rodar um comando".into(),
                updated_at: 1,
            },
        );
        let (tx, _rx) = channel::<Decision>();
        g.pending.insert("s1".into(), Pending { tx, rule_key: "Bash rm -rf x".into() });

        assert!(reset_timed_out(&mut g, "s1"));
        let s = g.sessions.get("s1").unwrap();
        assert!(matches!(s.state, SessionState::Working));
        assert!(s.req_kind.is_none());
        assert!(s.cmd.is_none());
        assert!(s.updated_at > 1);
        assert!(!g.pending.contains_key("s1"));
    }

    #[test]
    fn timeout_is_a_noop_when_already_answered() {
        // No pending slot (someone clicked, or a newer request replaced ours):
        // resetting would clobber whatever now owns the session.
        let mut g = inner();
        apply_event(&mut g, "UserPromptSubmit", "s1", "/x/api", false, None, &Value::Null);
        assert!(!reset_timed_out(&mut g, "s1"));
        assert!(matches!(g.sessions.get("s1").unwrap().state, SessionState::Working));
    }
}
