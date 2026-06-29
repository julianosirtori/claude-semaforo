mod commands;
mod config;
mod server;
mod setup;
mod state;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
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

            let panel_open = Arc::new(AtomicBool::new(false));
            let server = server::start(&bind, inner.clone(), handle.clone());
            app.manage(AppState {
                inner: inner.clone(),
                server: Mutex::new(server),
                panel_open: panel_open.clone(),
            });

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_always_on_top(always_on_top);
                position_window(&window, saved);
                let _ = window.show();
            }

            // The window never resizes (no transparent-resize flicker); instead a
            // poller makes everything but the pill corner click-through.
            spawn_clickthrough(handle.clone(), panel_open);

            // The window has no taskbar entry, so the tray is the way to quit.
            if let Err(e) = build_tray(app) {
                eprintln!("[semaforo] tray setup failed: {e}");
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
            commands::set_panel_open,
            commands::regenerate_token,
            commands::reveal_token,
            commands::save_window,
            commands::install_hooks,
            commands::hooks_installed,
            commands::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Place the widget at its saved corner (clamped on-screen), or default to the
/// bottom-right. The window is a fixed panel-sized rect; only the bottom-right
/// pill is opaque, the rest is click-through.
fn position_window(window: &WebviewWindow, saved: (Option<i32>, Option<i32>)) {
    let Ok(size) = window.outer_size() else { return };
    // primary_monitor works before the window is shown; current_monitor doesn't,
    // and without a monitor the clamp below is skipped and a stale saved corner
    // can push the (now larger) window off-screen.
    let monitor = window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| window.current_monitor().ok().flatten());

    let (mut x, mut y) = match saved {
        (Some(sx), Some(sy)) => (sx, sy),
        _ => match &monitor {
            Some(m) => {
                let scale = m.scale_factor();
                let margin = (MARGIN as f64 * scale) as i32;
                (
                    m.position().x + m.size().width as i32 - size.width as i32 - margin,
                    m.position().y + m.size().height as i32 - size.height as i32 - margin,
                )
            }
            None => return,
        },
    };

    // Keep it on-screen — the window size differs from older saved corners.
    if let Some(m) = &monitor {
        let mx = m.position().x;
        let my = m.position().y;
        let max_x = (mx + m.size().width as i32 - size.width as i32).max(mx);
        let max_y = (my + m.size().height as i32 - size.height as i32).max(my);
        x = x.clamp(mx, max_x);
        y = y.clamp(my, max_y);
    }
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Bottom-right square (logical px, scaled by DPI at runtime) that stays
/// clickable while collapsed — covers the pill (74px from the corner) + margin.
const PILL_HIT: f64 = 96.0;

/// Poll the cursor and toggle window-wide click-through: the whole window is
/// interactive while the panel is open; collapsed, only the pill corner is, so
/// clicks land on the apps behind the transparent area.
fn spawn_clickthrough(app: tauri::AppHandle, panel_open: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut last: Option<bool> = None;
        loop {
            std::thread::sleep(Duration::from_millis(60));
            let Some(window) = app.get_webview_window("main") else { continue };
            let ignore = !pointer_interactive(&window, panel_open.load(Ordering::Relaxed));
            if last != Some(ignore) {
                let _ = window.set_ignore_cursor_events(ignore);
                last = Some(ignore);
            }
        }
    });
}

/// Whether the cursor is over an interactive region. Fail-safe: any missing
/// reading returns `true`, so a hiccup never traps the user's clicks.
fn pointer_interactive(window: &WebviewWindow, open: bool) -> bool {
    if open {
        return true;
    }
    let (Ok(pos), Ok(size), Ok(cursor), Ok(scale)) = (
        window.outer_position(),
        window.outer_size(),
        window.cursor_position(),
        window.scale_factor(),
    ) else {
        return true;
    };
    let hit = (PILL_HIT * scale) as i32;
    let right = pos.x + size.width as i32;
    let bottom = pos.y + size.height as i32;
    let cx = cursor.x as i32;
    let cy = cursor.y as i32;
    cx >= right - hit && cx < right && cy >= bottom - hit && cy < bottom
}

/// System tray: left-click toggles the panel, right-click opens a Quit menu.
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let Some(icon) = app.default_window_icon().cloned() else {
        return Ok(());
    };
    let toggle = MenuItem::with_id(app, "toggle", "Abrir / fechar painel", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &quit])?;

    TrayIconBuilder::with_id("semaforo")
        .icon(icon)
        .tooltip("Claude Semáforo")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "toggle" => {
                let _ = app.emit("toggle-panel", ());
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = tray.app_handle().emit("toggle-panel", ());
            }
        })
        .build(app)?;
    Ok(())
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
