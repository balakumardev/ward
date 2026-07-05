# Plan 14 — Usage Engine (Rust) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A local, offline `usage/` Rust module that reports Claude Code and Codex CLI usage — token totals, cost, and reset countdowns — from the session files already on disk, exposed as one `usage_snapshot(harness)` Tauri command.

**Architecture:** `usage/blocks.rs` reconstructs Claude's 5-hour billing windows from message timestamps (ported from MIT ccusage). `usage/claude.rs` parses `~/.claude/projects/**/*.jsonl` into de-duplicated usage entries, runs the block algorithm for the current window, and sums a rolling week. `usage/codex.rs` parses `~/.codex/sessions/**/rollout-*.jsonl` `token_count` events, buckets cumulative-delta tokens by event timestamp, and reads the real `rate_limits` payload for authoritative percent + reset. Cost reuses the existing `sessions::cost` pricing table (no new pricing source, no build-time network). `usage/mod.rs` holds the shared models and the `usage_snapshot` dispatcher.

**Tech Stack:** Rust, `serde`/`serde_json`, `chrono` (already deps), reusing `sessions::cost` + `sessions::parse::Usage`.

## Global Constraints

- **Local-only, no network, no credential reading.** Every value comes from files under `~/.claude` / `~/.codex`. No HTTP, no reading `.credentials.json`/Keychain.
- **Models** derive `Debug, Clone, Serialize, Deserialize, PartialEq` with `#[serde(rename_all = "camelCase")]` (project convention). Name new types to avoid the existing `sessions::Usage` / `sessions::cost::ModelCost` — use `UsageSnapshot`, `UsageWindow`, `TokenTotals`, `UsageSource`.
- **Errors** extend/return `WardError` (thiserror + manual camelCase `Serialize`). A missing/empty harness dir is `available: false`, NOT an error. Malformed JSONL lines are skipped (parse-tolerant), matching `sessions::parse`.
- **Tauri v2 only.** Command registered in `lib.rs` via `tauri::generate_handler!`. JS camelCase → Rust snake_case automatic.
- **DRY pricing:** reuse `sessions::cost::{price_for, cost_for, ModelPrice}` (make them `pub(crate)`). Do NOT add a second pricing table or a `build.rs` network fetch (would break offline builds and the no-network rule — a documented deviation from the design spec §5's "embed LiteLLM at build time").
- **Attribution:** the block algorithm is ported from `ryoppippi/ccusage` (MIT) and its Rust ports `hydai/ccstat` (MIT) / `DaveDev42/ccusage-in-rust` (BSD-3). Add a `NOTICE` entry (Task 4), consistent with the existing CCO attribution.
- **Do not rename** existing names: `WardError`, `sessions::parse::Usage`, `sessions::cost::compute`, `commands`, `run()`.
- **Commit `Cargo.lock`** only if deps change (none expected — chrono/serde already present). One commit per task, conventional prefix (`feat(plan14):` / `refactor(plan14):`).
- **No stubs / TODOs / placeholders.** Golden tests use synthetic JSONL written to tempdirs (the repo's pattern) — do NOT commit real session files.
- **Tests:** every `cd src-tauri && cargo test --lib` must pass before a task is done.

**Real on-disk formats this plan targets (verified against the user's files):**
- Claude assistant line: `{"type":"assistant","timestamp":"2026-05-14T04:34:54.786Z","uuid":"…","message":{"id":"msg_…","model":"claude-…","usage":{"input_tokens":N,"output_tokens":N,"cache_read_input_tokens":N,"cache_creation_input_tokens":N}}}` (dedup key = `message.id`, fallback top-level `uuid`).
- Codex line: `{"timestamp":"2026-05-14T04:34:54.786Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":N,"cached_input_tokens":N,"output_tokens":N,"reasoning_output_tokens":N,"total_tokens":N},"last_token_usage":{…}},"rate_limits":{"primary":{"used_percent":8.46,"window_minutes":300,"resets_at":1778067464},"secondary":{"used_percent":16.69,"window_minutes":10080,"resets_at":1778305419},"plan_type":null}}}` (`used_percent` is 0–100; `resets_at` is epoch **seconds**; `primary`=5h, `secondary`=weekly).

---

## Task 1: Usage models + 5-hour-block algorithm

Create the shared data models and the pure block algorithm (with golden tests). Register the `usage` module.

**Files:**
- Create: `src-tauri/src/usage/mod.rs` (models + module decls)
- Create: `src-tauri/src/usage/blocks.rs` (algorithm)
- Modify: `src-tauri/src/lib.rs` (add `pub mod usage;`)
- Test: both new files (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `usage::TokenTotals { input, output, cache_creation, cache_read, total: u64 }`
  - `usage::UsageWindow { tokens: TokenTotals, cost_usd: f64, percent: Option<f64>, resets_at: Option<String>, resets_in_secs: Option<i64>, is_active: bool, started_at: Option<String>, plan_type: Option<String> }`
  - `usage::UsageSource` (`Local` | `RateLimits`)
  - `usage::UsageSnapshot { harness: String, block: UsageWindow, week: UsageWindow, source: UsageSource, available: bool, generated_at: String }`
  - `usage::blocks::{SESSION_MS, floor_to_hour, identify_blocks, current_block, BlockInfo}`
  - `usage::TokenTotals::add(&mut self, input, output, cache_creation, cache_read)` and `UsageWindow::empty()` helpers used by Tasks 2/3.

- [ ] **Step 1: Write the failing tests for the models + blocks**

Create `src-tauri/src/usage/blocks.rs` with the test module first (implementation stubbed to fail):

```rust
//! Plan 14 — Claude 5-hour billing-window reconstruction.
//! Ported from ccusage (MIT) `identify_session_blocks`. (WIP — Step 3.)

pub const SESSION_MS: i64 = 5 * 60 * 60 * 1000; // 18_000_000

pub fn floor_to_hour(_ms: i64) -> i64 { unimplemented!() }

#[derive(Debug, Clone, PartialEq)]
pub struct BlockInfo { pub start_ms: i64, pub end_ms: i64, pub is_active: bool }

pub fn identify_blocks(_sorted_ts: &[i64]) -> Vec<BlockInfo> { unimplemented!() }
pub fn current_block(_sorted_ts: &[i64], _now_ms: i64) -> Option<BlockInfo> { unimplemented!() }

#[cfg(test)]
mod tests {
    use super::*;
    const HOUR: i64 = 3_600_000;

    #[test]
    fn floor_to_hour_snaps_down() {
        assert_eq!(floor_to_hour(10 * HOUR + 500_000), 10 * HOUR);
        assert_eq!(floor_to_hour(10 * HOUR), 10 * HOUR);
    }

    #[test]
    fn single_entry_one_block_end_is_start_plus_5h() {
        let t = 10 * HOUR + 90 * 60 * 1000; // 10:90 → floors to 10:00
        let blocks = identify_blocks(&[t]);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].start_ms, 10 * HOUR);
        assert_eq!(blocks[0].end_ms, 10 * HOUR + SESSION_MS);
    }

    #[test]
    fn gap_over_5h_opens_new_block() {
        let a = 10 * HOUR;
        let b = a + SESSION_MS + 1; // >5h after previous entry
        let blocks = identify_blocks(&[a, b]);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].start_ms, floor_to_hour(a));
        assert_eq!(blocks[1].start_ms, floor_to_hour(b));
    }

    #[test]
    fn entries_within_5h_stay_one_block() {
        let a = 10 * HOUR;
        let b = a + 4 * HOUR; // within 5h of start and previous
        let blocks = identify_blocks(&[a, b]);
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn boundary_exactly_5h_is_same_block() {
        // strict `>`: exactly SESSION_MS after start is NOT a new block
        let a = 10 * HOUR;
        let b = a + SESSION_MS;
        assert_eq!(identify_blocks(&[a, b]).len(), 1);
    }

    #[test]
    fn current_block_active_when_recent_and_before_end() {
        let start = 10 * HOUR;
        let last = start + 60 * 60 * 1000; // 1h in
        let now = last + 30 * 60 * 1000;   // 30m after last, still < end
        let b = current_block(&[start, last], now).unwrap();
        assert!(b.is_active);
        assert_eq!(b.end_ms, start + SESSION_MS);
    }

    #[test]
    fn current_block_inactive_when_stale() {
        let start = 10 * HOUR;
        let last = start + 60 * 60 * 1000;
        let now = last + SESSION_MS + 1; // >5h since last activity
        let b = current_block(&[start, last], now).unwrap();
        assert!(!b.is_active);
    }

    #[test]
    fn current_block_none_when_empty() {
        assert!(current_block(&[], 123).is_none());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib usage::blocks::tests`
Expected: FAIL — the functions panic with `not implemented`. (You must add `pub mod usage;` to `lib.rs` and the module decls in Step 3's `mod.rs` for this to even compile; do the minimal `mod.rs` + `lib.rs` wiring now so the test compiles and fails on `unimplemented!()`, not on a missing module.)

To compile, also create a minimal `src-tauri/src/usage/mod.rs`:

```rust
pub mod blocks;
```

and add to `src-tauri/src/lib.rs` (with the other `pub mod` lines, alphabetically near `pub mod tokenizer;`):

```rust
pub mod usage;
```

- [ ] **Step 3: Implement the block algorithm**

Replace the non-test portion of `src-tauri/src/usage/blocks.rs` with:

```rust
//! Plan 14 — Claude 5-hour billing-window reconstruction.
//!
//! Ported from ccusage (MIT © ryoppippi) `identify_session_blocks` and its
//! Rust port ccstat (MIT). All timestamps are epoch-millis UTC. A block
//! starts at the first entry floored to the top of the hour; a new block
//! opens when an entry is more than `SESSION_MS` past the block start OR
//! past the previous entry (strict `>`); a block's end (its reset time) is
//! `start + SESSION_MS`.

/// The Claude billing window: 5 hours, in milliseconds.
pub const SESSION_MS: i64 = 5 * 60 * 60 * 1000; // 18_000_000

/// Floor an epoch-millis timestamp to the top of its UTC hour.
pub fn floor_to_hour(ms: i64) -> i64 {
    ms.div_euclid(3_600_000) * 3_600_000
}

/// One reconstructed 5-hour block. `end_ms` is the reset time shown to the user.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockInfo {
    pub start_ms: i64,
    pub end_ms: i64,
    pub is_active: bool,
}

/// Identify all 5-hour blocks from ascending-sorted event timestamps.
pub fn identify_blocks(sorted_ts: &[i64]) -> Vec<BlockInfo> {
    let mut blocks = Vec::new();
    let mut start: Option<i64> = None;
    let mut last: i64 = 0;
    for &ts in sorted_ts {
        match start {
            None => start = Some(floor_to_hour(ts)),
            Some(s) => {
                if ts - s > SESSION_MS || ts - last > SESSION_MS {
                    blocks.push(BlockInfo { start_ms: s, end_ms: s + SESSION_MS, is_active: false });
                    start = Some(floor_to_hour(ts));
                }
            }
        }
        last = ts;
    }
    if let Some(s) = start {
        blocks.push(BlockInfo { start_ms: s, end_ms: s + SESSION_MS, is_active: false });
    }
    blocks
}

/// The current block = the most recent block, marked active iff the last
/// entry is within `SESSION_MS` of `now` AND `now` is before the block end.
pub fn current_block(sorted_ts: &[i64], now_ms: i64) -> Option<BlockInfo> {
    let last_ts = *sorted_ts.last()?;
    let mut b = identify_blocks(sorted_ts).pop()?;
    b.is_active = (now_ms - last_ts) < SESSION_MS && now_ms < b.end_ms;
    Some(b)
}
```

- [ ] **Step 4: Run the block tests to verify they pass**

Run: `cd src-tauri && cargo test --lib usage::blocks::tests`
Expected: PASS (all 8 tests).

- [ ] **Step 5: Write the models + their serialization tests**

Replace `src-tauri/src/usage/mod.rs` with:

```rust
//! Plan 14 — local usage engine: Claude & Codex token/cost/reset from the
//! session files on disk. No network, no credential reading.
//!
//! `usage_snapshot(harness)` (Task 4) dispatches to `claude` / `codex`.

pub mod blocks;
pub mod claude;
pub mod codex;

use serde::{Deserialize, Serialize};

/// Token counts for a window. `total` is input+output+cache (what the UI shows).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenTotals {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    pub total: u64,
}

impl TokenTotals {
    /// Accumulate one entry's counts and keep `total` in sync.
    pub fn add(&mut self, input: u64, output: u64, cache_creation: u64, cache_read: u64) {
        self.input += input;
        self.output += output;
        self.cache_creation += cache_creation;
        self.cache_read += cache_read;
        self.total = self.input + self.output + self.cache_creation + self.cache_read;
    }
}

/// Where a window's numbers came from: reconstructed locally (Claude) or the
/// harness's own rate-limit payload (Codex).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UsageSource {
    Local,
    RateLimits,
}

/// One usage window (the current 5-hour block, or the week).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub tokens: TokenTotals,
    pub cost_usd: f64,
    /// 0.0..=1.0 when known (Codex, or Claude with a configured limit); else None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_in_secs: Option<i64>,
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
}

impl UsageWindow {
    /// A zeroed, inactive window.
    pub fn empty() -> Self {
        Self {
            tokens: TokenTotals::default(),
            cost_usd: 0.0,
            percent: None,
            resets_at: None,
            resets_in_secs: None,
            is_active: false,
            started_at: None,
            plan_type: None,
        }
    }
}

/// The full usage snapshot for one harness.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub harness: String,
    pub block: UsageWindow,
    pub week: UsageWindow,
    pub source: UsageSource,
    pub available: bool,
    pub generated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_totals_add_keeps_total_in_sync() {
        let mut t = TokenTotals::default();
        t.add(100, 20, 5, 1000);
        t.add(50, 10, 0, 0);
        assert_eq!(t.input, 150);
        assert_eq!(t.output, 30);
        assert_eq!(t.cache_creation, 5);
        assert_eq!(t.cache_read, 1000);
        assert_eq!(t.total, 150 + 30 + 5 + 1000);
    }

    #[test]
    fn snapshot_serializes_camel_case() {
        let s = UsageSnapshot {
            harness: "claude".into(),
            block: UsageWindow::empty(),
            week: UsageWindow::empty(),
            source: UsageSource::Local,
            available: true,
            generated_at: "2026-07-05T00:00:00Z".into(),
        };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"harness\":\"claude\""));
        assert!(j.contains("\"source\":\"local\""));
        assert!(j.contains("\"available\":true"));
        assert!(j.contains("\"generatedAt\":\"2026-07-05T00:00:00Z\""));
        // isActive present; optional Nones omitted
        assert!(j.contains("\"isActive\":false"));
        assert!(!j.contains("resetsAt"));
        assert!(!j.contains("\"percent\""));
    }

    #[test]
    fn source_rate_limits_serializes_camel() {
        assert_eq!(serde_json::to_string(&UsageSource::RateLimits).unwrap(), "\"rateLimits\"");
    }
}
```

Note: `mod.rs` now declares `pub mod claude;` and `pub mod codex;`, which don't exist yet — create **empty placeholder files** so the crate compiles for this task's tests:
`src-tauri/src/usage/claude.rs` containing only `//! Plan 14 — Claude usage (Task 2).` and
`src-tauri/src/usage/codex.rs` containing only `//! Plan 14 — Codex usage (Task 3).`
(Task 2/3 replace them. An empty module compiles.)

- [ ] **Step 6: Run the full new module's tests**

Run: `cd src-tauri && cargo test --lib usage::`
Expected: PASS (blocks tests + mod tests). `cargo check` clean.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/usage/mod.rs src-tauri/src/usage/blocks.rs \
        src-tauri/src/usage/claude.rs src-tauri/src/usage/codex.rs src-tauri/src/lib.rs
git commit -m "feat(plan14): usage models + 5-hour-block algorithm (ported from ccusage)"
```

---

## Task 2: Claude usage engine

Parse `~/.claude/projects/**/*.jsonl` assistant lines into de-duplicated timestamped usage entries, run the block algorithm for the current window, sum a rolling 7-day week, and cost via the reused pricing table.

**Files:**
- Modify: `src-tauri/src/sessions/cost.rs` (make `price_for`, `cost_for`, `ModelPrice` `pub(crate)`)
- Replace: `src-tauri/src/usage/claude.rs`
- Test: `src-tauri/src/usage/claude.rs` (inline, synthetic JSONL in tempdirs)

**Interfaces:**
- Consumes: `blocks::{current_block, SESSION_MS}`, `TokenTotals`, `UsageWindow`, `UsageSnapshot`, `UsageSource`; `sessions::cost::{price_for, cost_for}`; `sessions::parse::Usage`.
- Produces: `usage::claude::snapshot() -> Result<UsageSnapshot, WardError>`; internal `parse_entries_from(base: &Path) -> Vec<Entry>` (test seam taking a base dir).

- [ ] **Step 1: Make the pricing helpers reusable**

In `src-tauri/src/sessions/cost.rs`, change three private items to `pub(crate)` (do NOT change their bodies):
- `struct ModelPrice` → `pub(crate) struct ModelPrice` and its fields to `pub(crate)` (`input_per_mtok`, `output_per_mtok`, `cache_read_multiplier`, `cache_write_multiplier`).
- `fn price_for(model: &str) -> ModelPrice` → `pub(crate) fn price_for(...)`.
- `fn cost_for(u: &crate::sessions::parse::Usage, p: ModelPrice) -> f64` → `pub(crate) fn cost_for(...)`.

- [ ] **Step 2: Verify existing cost tests still pass (no behavior change)**

Run: `cd src-tauri && cargo test --lib sessions::cost::tests`
Expected: PASS (visibility-only change).

- [ ] **Step 3: Write the failing tests for Claude usage**

Replace `src-tauri/src/usage/claude.rs` with the tests + a stub `snapshot`/`parse_entries_from`:

```rust
//! Plan 14 — Claude usage. (WIP — Step 5.)
use std::path::Path;
use crate::error::WardError;
use super::UsageSnapshot;

pub(crate) struct Entry {
    pub ts_ms: i64,
    pub model: String,
    pub usage: crate::sessions::parse::Usage,
}

pub(crate) fn parse_entries_from(_base: &Path) -> Vec<Entry> { unimplemented!() }
pub fn snapshot() -> Result<UsageSnapshot, WardError> { unimplemented!() }

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // Write a projects/<proj>/<name>.jsonl under `base`.
    fn write_session(base: &Path, name: &str, body: &str) {
        let dir = base.join("projects").join("proj");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(name), body).unwrap();
    }

    fn assistant_line(ts: &str, msg_id: &str, model: &str, inp: u64, out: u64, cr: u64, cw: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","uuid":"u-{msg_id}","message":{{"id":"{msg_id}","model":"{model}","usage":{{"input_tokens":{inp},"output_tokens":{out},"cache_read_input_tokens":{cr},"cache_creation_input_tokens":{cw}}}}}}}"#
        )
    }

    #[test]
    fn parses_and_dedups_by_message_id() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let l = assistant_line("2026-05-14T04:34:54.786Z", "msg_1", "claude-sonnet-4-5", 100, 20, 1000, 500);
        // same message.id twice (streamed duplicate) → counted once
        write_session(base, "a.jsonl", &format!("{l}\n{l}\n"));
        let entries = parse_entries_from(base);
        assert_eq!(entries.len(), 1, "duplicate message.id must be deduped");
        assert_eq!(entries[0].usage.input_tokens, 100);
        assert_eq!(entries[0].model, "claude-sonnet-4-5");
    }

    #[test]
    fn skips_non_assistant_and_malformed_lines() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let good = assistant_line("2026-05-14T04:34:54.786Z", "msg_2", "claude-haiku-4-5", 10, 2, 0, 0);
        let body = format!(
            "{}\n{}\n{}\n",
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#,
            "{ broken json",
            good
        );
        write_session(base, "b.jsonl", &body);
        let entries = parse_entries_from(base);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn snapshot_unavailable_when_no_dir() {
        // Point CLAUDE_CONFIG_DIR at an empty temp dir → available:false, no error.
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.path());
        let snap = snapshot().unwrap();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        assert!(!snap.available);
        assert_eq!(snap.harness, "claude");
        assert_eq!(snap.block.tokens.total, 0);
    }
}
```

- [ ] **Step 4: Run to verify failure**

Run: `cd src-tauri && cargo test --lib usage::claude::tests`
Expected: FAIL — `not implemented`.

- [ ] **Step 5: Implement the Claude usage engine**

Replace `src-tauri/src/usage/claude.rs` with:

```rust
//! Plan 14 — Claude Code usage from `~/.claude/projects/**/*.jsonl`.
//!
//! Reads assistant lines (timestamp + message.id + model + usage), dedupes
//! by `message.id` (fallback top-level `uuid`), reconstructs the current
//! 5-hour block for the reset countdown, and sums a rolling 7-day week.
//! Cost reuses `sessions::cost` pricing. No network, no credentials.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::WardError;
use crate::sessions::cost::{cost_for, price_for};
use crate::sessions::parse::Usage;

