pub mod backup;
pub mod cli;
mod commands;
mod effective;
mod error;
mod fs_utils;
mod harness;
mod model;
pub mod mcp;
pub mod native;
pub mod security;
pub mod sessions;
pub mod tokenizer;
pub mod usage;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use notify_debouncer_mini::Debouncer;
use tauri::{Emitter, Listener, Manager};

use crate::backup::git as git_ops;
use crate::harness::adapters::claude::ClaudeAdapter;
use crate::harness::{framework, Ctx, Registry};
use crate::native::watch;

/// Plan 08 — headless backup run triggered by the launchd agent.
///
/// Parses `--backup-once <scan_target>` out of `argv`, runs
/// `backup_run` semantics (export + commit) and exits with the
/// process status. Returns exit code 0 on success, non-zero on
/// failure. Does NOT push — that's gated behind `backup_push`.
pub fn run_backup_once(argv: &[String]) -> i32 {
    let scan_target = match argv.windows(2).find(|w| w[0] == "--backup-once") {
        Some(w) => w[1].clone(),
        None => {
            eprintln!("ward: --backup-once requires <scan_target>");
            return 2;
        }
    };

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!("ward: cannot resolve HOME");
            return 3;
        }
    };
    let bd = match git_ops::backup_dir() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("ward: backup_dir: {e}");
            return 4;
        }
    };
    let repo = git_ops::repo_dir(&bd);

    // Init the repo on first run.
    if let Err(e) = git_ops::init(
        repo,
        git_ops::FALLBACK_USER_NAME,
        git_ops::FALLBACK_USER_EMAIL,
    ) {
        eprintln!("ward: init failed: {e}");
        return 5;
    }
    let _ = git_ops::ensure_identity_or_fallback(
        repo,
        git_ops::FALLBACK_USER_NAME,
        git_ops::FALLBACK_USER_EMAIL,
    );

    // Resolve which harness we're backing up. Today only claude;
    // the launcher forwards the harness id string so Plan 09 (Codex)
    // slots in transparently.
    let harness_id = scan_target.as_str();
    let scan = match harness_id {
        "claude" => {
            let mut r = Registry::new();
            r.register(Box::new(ClaudeAdapter));
            let ctx = Ctx { home: &home, cwd: None };
            match framework::run_scan(&ClaudeAdapter, &ctx) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("ward: scan failed: {e}");
                    return 6;
                }
            }
        }
        other => {
            eprintln!("ward: unknown scan_target: {other}");
            return 7;
        }
    };
    let _ = scan;

    // Export + commit the source tree.
    let source_root = home.join(".claude");
    let report = match git_ops::export_to_repo(&source_root, repo) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ward: export failed: {e}");
            return 8;
        }
    };
    if report.files_copied > 0 {
        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let msg = format!("backup: ward (claude) {}", ts);
        if let Err(e) = git_ops::commit(repo, &msg) {
            eprintln!("ward: commit failed: {e}");
            return 9;
        }
    }
    0
}

/// Plan 10 — keepalive handle for the fs watcher. We share the
/// debouncer with the bridge thread via `Arc<Mutex<>>` so dropping
/// the handle on either side cleanly stops the watch.
pub struct WatcherHandle(pub Arc<Mutex<Option<Debouncer<notify::RecommendedWatcher>>>>);

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        if let Ok(mut g) = self.0.lock() {
            // Dropping the debouncer here stops the OS-level watch.
            *g = None;
        }
    }
}

