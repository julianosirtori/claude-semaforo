// HTTP listener reachable from the host and from containers.
//
//   POST /events      state updates from the lifecycle hooks
//
// Every request must carry `Authorization: Bearer <token>`.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tiny_http::{Header, Request, Response, Server};

use crate::state::{basename, now_ms, Inner, Session, SessionState};

/// Cap on request body size. The server is one-thread-per-request, so an
/// authenticated-but-hostile container could otherwise OOM the widget with a few
/// huge concurrent POSTs.
const MAX_BODY: u64 = 64 * 1024;

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
    let _ = req.as_reader().take(MAX_BODY).read_to_string(&mut body);
    let payload: Value = serde_json::from_str(&body).unwrap_or(Value::Null);

    if url.starts_with("/events") {
        handle_events(req, &inner, &app, &payload);
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
        return None;
    }

    if event == "PostToolUse" {
        // A tool finished. Only un-stick a session stranded in waiting (e.g. a
        // permission answered in the terminal). Leave working/ready sessions
        // alone so we don't bump updated_at and flash the row on every tool call.
        if let Some(s) = g.sessions.get_mut(session_id) {
            if s.state == SessionState::Waiting {
                s.state = SessionState::Working;
                s.last_msg = "Voltando ao trabalho…".into();
                s.updated_at = now_ms();
            }
        }
        return None;
    }

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
            entry.last_msg = provided_msg.unwrap_or_else(|| "Pensando…".into());
            changed = true;
        }
        "Notification" => {
            entry.state = SessionState::Waiting;
            entry.last_msg = provided_msg.unwrap_or_else(|| "Esperando você.".into());
            became_waiting = Some(entry.folder.clone());
            changed = true;
        }
        "Stop" => {
            entry.state = SessionState::Ready;
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
    // Only bump the timestamp on a real change, so the row doesn't flash on a
    // no-op event.
    if changed {
        entry.updated_at = now_ms();
    }
    became_waiting
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
    use std::collections::HashMap;

    fn inner() -> Inner {
        Inner {
            sessions: HashMap::new(),
            config: Config::default(),
        }
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
        // A Notification left the session waiting. Once a tool runs, it's working.
        let mut g = inner();
        apply_event(&mut g, "Notification", "s1", "/x/api", false, Some("posso?".into()), &Value::Null);
        assert!(matches!(g.sessions.get("s1").unwrap().state, SessionState::Waiting));

        apply_event(&mut g, "PostToolUse", "s1", "/x/api", false, None, &Value::Null);
        assert!(matches!(g.sessions.get("s1").unwrap().state, SessionState::Working));
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
}