use super::blocks;
use super::{TokenTotals, UsageSnapshot, UsageSource, UsageWindow};

pub(crate) struct Entry {
    pub ts_ms: i64,
    pub model: String,
    pub usage: Usage,
}

/// Base config dirs, most-specific first. `$CLAUDE_CONFIG_DIR` (comma-split)
/// overrides; otherwise `$XDG_CONFIG_HOME|~/.config /claude` then `~/.claude`.
fn base_dirs() -> Vec<PathBuf> {
    if let Ok(v) = std::env::var("CLAUDE_CONFIG_DIR") {
        let dirs: Vec<PathBuf> = v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).collect();
        if !dirs.is_empty() {
            return dirs;
        }
    }
    let mut out = Vec::new();
    let xdg = std::env::var("XDG_CONFIG_HOME").ok().map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")));
    if let Some(x) = xdg {
        out.push(x.join("claude"));
    }
    if let Some(h) = dirs::home_dir() {
        out.push(h.join(".claude"));
    }
    out
}

/// Recursively collect `*.jsonl` under `dir` into `out` (sorted by the caller).
fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_jsonl(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(p);
        }
    }
}

/// Parse + dedup all usage entries under one base dir's `projects/`.
pub(crate) fn parse_entries_from(base: &Path) -> Vec<Entry> {
    let mut files = Vec::new();
    collect_jsonl(&base.join("projects"), &mut files);
    files.sort();

    let mut seen: HashSet<String> = HashSet::new();
    let mut entries: Vec<Entry> = Vec::new();

    for f in files {
        let content = match std::fs::read_to_string(&f) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue, // skip malformed
            };
            if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                continue;
            }
            let msg = match v.get("message").and_then(|m| m.as_object()) {
                Some(m) => m,
                None => continue,
            };
            let usage_val = match msg.get("usage") {
                Some(u) => u,
                None => continue,
            };
            // dedup key: message.id, else top-level uuid; skip if neither.
            let key = msg
                .get("id")
                .and_then(|i| i.as_str())
                .or_else(|| v.get("uuid").and_then(|u| u.as_str()))
                .map(str::to_string);
            if let Some(k) = &key {
                if !seen.insert(k.clone()) {
                    continue; // already counted this message
                }
            }
            let ts_ms = match v.get("timestamp").and_then(|t| t.as_str()).and_then(parse_ts_ms) {
                Some(ms) => ms,
                None => continue,
            };
            let model = msg.get("model").and_then(|m| m.as_str()).unwrap_or("unknown").to_string();
            let usage = Usage {
                input_tokens: usage_val.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                output_tokens: usage_val.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                cache_read: usage_val.get("cache_read_input_tokens").and_then(|x| x.as_u64()),
                cache_write: usage_val.get("cache_creation_input_tokens").and_then(|x| x.as_u64()),
            };
            entries.push(Entry { ts_ms, model, usage });
        }
    }
    entries
}

