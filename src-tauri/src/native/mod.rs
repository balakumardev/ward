//! Plan 10 — native shell primitives.
//!
//! Submodules:
//!   - `anchor`    — pure multi-monitor placement math for the tray popover.
//!   - `autostart` — launch-at-login via `tauri-plugin-autostart` (Plan 13).
//!   - `lifecycle` — close-to-tray vs. genuine quit gating (Plan 13).
//!   - `notify`    — hash-based notification dedup so the user only sees a
//!                   critical-finding toast the first time a finding appears.
//!   - `tray`      — Tauri menu-bar tray + right-click menu + dock badge.
//!   - `watch`     — `notify-debouncer-mini`-backed fs watcher with a
//!                   500ms debounce; emits changed paths on an mpsc channel.
//!
//! All five modules are designed to be testable in isolation. `tray` and the
//! `autostart`/`lifecycle` glue require a real Tauri App context and are
//! verified manually; `notify` and `watch` have unit tests exercising the
//! core logic.

pub mod anchor;
pub mod autostart;
pub mod lifecycle;
pub mod notify;
pub mod tray;
pub mod watch;