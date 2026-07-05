//! Plan 13 — app lifecycle: close-to-tray vs. real quit.
//!
//! `QUITTING` is set true only by a genuine quit path (the tray "Quit"
//! menu item, or macOS `ExitRequested` from ⌘Q). While it is false, a
//! window `CloseRequested` (red button / ⌘W) hides the window to the
//! menu bar instead of letting it close — so the tray + fs-watcher keep
//! the background agent alive.

use std::sync::atomic::{AtomicBool, Ordering};

/// True while a genuine quit is in progress.
pub static QUITTING: AtomicBool = AtomicBool::new(false);

/// Mark that a genuine quit is in progress.
pub fn mark_quitting() {
    QUITTING.store(true, Ordering::SeqCst);
}

/// Is a genuine quit in progress?
pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

/// Should a window `CloseRequested` hide-to-tray (true) rather than
/// proceed as a real close (false)? Hide unless we're genuinely quitting.
pub fn should_hide_on_close(quitting: bool) -> bool {
    !quitting
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_when_not_quitting() {
        assert!(should_hide_on_close(false));
    }

    #[test]
    fn does_not_hide_when_quitting() {
        assert!(!should_hide_on_close(true));
    }

    #[test]
    fn mark_and_read_quitting_roundtrip() {
        QUITTING.store(false, Ordering::SeqCst);
        assert!(!is_quitting());
        mark_quitting();
        assert!(is_quitting());
        QUITTING.store(false, Ordering::SeqCst); // reset so other tests see a clean flag
    }
}
