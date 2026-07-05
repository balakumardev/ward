//! Plan 13 — launch-at-login (autostart) helpers.
//!
//! Wraps `tauri-plugin-autostart`'s `ManagerExt` so the rest of the app
//! deals in plain `Result<_, WardError>`. The plugin registers a per-user
//! macOS LaunchAgent — distinct from the Plan 08 backup LaunchAgent
//! (`dev.balakumar.ward.backup`), so the two never collide.
//!
//! First-run policy: Ward enables launch-at-login ONCE, the first time it
//! starts, gated by a `~/.ward/first-run` sentinel. After that we never
//! re-enable, so a user who turns it off stays off.

use std::path::PathBuf;

use tauri::{AppHandle, Runtime};
use tauri_plugin_autostart::ManagerExt;

use crate::error::WardError;

/// Path to the first-run sentinel: `~/.ward/first-run`.
pub fn first_run_sentinel_path() -> Result<PathBuf, WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    Ok(home.join(".ward").join("first-run"))
}

/// Pure decision: auto-enable launch-at-login only on the very first run
/// (no sentinel yet) AND only if it isn't already enabled.
pub fn should_enable_on_first_run(sentinel_exists: bool, currently_enabled: bool) -> bool {
    !sentinel_exists && !currently_enabled
}

/// Create `~/.ward/` (if needed) and write the sentinel so we never
/// auto-enable again.
pub fn mark_first_run_done() -> Result<(), WardError> {
    let path = first_run_sentinel_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, b"1")?;
    Ok(())
}

/// Is launch-at-login currently enabled?
pub fn status<R: Runtime>(app: &AppHandle<R>) -> Result<bool, WardError> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| WardError::Autostart(format!("is_enabled: {e}")))
}

/// Enable or disable launch-at-login.
pub fn set<R: Runtime>(app: &AppHandle<R>, enabled: bool) -> Result<(), WardError> {
    let mgr = app.autolaunch();
    let res = if enabled { mgr.enable() } else { mgr.disable() };
    res.map_err(|e| WardError::Autostart(format!("set({enabled}): {e}")))
}

/// First-run policy: enable launch-at-login once, then mark done. Errors
/// are logged, never propagated — autostart must never block startup.
pub fn enable_on_first_run<R: Runtime>(app: &AppHandle<R>) {
    let sentinel_exists = first_run_sentinel_path().map(|p| p.exists()).unwrap_or(true);
    let currently_enabled = status(app).unwrap_or(false);
    if should_enable_on_first_run(sentinel_exists, currently_enabled) {
        if let Err(e) = set(app, true) {
            eprintln!("ward: autostart first-run enable failed: {e}");
        }
    }
    if let Err(e) = mark_first_run_done() {
        eprintln!("ward: autostart mark first-run failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enables_only_on_true_first_run() {
        assert!(should_enable_on_first_run(false, false));
        assert!(!should_enable_on_first_run(true, false));
        assert!(!should_enable_on_first_run(false, true));
        assert!(!should_enable_on_first_run(true, true));
    }

    #[test]
    fn sentinel_path_is_under_dot_ward() {
        let p = first_run_sentinel_path().unwrap();
        assert!(p.ends_with(".ward/first-run"), "got {p:?}");
    }
}
