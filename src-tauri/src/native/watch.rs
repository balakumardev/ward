//! Plan 10 — fs watcher with debounce.
//!
//! Wraps `notify-debouncer-mini` so callers receive changed paths on a
//! `std::sync::mpsc::Receiver<PathBuf>`. The debounce window defaults
//! to 500ms — long enough to coalesce editor save storms, short enough
//! that the UI feels live.
//!
//! `watch_paths` is the single entry point; tests create a tempdir,
//! write a file inside it, sleep 700ms (longer than the debounce
//! window), and assert at least one event arrived.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};

use crate::error::WardError;

/// Default debounce window. Long enough to coalesce a 5-write editor
/// save storm into one event; short enough to feel live.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(500);

/// Owns the underlying `notify` watcher. Drop = stop watching. We wrap
/// the inner handle so callers don't need to depend on the
/// `notify-debouncer-mini` types directly.
pub struct Watcher {
    /// Held so the OS-level watch stays alive. Drop = stop.
    _debouncer: Debouncer<notify::RecommendedWatcher>,
    /// Receives the changed paths emitted by the debouncer.
    pub rx: Receiver<PathBuf>,
}

impl Watcher {
    /// Stop watching. The underlying debouncer stops on drop; this
    /// method exists for symmetry so callers can be explicit about
    /// shutdown (and so future stop-event integration has a hook).
    pub fn stop(self) {
        // Drop the debouncer to stop the OS-level watch. Drop the rx
        // to close the channel on the consumer side.
        drop(self._debouncer);
        drop(self.rx);
    }
}

/// Start watching `paths` with a 500ms debounce. Returns the
/// `Watcher` (which must be kept alive — drop = stop) and a
/// `Receiver<PathBuf>` of changed paths.
///
/// Errors are returned when the underlying `notify` watcher fails to
/// initialize (e.g. path does not exist or permission denied).
pub fn watch_paths(paths: Vec<PathBuf>) -> Result<Watcher, WardError> {
    watch_paths_with_debounce(paths, DEFAULT_DEBOUNCE)
}

/// Same as `watch_paths` but lets callers override the debounce
/// window. Useful for tests that want a tighter loop.
pub fn watch_paths_with_debounce(
    paths: Vec<PathBuf>,
    debounce: Duration,
) -> Result<Watcher, WardError> {
    let (tx, rx) = channel::<PathBuf>();

    // Bridge the debouncer's `DebounceEventResult` to our flat
    // `PathBuf` channel. The closure owns `tx` by move.
    let mut debouncer = new_debouncer(debounce, move |res: DebounceEventResult| {
        match res {
            Ok(events) => {
                for e in events {
                    // We intentionally send every event; consumers can
                    // dedup by path if they need to. Sending errors
                    // here would require an error type on the channel,
                    // which complicates the API for no current benefit.
                    let _ = tx.send(e.path);
                }
            }
            Err(e) => {
                // Errors from notify are swallowed today — a watcher
                // can fail mid-lifetime (path unmounted, etc.) and
                // we don't have a UI surface for that. The OS-level
                // watcher keeps running for the other paths.
                eprintln!("ward: fs-watch error: {e}");
            }
        }
    })
    .map_err(|e| WardError::NotFound(format!("notify watcher init: {e}")))?;

    // Register every requested path. Failure to watch one path doesn't
    // abort the others — we log and continue.
    for p in &paths {
        if let Err(e) = debouncer
            .watcher()
            .watch(p.as_path(), RecursiveMode::Recursive)
        {
            // Continue; the other paths are still watched.
            eprintln!("ward: cannot watch {}: {e}", p.display());
        }
    }

    Ok(Watcher {
        _debouncer: debouncer,
        rx,
    })
}

/// Compute the default set of paths Ward should watch. Today this is
/// `~/.claude` and (when present) `~/.codex`. The Tauri runtime calls
/// this once at startup; tests can call it directly to assert the
/// behavior with a fake HOME.
pub fn default_watch_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let claude = home.join(".claude");
        if claude.exists() {
            paths.push(claude);
        }
        let codex = home.join(".codex");
        if codex.exists() {
            paths.push(codex);
        }
    }
    paths
}

/// Helper for tests: collect events from `rx` for up to `timeout`,
/// returning whatever arrived. We use this rather than a busy loop
/// because the tests need to share time with the watcher thread.
#[cfg(test)]
pub fn drain_for(rx: &Receiver<PathBuf>, timeout: Duration) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            break;
        }
        let remaining = deadline - now;
        match rx.recv_timeout(remaining) {
            Ok(p) => out.push(p),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    out
}

