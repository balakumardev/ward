//! Plan 08 — Backup Center.
//!
//! Submodules:
//!   - `git`       — git init/export/commit/push/status/log primitives.
//!   - `scheduler` — launchd plist install/remove/status.
//!
//! Production callers reach these via the Tauri commands in
//! `crate::commands` (and the `--backup-once` CLI flag in `main.rs`).

pub mod git;
pub mod scheduler;