fn parse_ts_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.timestamp_millis())
}

fn iso(ms: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(ms).map(|dt| dt.to_rfc3339())
}

/// Aggregate the given entries into a window's token totals + cost.
fn aggregate(entries: &[&Entry]) -> (TokenTotals, f64) {
    let mut tokens = TokenTotals::default();
    let mut cost = 0.0_f64;
    for e in entries {
        let cr = e.usage.cache_read.unwrap_or(0);
        let cw = e.usage.cache_write.unwrap_or(0);
        tokens.add(e.usage.input_tokens, e.usage.output_tokens, cw, cr);
        cost += cost_for(&e.usage, price_for(&e.model));
    }
    (tokens, (cost * 1000.0).round() / 1000.0)
}

/// Build the Claude usage snapshot.
pub fn snapshot() -> Result<UsageSnapshot, WardError> {
    let mut entries: Vec<Entry> = Vec::new();
    for base in base_dirs() {
        entries.extend(parse_entries_from(&base));
    }
    let now = Utc::now().timestamp_millis();
    let generated_at = iso(now).unwrap_or_default();

    if entries.is_empty() {
        return Ok(UsageSnapshot {
            harness: "claude".into(),
            block: UsageWindow::empty(),
            week: UsageWindow::empty(),
            source: UsageSource::Local,
            available: false,
            generated_at,
        });
    }

    entries.sort_by_key(|e| e.ts_ms);
    let ts: Vec<i64> = entries.iter().map(|e| e.ts_ms).collect();

    // Current 5-hour block.
    let block = match blocks::current_block(&ts, now) {
        Some(b) => {
            let in_block: Vec<&Entry> = entries.iter().filter(|e| e.ts_ms >= b.start_ms && e.ts_ms < b.end_ms).collect();
            let (tokens, cost_usd) = aggregate(&in_block);
            UsageWindow {
                tokens,
                cost_usd,
                percent: None, // Claude: no server % locally; countdown only
                resets_at: iso(b.end_ms),
                resets_in_secs: Some(((b.end_ms - now) / 1000).max(0)),
                is_active: b.is_active,
                started_at: iso(b.start_ms),
                plan_type: None,
            }
        }
        None => UsageWindow::empty(),
    };

    // Rolling 7-day week (totals only; Claude has no local weekly reset clock).
    let week_start = now - 7 * 24 * 60 * 60 * 1000;
    let in_week: Vec<&Entry> = entries.iter().filter(|e| e.ts_ms >= week_start).collect();
    let (wtokens, wcost) = aggregate(&in_week);
    let week = UsageWindow {
        tokens: wtokens,
        cost_usd: wcost,
        percent: None,
        resets_at: None,
        resets_in_secs: None,
        is_active: !in_week.is_empty(),
        started_at: iso(week_start),
        plan_type: None,
    };

    Ok(UsageSnapshot {
        harness: "claude".into(),
        block,
        week,
        source: UsageSource::Local,
        available: true,
        generated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_session(base: &Path, name: &str, body: &str) {
        let dir = base.join("projects").join("proj");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(name), body).unwrap();
    }

    fn assistant_line(ts: &str, msg_id: &str, model: &str, inp: u64, out: u64, cr: u64, cw: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","uuid":"u-{msg_id}","message":{{"id":"{msg_id}","model":"{model}","usage":{{"input_tokens":{inp},"output_tokens":{out},"cache_read_input_tokens":{cr},"cache_creation_input_tokens":{cw}}}}}}}"#
        )
    }

    #[test]
    fn parses_and_dedups_by_message_id() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let l = assistant_line("2026-05-14T04:34:54.786Z", "msg_1", "claude-sonnet-4-5", 100, 20, 1000, 500);
        write_session(base, "a.jsonl", &format!("{l}\n{l}\n"));
        let entries = parse_entries_from(base);
        assert_eq!(entries.len(), 1, "duplicate message.id must be deduped");
        assert_eq!(entries[0].usage.input_tokens, 100);
        assert_eq!(entries[0].model, "claude-sonnet-4-5");
    }

    #[test]
    fn skips_non_assistant_and_malformed_lines() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let good = assistant_line("2026-05-14T04:34:54.786Z", "msg_2", "claude-haiku-4-5", 10, 2, 0, 0);
        let body = format!(
            "{}\n{}\n{}\n",
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#,
            "{ broken json",
            good
        );
        write_session(base, "b.jsonl", &body);
        let entries = parse_entries_from(base);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn snapshot_unavailable_when_no_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.path());
        let snap = snapshot().unwrap();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        assert!(!snap.available);
        assert_eq!(snap.harness, "claude");
        assert_eq!(snap.block.tokens.total, 0);
    }

    #[test]
    fn snapshot_aggregates_recent_entries_into_block_and_week() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        // two recent messages (now-ish); use a fixed recent ISO within 5h.
        let now = Utc::now();
        let t1 = (now - chrono::Duration::minutes(30)).to_rfc3339();
        let t2 = (now - chrono::Duration::minutes(10)).to_rfc3339();
        let body = format!(
            "{}\n{}\n",
            assistant_line(&t1, "m1", "claude-sonnet-4-5", 1_000_000, 0, 0, 0),
            assistant_line(&t2, "m2", "claude-sonnet-4-5", 0, 100_000, 0, 0),
        );
        write_session(base, "c.jsonl", &body);
        std::env::set_var("CLAUDE_CONFIG_DIR", base);
        let snap = snapshot().unwrap();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        assert!(snap.available);
        assert!(snap.block.is_active, "recent activity → active block");
        assert_eq!(snap.block.tokens.input, 1_000_000);
        assert_eq!(snap.block.tokens.output, 100_000);
        assert_eq!(snap.block.tokens.total, 1_100_000);
        // sonnet: 1M input @ $3 + 100k output @ $15 = $4.50
        assert!((snap.block.cost_usd - 4.5).abs() < 1e-6, "got {}", snap.block.cost_usd);
        assert!(snap.block.resets_in_secs.unwrap() > 0);
        assert!(snap.block.percent.is_none(), "Claude has no local percent");
        assert_eq!(snap.week.tokens.total, 1_100_000);
    }
}
```

- [ ] **Step 6: Run the Claude tests to verify they pass**

Run: `cd src-tauri && cargo test --lib usage::claude::tests`
Expected: PASS (4 tests). Then `cargo test --lib sessions::cost::tests` still PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/sessions/cost.rs src-tauri/src/usage/claude.rs
git commit -m "feat(plan14): Claude usage engine — JSONL dedup + 5h block + weekly + cost"
```

