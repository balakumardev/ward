//! Plan 10 — native shell primitives.
//!
//! Submodules:
//!   - `watch`  — `notify-debouncer-mini`-backed fs watcher with a
//!                500ms debounce; emits changed paths on an mpsc channel.
//!   - `notify` — hash-based notification dedup so the user only sees a
//!                critical-finding toast the first time a finding appears.
//!   - `tray`   — Tauri menu-bar tray + right-click menu + dock badge.
//!
//! All three modules are designed to be testable in isolation. `tray`
//! requires a real Tauri App context and is verified manually; the
//! other two have unit tests that exercise the core logic.

pub mod autostart;
pub mod notify;
pub mod tray;
pub mod watch;