mod commands;
mod config;
mod server;
mod state;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{Emitter, Manager, PhysicalPosition, WebviewWindow};

use state::{now_ms, AppState, Inner};

const IDLE_REMOVE_MS: i64 = 15 * 60 * 1000;
const MARGIN: i32 = 16;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        builder = builder.plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));
    }

    builder
        .setup(|app| {
            let handle = app.handle().clone();
            let cfg = config::load(&handle);
            let always_on_top = cfg.always_on_top;
            let autostart = cfg.autostart;
            let bind = cfg.bind.clone();
            let saved = (cfg.win_x, cfg.win_y);

            let inner = Arc::new(Mutex::new(Inner {
                sessions: HashMap::new(),
                config: cfg,
                pending: HashMap::new(),
                allow_rules: HashSet::new(),
            }));

            let server = server::start(&bind, inner.clone(), handle.clone());
            app.manage(AppState {
                inner: inner.clone(),
                server: Mutex::new(server),
            });

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_always_on_top(always_on_top);
                position_window(&window, saved);
                let _ = window.show();
            }

            if autostart {
                commands::apply_autostart(&handle, true);
            }

            spawn_sweeper(handle.clone(), inner);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::respond,
            commands::reply_text,
            commands::get_config,
            commands::set_config,
            commands::regenerate_token,
            commands::reveal_token,
            commands::save_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Place the widget at its saved corner, or default to the bottom-right.
fn position_window(window: &WebviewWindow, saved: (Option<i32>, Option<i32>)) {
    if let (Some(x), Some(y)) = saved {
        let _ = window.set_position(PhysicalPosition::new(x, y));
        return;
    }
    if let (Ok(Some(monitor)), Ok(size)) = (window.current_monitor(), window.outer_size()) {
        let scale = monitor.scale_factor();
        let margin = (MARGIN as f64 * scale) as i32;
        let mpos = monitor.position();
        let msize = monitor.size();
        let x = mpos.x + msize.width as i32 - size.width as i32 - margin;
        let y = mpos.y + msize.height as i32 - size.height as i32 - margin;
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

/// Drop sessions that have gone quiet for a while.
fn spawn_sweeper(app: tauri::AppHandle, inner: Arc<Mutex<Inner>>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(30));
        let changed = {
            let mut g = match inner.lock() {
                Ok(g) => g,
                Err(_) => continue,
            };
            let cutoff = now_ms() - IDLE_REMOVE_MS;
            let before = g.sessions.len();
            let stale: Vec<String> = g
                .sessions
                .iter()
                .filter(|(_, s)| s.updated_at < cutoff)
                .map(|(id, _)| id.clone())
                .collect();
            for id in &stale {
                g.sessions.remove(id);
                g.pending.remove(id);
            }
            before != g.sessions.len()
        };
        if changed {
            if let Ok(g) = inner.lock() {
                let _ = app.emit("snapshot", g.snapshot());
            }
        }
    });
}