---

## Task 3: Codex usage engine

Parse `~/.codex/sessions/**/rollout-*.jsonl` `token_count` events, bucket `total_token_usage` **deltas** by event timestamp into the 5-hour and weekly windows (bounds derived from the real `rate_limits.resets_at`), and read the authoritative `used_percent` / `resets_at` / `plan_type`.

**Files:**
- Replace: `src-tauri/src/usage/codex.rs`
- Test: same file (synthetic rollout JSONL in tempdirs)

**Interfaces:**
- Consumes: `TokenTotals`, `UsageWindow`, `UsageSnapshot`, `UsageSource`; `sessions::cost::{price_for, cost_for}`; `sessions::parse::Usage`.
- Produces: `usage::codex::snapshot() -> Result<UsageSnapshot, WardError>`; internal `collect_events_from(base: &Path) -> Vec<Event>` test seam.

- [ ] **Step 1: Write the failing tests for Codex usage**

Replace `src-tauri/src/usage/codex.rs` with tests + stubs:

```rust
//! Plan 14 — Codex usage. (WIP — Step 3.)
use std::path::Path;
use crate::error::WardError;
use super::UsageSnapshot;

pub(crate) struct Event {
    pub ts_ms: i64,
    pub cumulative_total: u64,
    pub cumulative_input: u64,
    pub cumulative_output: u64,
    pub cumulative_cached: u64,
    pub rate_limits: Option<RateLimits>,
}

#[derive(Clone)]
pub(crate) struct RateLimits {
    pub primary_percent: f64,
    pub primary_window_min: i64,
    pub primary_resets_at: i64,
    pub secondary_percent: f64,
    pub secondary_window_min: i64,
    pub secondary_resets_at: i64,
    pub plan_type: Option<String>,
}

pub(crate) fn collect_events_from(_base: &Path) -> Vec<Event> { unimplemented!() }
pub fn snapshot() -> Result<UsageSnapshot, WardError> { unimplemented!() }

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_rollout(base: &Path, day: &str, name: &str, body: &str) {
        let dir = base.join("sessions").join(day);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(name), body).unwrap();
    }

    fn tc_line(ts: &str, total: u64, inp: u64, out: u64, cached: u64, p_pct: f64, p_reset: i64, s_pct: f64, s_reset: i64) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{inp},"cached_input_tokens":{cached},"output_tokens":{out},"reasoning_output_tokens":0,"total_tokens":{total}}},"last_token_usage":{{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":0}}}},"rate_limits":{{"primary":{{"used_percent":{p_pct},"window_minutes":300,"resets_at":{p_reset}}},"secondary":{{"used_percent":{s_pct},"window_minutes":10080,"resets_at":{s_reset}}},"plan_type":"plus"}}}}}}"#
        )
    }

    #[test]
    fn collects_token_count_events_only() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let body = format!(
            "{}\n{}\n{}\n",
            r#"{"timestamp":"2026-05-14T00:00:00.000Z","type":"session_meta","payload":{}}"#,
            tc_line("2026-05-14T00:00:01.000Z", 100, 80, 20, 0, 5.0, 1778067464, 10.0, 1778305419),
            r#"{"timestamp":"2026-05-14T00:00:02.000Z","type":"response_item","payload":{"type":"message"}}"#,
        );
        write_rollout(base, "2026/05/14", "rollout-2026-05-14T00-00-00-abc.jsonl", &body);
        let events = collect_events_from(base);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].cumulative_total, 100);
        assert!(events[0].rate_limits.is_some());
    }

    #[test]
    fn snapshot_unavailable_when_no_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CODEX_HOME", dir.path());
        let snap = snapshot().unwrap();
        std::env::remove_var("CODEX_HOME");
        assert!(!snap.available);
        assert_eq!(snap.harness, "codex");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd src-tauri && cargo test --lib usage::codex::tests`