/// Plan 10 — start the fs watcher on the harness directories. Spawns a
/// background thread that drains the watcher's `mpsc::Receiver<PathBuf>`
/// and emits a `config-changed` event on every flushed change (debounced
/// to 1s windows on top of the watcher's own 500ms debounce, so editor
/// save storms collapse into one event per second).
///
/// Returns `Some(WatcherHandle)` if any of the requested paths existed
/// and a watcher was successfully set up; `None` if every path was
/// missing — in which case the GUI still launches, just without
/// live-refresh.
pub fn start_watcher<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Option<WatcherHandle> {
    let paths = watch::default_watch_paths();
    if paths.is_empty() {
        return None;
    }
    let WatcherSplit { debouncer, rx } = build_watcher_split(paths)?;

    // Bridge the path receiver into a Tauri event. We throttle to 1Hz
    // so a flurry of saves doesn't flood the frontend with
    // `config-changed` events (the frontend re-runs a full scan on
    // each one).
    let app_handle = app.clone();
    std::thread::spawn(move || {
        let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(10);
        let min_gap = std::time::Duration::from_millis(1000);
        let mut pending: Vec<PathBuf> = Vec::new();
        loop {
            let recv = rx.recv_timeout(std::time::Duration::from_millis(500));
            match recv {
                Ok(p) => pending.push(p),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if !pending.is_empty() && last_emit.elapsed() >= min_gap {
                let _ = app_handle.emit("config-changed", &pending);
                pending.clear();
                last_emit = std::time::Instant::now();
            }
        }
    });

    Some(WatcherHandle(Arc::new(Mutex::new(Some(debouncer)))))
}

/// Internal helper: build the watcher, then split into debouncer +
/// receiver so the debouncer can be shared via `Arc<Mutex<>>`. Used
/// by `start_watcher` and the relevant test(s) if added later.
struct WatcherSplit {
    debouncer: Debouncer<notify::RecommendedWatcher>,
    rx: std::sync::mpsc::Receiver<PathBuf>,
}

fn build_watcher_split(paths: Vec<PathBuf>) -> Option<WatcherSplit> {
    let watcher = watch::watch_paths(paths).ok()?;
    // `watch_paths` returns a Watcher we own. Destructure; the
    // debouncer keeps the OS-level watch alive, the receiver pumps
    // events into the bridge.
    let watch::Watcher { _debouncer, rx } = watcher;
    Some(WatcherSplit {
        debouncer: _debouncer,
        rx,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--start-hidden"]),
        ))
        .setup(|app| {
            // Menu-bar tray (Plan 10). Build it after the app is
            // initialised so the window icon is available. We hold
            // the TrayIcon in a keepalive box on the app's managed
            // state so it lives as long as the app does.
            match crate::native::tray::setup(app) {
                Ok(tray) => {
                    app.manage(Plan10Tray(Some(Box::new(tray))));
                }
                Err(e) => {
                    eprintln!("ward: tray setup failed: {e}");
                }
            }

            // Launch-at-login UX (Plan 13): when the LaunchAgent starts
            // Ward with `--start-hidden`, hide the main window so it boots
            // into the menu bar instead of stealing focus. Reopen via the
            // tray "Open Ward" menu item.
            if std::env::args().any(|a| a == "--start-hidden") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            // Launch-at-login (Plan 13): enable once on first run, gated by
            // a ~/.ward/first-run sentinel so a later opt-out persists.
            crate::native::autostart::enable_on_first_run(app.handle());

            // Start the fs watcher (Plan 10). Stash the keepalive
            // handle in app-managed state.
            if let Some(handle) = start_watcher(app.handle().clone()) {
                app.manage(Plan10Watcher(Some(handle)));
            }

            // Handle tray menu actions. We use a global event
            // (`tray_action`) so the frontend can react.
            let app_handle = app.handle().clone();
            app.listen("tray_action", move |event| {
                let id_str: String = match event.payload() {
                    payload if payload.starts_with('"') && payload.ends_with('"') => {
                        payload[1..payload.len() - 1].to_string()
                    }
                    _ => event.payload().to_string(),
                };
                match id_str.as_str() {
                    "scan" => {
                        // Re-emit a fresh `scan-now` for the frontend.
                        let _ = app_handle.emit("scan-now", ());
                    }
                    "quit" => {
                        crate::native::lifecycle::mark_quitting();
                        app_handle.exit(0);
                    }
                    "open" | _ => {
                        // Bring the main window forward.
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Close-to-tray (Plan 13): the red button / ⌘W hides the
            // window to the menu bar; only a genuine quit closes it.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if crate::native::lifecycle::should_hide_on_close(
                    crate::native::lifecycle::is_quitting(),
                ) {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::scan,
            commands::read_file_content,
            commands::list_destinations,
            commands::move_item,
            commands::delete_item,
            commands::restore,
            commands::save_file,
            commands::bulk,
            commands::bulk_restore,
            commands::mcp_get_disabled,
            commands::mcp_set_disabled,
            commands::mcp_get_policy,
            commands::mcp_set_policy,
            commands::mcp_check_policy,
            commands::security_scan,
            commands::security_baseline_check,
            commands::security_baseline_accept,
            commands::context_budget,
            commands::session_preview,
            commands::session_cost,
            commands::session_distill,
            commands::session_trim,
            commands::backup_status,
            commands::backup_run,
            commands::backup_sync,
            commands::backup_push,
            commands::backup_set_remote,
            commands::backup_scheduler_install,
            commands::backup_scheduler_remove,
            commands::autostart_status,
            commands::autostart_set
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                crate::native::lifecycle::mark_quitting();
            }
        });
}

/// Holds the tray icon alive for the lifetime of the app. We use a
/// thin newtype rather than `Box<dyn Any>` so the type is searchable
/// in managed state.
struct Plan10Tray<R: tauri::Runtime>(Option<Box<tauri::tray::TrayIcon<R>>>);

/// Holds the watcher keepalive alive for the lifetime of the app.
struct Plan10Watcher(Option<WatcherHandle>);