/// macOS resolves `/var/folders/...` to `/private/var/folders/...`
/// inside the watcher callback (FSEvents hands us the resolved form),
/// but `tempdir` gives back the unresolved form. Tests compare with
/// this helper so the assertion doesn't fail on the symlink shim.
#[cfg(test)]
pub fn path_eq(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    let ca = std::fs::canonicalize(a).ok();
    let cb = std::fs::canonicalize(b).ok();
    match (ca, cb) {
        (Some(ca), Some(cb)) => ca == cb,
        _ => false,
    }
}

/// Convenience used by tests and the GUI launcher to assert that a
/// directory at `p` exists before adding it to the watch list.
pub fn path_exists(p: &Path) -> bool {
    p.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    #[test]
    fn watch_paths_emits_event_on_file_create() {
        let dir = tempfile::tempdir().unwrap();
        let watcher = watch_paths(vec![dir.path().to_path_buf()]).unwrap();

        // Give the OS a moment to register the watch, then create
        // a file. The debounce window is 500ms; we wait 1500ms total
        // to be safe on slow CI.
        std::thread::sleep(Duration::from_millis(100));
        let target = dir.path().join("hello.txt");
        fs::write(&target, "world").unwrap();

        let events = drain_for(&watcher.rx, Duration::from_millis(1500));
        assert!(
            events.iter().any(|p| path_eq(p, &target)),
            "expected at least one event for {}, got {:?}",
            target.display(),
            events
        );

        watcher.stop();
    }

    #[test]
    fn watch_paths_debounces_burst() {
        // A burst of writes within the debounce window should
        // collapse into one (or a small number of) events, not N.
        let dir = tempfile::tempdir().unwrap();
        let watcher = watch_paths(vec![dir.path().to_path_buf()]).unwrap();
        std::thread::sleep(Duration::from_millis(100));

        let target = dir.path().join("burst.txt");
        for i in 0..5 {
            fs::write(&target, format!("write {i}")).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }

        let events = drain_for(&watcher.rx, Duration::from_millis(1500));
        let count = events.iter().filter(|p| path_eq(p, &target)).count();
        // We don't pin to 1 — some platforms (notably macOS FSEvents)
        // emit intermediate frames — but a burst of 5 writes should
        // definitely produce fewer than 5 events.
        assert!(
            count <= 4,
            "expected debounce to coalesce 5 writes, got {count} events"
        );
        assert!(count >= 1, "expected at least one event");

        watcher.stop();
    }

    #[test]
    fn watch_paths_with_unknown_dir_still_returns_watcher() {
        // A path that doesn't exist is logged-and-skipped; the watcher
        // is still created (and can later be told to watch other paths
        // — that's a future extension). We just assert no panic.
        let bogus = std::env::temp_dir().join("ward-definitely-does-not-exist-xyz");
        let _ = fs::remove_dir_all(&bogus);
        let result = watch_paths(vec![bogus.clone()]);
        assert!(result.is_ok());
        if let Ok(w) = result {
            w.stop();
        }
        let _ = fs::remove_dir_all(&bogus);
    }

    #[test]
    fn watch_paths_multiple_paths_at_least_one_event() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let watcher =
            watch_paths(vec![dir_a.path().to_path_buf(), dir_b.path().to_path_buf()]).unwrap();
        std::thread::sleep(Duration::from_millis(100));

        let target = dir_b.path().join("in_b.txt");
        fs::write(&target, "x").unwrap();

        let events = drain_for(&watcher.rx, Duration::from_millis(1500));
        assert!(
            events.iter().any(|p| path_eq(p, &target)),
            "expected event from dir_b, got {:?}",
            events
        );
        watcher.stop();
    }

    #[test]
    fn default_watch_paths_does_not_panic() {
        // We can't easily mock HOME in a unit test, but we can assert
        // the function returns a Vec (possibly empty) and doesn't panic.
        let paths = default_watch_paths();
        // Every returned path should exist; the function filters by
        // `exists()` so this is a no-cost invariant.
        for p in &paths {
            assert!(p.exists(), "{} should exist (pre-filtered)", p.display());
        }
    }

    #[test]
    fn path_exists_works() {
        let dir = tempfile::tempdir().unwrap();
        assert!(path_exists(dir.path()));
        assert!(!path_exists(&dir.path().join("nope")));
    }
}