Expected: FAIL — `not implemented`.

- [ ] **Step 3: Implement the Codex usage engine**

Replace `src-tauri/src/usage/codex.rs` with:

```rust
//! Plan 14 — Codex CLI usage from `~/.codex/sessions/**/rollout-*.jsonl`.
//!
//! Codex writes `token_count` events carrying CUMULATIVE `total_token_usage`
//! plus a real `rate_limits` payload (primary = rolling 5-hour window,
//! secondary = weekly). We bucket per-turn token DELTAS (of the cumulative
//! totals — never `last_token_usage`, which Codex re-emits stale) by event
//! timestamp into the windows the rate-limit resets define, and read the
//! authoritative `used_percent` / `resets_at` / `plan_type`. No network.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::WardError;
use crate::sessions::cost::{cost_for, price_for};
use crate::sessions::parse::Usage;

use super::{TokenTotals, UsageSnapshot, UsageSource, UsageWindow};

#[derive(Clone)]
pub(crate) struct RateLimits {
    pub primary_percent: f64,
    pub primary_window_min: i64,
    pub primary_resets_at: i64,
    pub secondary_percent: f64,
    pub secondary_window_min: i64,
    pub secondary_resets_at: i64,
    pub plan_type: Option<String>,
}

pub(crate) struct Event {
    pub ts_ms: i64,
    pub cumulative_total: u64,
    pub cumulative_input: u64,
    pub cumulative_output: u64,
    pub cumulative_cached: u64,
    pub rate_limits: Option<RateLimits>,
}

fn base_dirs() -> Vec<PathBuf> {
    if let Ok(v) = std::env::var("CODEX_HOME") {
        let dirs: Vec<PathBuf> = v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).collect();
        if !dirs.is_empty() {
            return dirs;
        }
    }
    dirs::home_dir().map(|h| vec![h.join(".codex")]).unwrap_or_default()
}

fn collect_rollouts(dir: &Path, out: &mut Vec<PathBuf>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_rollouts(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("jsonl")
            && p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("rollout-")).unwrap_or(false)
        {
            out.push(p);
        }
    }
}

fn parse_ts_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.timestamp_millis())
}

fn iso_secs(secs: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp(secs, 0).map(|dt| dt.to_rfc3339())
}

fn parse_rate_limits(rl: &Value) -> Option<RateLimits> {
    let obj = rl.as_object()?;
    let p = obj.get("primary")?.as_object()?;
    let s = obj.get("secondary")?.as_object()?;
    Some(RateLimits {
        primary_percent: p.get("used_percent").and_then(|x| x.as_f64()).unwrap_or(0.0),
        primary_window_min: p.get("window_minutes").and_then(|x| x.as_i64()).unwrap_or(300),
        primary_resets_at: p.get("resets_at").and_then(|x| x.as_i64()).unwrap_or(0),
        secondary_percent: s.get("used_percent").and_then(|x| x.as_f64()).unwrap_or(0.0),
        secondary_window_min: s.get("window_minutes").and_then(|x| x.as_i64()).unwrap_or(10080),
        secondary_resets_at: s.get("resets_at").and_then(|x| x.as_i64()).unwrap_or(0),
        plan_type: obj.get("plan_type").and_then(|x| x.as_str()).map(str::to_string),
    })
}

/// Collect all `token_count` events (with cumulative totals + rate_limits)
/// from every rollout file under `base/sessions`, sorted by timestamp.
pub(crate) fn collect_events_from(base: &Path) -> Vec<Event> {
    let mut files = Vec::new();
    collect_rollouts(&base.join("sessions"), &mut files);
    // archived sessions too, if present
    collect_rollouts(&base.join("archived_sessions"), &mut files);
    files.sort();

    let mut events: Vec<Event> = Vec::new();
    for f in files {
        let content = match std::fs::read_to_string(&f) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let payload = match v.get("payload") {
                Some(p) => p,
                None => continue,
            };
            if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
                continue;
            }
            let ts_ms = match v.get("timestamp").and_then(|t| t.as_str()).and_then(parse_ts_ms) {
                Some(ms) => ms,
                None => continue,
            };
            let tot = payload.get("info").and_then(|i| i.get("total_token_usage"));
            let (total, input, output, cached) = match tot.and_then(|t| t.as_object()) {
                Some(t) => (
                    t.get("total_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                    t.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                    t.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                    t.get("cached_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                ),
                None => (0, 0, 0, 0),
            };
            let rate_limits = payload.get("rate_limits").and_then(parse_rate_limits);
            events.push(Event {
                ts_ms,
                cumulative_total: total,
                cumulative_input: input,
                cumulative_output: output,
                cumulative_cached: cached,
                rate_limits,
            });
        }
    }
    events.sort_by_key(|e| e.ts_ms);
    events
}

/// Per-turn delta of cumulative totals, keyed by event timestamp. Resets to
/// the event's own value when cumulative drops (a new session started).
struct Delta { ts_ms: i64, input: u64, output: u64, cached: u64, model_tokens: u64 }

fn deltas(events: &[Event]) -> Vec<Delta> {
    let mut out = Vec::with_capacity(events.len());
    let (mut pt, mut pi, mut po, mut pc) = (0u64, 0u64, 0u64, 0u64);
    for e in events {
        // A cumulative total lower than the previous means a new session.
        let reset = e.cumulative_total < pt;
        let base_t = if reset { 0 } else { pt };
        let base_i = if reset { 0 } else { pi };
        let base_o = if reset { 0 } else { po };
        let base_c = if reset { 0 } else { pc };
        out.push(Delta {
            ts_ms: e.ts_ms,
            input: e.cumulative_input.saturating_sub(base_i),
            output: e.cumulative_output.saturating_sub(base_o),
            cached: e.cumulative_cached.saturating_sub(base_c),
            model_tokens: e.cumulative_total.saturating_sub(base_t),
        });
        pt = e.cumulative_total;
        pi = e.cumulative_input;
        po = e.cumulative_output;
        pc = e.cumulative_cached;
    }
    out
}

/// Sum deltas whose timestamp falls in `[start_ms, end_ms]` into a window.
fn window_from(deltas: &[Delta], start_ms: i64, end_ms: i64, percent: f64, resets_at_secs: i64, now_ms: i64, plan_type: Option<String>) -> UsageWindow {
    let mut tokens = TokenTotals::default();
    for d in deltas {
        if d.ts_ms >= start_ms && d.ts_ms <= end_ms {
            // Codex's cumulative split doesn't isolate cache-creation; treat
            // non-cached input as input, cached as cache_read, output as output.
            let cache_read = d.cached;
            let input = d.input.saturating_sub(d.cached);
            tokens.add(input, d.output, 0, cache_read);
        }
    }
    // Cost via the shared pricing table (Codex models fall through to the
    // estimate, labeled the same way `sessions::cost` labels unknown models).
    let usage = Usage {
        input_tokens: tokens.input,
        output_tokens: tokens.output,
        cache_read: Some(tokens.cache_read),
        cache_write: None,
    };
    let cost = cost_for(&usage, price_for("codex"));
    let resets_at_ms = resets_at_secs * 1000;
    UsageWindow {
        tokens,
        cost_usd: (cost * 1000.0).round() / 1000.0,
        percent: Some((percent / 100.0).clamp(0.0, 1.0)),
        resets_at: iso_secs(resets_at_secs),
        resets_in_secs: Some(((resets_at_ms - now_ms) / 1000).max(0)),
        is_active: percent < 100.0 && now_ms < resets_at_ms,
        started_at: iso_secs(resets_at_secs - (end_ms - start_ms) / 1000),
        plan_type,
    }
}

pub fn snapshot() -> Result<UsageSnapshot, WardError> {
    let mut events: Vec<Event> = Vec::new();
    for base in base_dirs() {
        events.extend(collect_events_from(&base));
    }
    events.sort_by_key(|e| e.ts_ms);
    let now = Utc::now().timestamp_millis();
    let generated_at = DateTime::<Utc>::from_timestamp_millis(now).map(|d| d.to_rfc3339()).unwrap_or_default();

    // The most recent event carrying rate_limits gives the current window state.
    let latest_rl = events.iter().rev().find_map(|e| e.rate_limits.clone());
    let rl = match latest_rl {
        Some(rl) => rl,
        None => {
            return Ok(UsageSnapshot {
                harness: "codex".into(),
                block: UsageWindow::empty(),
                week: UsageWindow::empty(),
                source: UsageSource::RateLimits,
                available: false,
                generated_at,
            });
        }
    };

    let ds = deltas(&events);
    let p_end = rl.primary_resets_at * 1000;
    let p_start = p_end - rl.primary_window_min * 60 * 1000;
    let s_end = rl.secondary_resets_at * 1000;
    let s_start = s_end - rl.secondary_window_min * 60 * 1000;

    let block = window_from(&ds, p_start, p_end, rl.primary_percent, rl.primary_resets_at, now, rl.plan_type.clone());
    let week = window_from(&ds, s_start, s_end, rl.secondary_percent, rl.secondary_resets_at, now, rl.plan_type.clone());

    Ok(UsageSnapshot {
        harness: "codex".into(),
        block,
        week,
        source: UsageSource::RateLimits,
        available: true,
        generated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_rollout(base: &Path, day: &str, name: &str, body: &str) {
        let dir = base.join("sessions").join(day);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(name), body).unwrap();
    }

    fn tc_line(ts: &str, total: u64, inp: u64, out: u64, cached: u64, p_pct: f64, p_reset: i64, s_pct: f64, s_reset: i64) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{inp},"cached_input_tokens":{cached},"output_tokens":{out},"reasoning_output_tokens":0,"total_tokens":{total}}},"last_token_usage":{{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":0}}}},"rate_limits":{{"primary":{{"used_percent":{p_pct},"window_minutes":300,"resets_at":{p_reset}}},"secondary":{{"used_percent":{s_pct},"window_minutes":10080,"resets_at":{s_reset}}},"plan_type":"plus"}}}}}}"#
        )
    }

    #[test]
    fn collects_token_count_events_only() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let body = format!(
            "{}\n{}\n{}\n",
            r#"{"timestamp":"2026-05-14T00:00:00.000Z","type":"session_meta","payload":{}}"#,
            tc_line("2026-05-14T00:00:01.000Z", 100, 80, 20, 0, 5.0, 1778067464, 10.0, 1778305419),
            r#"{"timestamp":"2026-05-14T00:00:02.000Z","type":"response_item","payload":{"type":"message"}}"#,
        );
        write_rollout(base, "2026/05/14", "rollout-2026-05-14T00-00-00-abc.jsonl", &body);
        let events = collect_events_from(base);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].cumulative_total, 100);
        assert!(events[0].rate_limits.is_some());
    }

    #[test]
    fn snapshot_unavailable_when_no_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CODEX_HOME", dir.path());
        let snap = snapshot().unwrap();
        std::env::remove_var("CODEX_HOME");
        assert!(!snap.available);
        assert_eq!(snap.harness, "codex");
    }

    #[test]
    fn snapshot_reads_real_percent_and_buckets_deltas() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        // Windows anchored so `now` sits inside them: reset 1h in the future.
        let now = Utc::now();
        let p_reset = (now + chrono::Duration::hours(1)).timestamp();
        let s_reset = (now + chrono::Duration::days(3)).timestamp();
        // Two events 10 min apart, both inside the 5h window; cumulative 100 → 260.
        let t1 = (now - chrono::Duration::minutes(20)).to_rfc3339();
        let t2 = (now - chrono::Duration::minutes(10)).to_rfc3339();
        let body = format!(
            "{}\n{}\n",
            tc_line(&t1, 100, 100, 0, 0, 8.46, p_reset, 16.69, s_reset),
            tc_line(&t2, 260, 240, 20, 40, 12.0, p_reset, 18.0, s_reset),
        );
        write_rollout(base, "2026/05/14", "rollout-2026-05-14T00-00-00-xyz.jsonl", &body);
        std::env::set_var("CODEX_HOME", base);
        let snap = snapshot().unwrap();
        std::env::remove_var("CODEX_HOME");

        assert!(snap.available);
        assert_eq!(snap.source, UsageSource::RateLimits);
        // Latest event's rate_limits win: primary 12% → 0.12
        assert!((snap.block.percent.unwrap() - 0.12).abs() < 1e-9);
        assert!((snap.week.percent.unwrap() - 0.18).abs() < 1e-9);
        assert_eq!(snap.block.plan_type.as_deref(), Some("plus"));
        assert!(snap.block.resets_in_secs.unwrap() > 0);
        // Delta tokens across the two events: total 100 + 160 = 260.
        assert_eq!(snap.block.tokens.total, 260);
        // Cumulative deltas: e1 input=100, cached=0; e2 input delta=140, cached=40.
        // window_from splits input = raw_input - cached, cache_read = cached:
        //   e1 → input 100, cache_read 0, output 0
        //   e2 → input 100 (140-40), cache_read 40, output 20
        // totals: input 200, output 20, cache_read 40 (200+20+40 = 260).
        assert_eq!(snap.block.tokens.input, 200);
        assert_eq!(snap.block.tokens.output, 20);
        assert_eq!(snap.block.tokens.cache_read, 40);
    }
}
```

