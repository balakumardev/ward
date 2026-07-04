//! Sessions mode (Plan 07) — view / cost / distill / trim Claude Code
//! JSONL session files. Built on top of the `session` category that
//! Plan 02 already exposes via the scan result.
//!
//! Module layout:
//!   - `parse`   — streaming JSONL → structured `Conversation`
//!   - `cost`    — per-model token/cost breakdown
//!   - `distill` — backup → clean → `index.md` writer
//!   - `trim`    — replace base64 image blocks with `[image redacted]`

pub mod cost;
pub mod distill;
pub mod parse;
pub mod trim;