- [ ] **Step 4: Run the Codex tests to verify they pass**

Run: `cd src-tauri && cargo test --lib usage::codex::tests`
Expected: PASS (3 tests). If the delta-split assertions in `snapshot_reads_real_percent_and_buckets_deltas` reveal an off-by-one in your `deltas`/`window_from` split, fix the implementation (not the test's documented arithmetic) until the totals match: `total 260`, `output 20`, `cache_read 40`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/usage/codex.rs
git commit -m "feat(plan14): Codex usage engine — token_count deltas + real rate_limits"
```

---

## Task 4: `usage_snapshot` command + wiring + attribution

Expose the engine as a Tauri command, add the dispatcher, and record the ccusage attribution in `NOTICE`.

**Files:**
- Modify: `src-tauri/src/usage/mod.rs` (add `usage_snapshot` dispatcher)
- Modify: `src-tauri/src/commands.rs` (add the `usage_snapshot` command)
- Modify: `src-tauri/src/lib.rs` (register in `generate_handler!`)
- Modify: `NOTICE` (attribution)
- Test: `src-tauri/src/usage/mod.rs` (dispatcher test)

**Interfaces:**
- Consumes: `claude::snapshot`, `codex::snapshot`.
- Produces: `usage::usage_snapshot(harness: &str) -> Result<UsageSnapshot, WardError>`; Tauri command `usage_snapshot(harness: String) -> Result<UsageSnapshot, WardError>` (Plan 15 wraps as `api.usageSnapshot(harness)`).

- [ ] **Step 1: Write the failing dispatcher test**

Add to the `#[cfg(test)] mod tests` in `src-tauri/src/usage/mod.rs`:

```rust
    #[test]
    fn dispatch_unknown_harness_errors() {
        let err = usage_snapshot("nope").unwrap_err();
        assert!(matches!(err, crate::error::WardError::HarnessUnavailable(_)));
    }

    #[test]
    fn dispatch_codex_no_dir_is_unavailable_not_error() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CODEX_HOME", dir.path());
        let snap = usage_snapshot("codex").unwrap();
        std::env::remove_var("CODEX_HOME");
        assert_eq!(snap.harness, "codex");
        assert!(!snap.available);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cd src-tauri && cargo test --lib usage::tests::dispatch`
Expected: FAIL — `usage_snapshot` not found.

- [ ] **Step 3: Add the dispatcher**

In `src-tauri/src/usage/mod.rs`, after the model definitions and before the tests, add:

```rust
use crate::error::WardError;

/// Dispatch to the per-harness usage engine. Unknown harness → error;
/// a known harness with no data → `available: false` (not an error).
pub fn usage_snapshot(harness: &str) -> Result<UsageSnapshot, WardError> {
    match harness {
        "claude" => claude::snapshot(),
        "codex" => codex::snapshot(),
        other => Err(WardError::HarnessUnavailable(other.to_string())),
    }
}
```

- [ ] **Step 4: Add the Tauri command**

In `src-tauri/src/commands.rs`, add near the other commands:

```rust
/// Plan 14 — local usage snapshot (tokens/cost/reset) for a harness.
#[tauri::command]
pub fn usage_snapshot(harness: String) -> Result<crate::usage::UsageSnapshot, WardError> {
    crate::usage::usage_snapshot(&harness)
}
```

- [ ] **Step 5: Register the command**

In `src-tauri/src/lib.rs`, inside `tauri::generate_handler![ … ]`, add `commands::usage_snapshot` after `commands::autostart_set` (add a trailing comma to the prior last entry):

```rust
            commands::autostart_set,
            commands::usage_snapshot
```

- [ ] **Step 6: Add attribution**

Append to `NOTICE` (create the entry after the existing CCO block):

```
Ward's usage engine (src-tauri/src/usage/) ports the 5-hour session-block
algorithm and pricing approach from ccusage (https://github.com/ryoppippi/ccusage,
MIT © 2025 ryoppippi) and its Rust ports ccstat (MIT) and ccusage-in-rust
(BSD-3-Clause). No source was copied verbatim; the algorithm was reimplemented
for Ward's data model.
```

- [ ] **Step 7: Run the full suite + typecheck**

Run: `cd src-tauri && cargo test --lib && cargo check`
Expected: all tests PASS (including the new `usage::tests::dispatch_*`), `cargo check` clean.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/usage/mod.rs src-tauri/src/commands.rs src-tauri/src/lib.rs NOTICE
git commit -m "feat(plan14): usage_snapshot command + dispatcher + ccusage attribution"
```

---

## Self-Review

**Spec coverage (design §4–§6, §11 for the engine):**
- 5h-block algorithm (§5) → Task 1 (`blocks.rs`) ✓
- Claude local reconstruction, dedup, weekly totals, cost (§4.1, §6) → Task 2 ✓
- Codex `token_count` deltas + real `rate_limits` percent/reset/plan_type (§4.2) → Task 3 ✓
- `usage_snapshot(harness)` command + camelCase models (§6.1–6.3) → Tasks 1 & 4 ✓
- Local-only, `available:false` on missing dir, parse-tolerant (§10) → Tasks 2/3 ✓
- Attribution (§2) → Task 4 ✓
- **Documented deviations:** (a) reuse `sessions::cost` pricing instead of a `build.rs` LiteLLM fetch (no build-time network; DRY); (b) Claude `percent` is `None` (the spec's optional plan-limit config in §6.4 is deferred — Claude shows tokens/cost + countdown honestly); (c) Codex per-window token bucketing uses cumulative-delta by event timestamp within the rate-limit window bounds — the authoritative signal remains `used_percent`.

**Placeholder scan:** the `unimplemented!()` bodies are transient RED-state, each replaced in the same task before its commit. No `TODO`/`TBD` remain.

**Type consistency:** `UsageSnapshot`/`UsageWindow`/`TokenTotals`/`UsageSource` are defined once in `mod.rs` and consumed unchanged by `claude.rs`/`codex.rs`/`commands.rs`. `blocks::{current_block, identify_blocks, floor_to_hour, SESSION_MS, BlockInfo}` signatures match between definition and Task 2 use. `sessions::cost::{price_for, cost_for}` are made `pub(crate)` in Task 2 before first use. Command name `usage_snapshot` matches between `commands.rs` and `generate_handler!`. `TokenTotals::add(input, output, cache_creation, cache_read)` argument order is identical at every call site.
