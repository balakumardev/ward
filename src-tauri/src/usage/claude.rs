//! Plan 14 — Claude Code usage from `~/.claude/projects/**/*.jsonl`.
//!
//! Reads assistant lines (timestamp + message.id + model + usage), dedupes
//! by `message.id` (fallback top-level `uuid`), reconstructs the current
//! 5-hour block for the reset countdown, and sums a rolling 7-day week.
//! Cost reuses `sessions::cost` pricing. No network, no credentials.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

/// Recursively collect `*.jsonl` under `dir` into `out` (sorted by the caller),
/// skipping files last modified before `cutoff` — a file untouched in the last
/// ~8 days can hold no entry inside the current 5h block or the rolling week.
fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>, cutoff: SystemTime) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_jsonl(&p, out, cutoff);
        } else if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            match entry.metadata() {
                Ok(md) if !super::file_is_recent(&md, cutoff) => continue,
                _ => out.push(p),
            }
        }
    }
}

/// Parse + dedup all usage entries under one base dir's `projects/`.
pub(crate) fn parse_entries_from(base: &Path) -> Vec<Entry> {
    let mut files = Vec::new();
    let cutoff = super::recent_cutoff(SystemTime::now());
    collect_jsonl(&base.join("projects"), &mut files, cutoff);
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
            let ts_ms = match v.get("timestamp").and_then(|t| t.as_str()).and_then(parse_ts_ms) {
                Some(ms) => ms,
                None => continue,
            };
            // Claim the dedup id only AFTER the timestamp guard: a first-seen line
            // with a missing/unparseable timestamp is skipped above and must not
            // insert its id into `seen`, or it would shadow a later valid line
            // carrying the same `message.id`.
            if let Some(k) = &key {
                if !seen.insert(k.clone()) {
                    continue; // already counted this message
                }
            }
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
    fn parse_skips_files_older_than_window() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        // Two files with distinct valid assistant lines; only the recent-mtime
        // one survives once the old file is backdated past the 8-day window.
        write_session(base, "recent.jsonl", &format!("{}\n", assistant_line("2026-07-05T10:00:00.000Z", "recent", "claude-sonnet-4-5", 10, 2, 0, 0)));
        write_session(base, "old.jsonl", &format!("{}\n", assistant_line("2026-07-05T10:00:00.000Z", "old", "claude-sonnet-4-5", 10, 2, 0, 0)));
        let old_path = base.join("projects").join("proj").join("old.jsonl");
        let thirty_days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 24 * 60 * 60);
        std::fs::File::options().write(true).open(&old_path).unwrap().set_modified(thirty_days_ago).unwrap();

        let entries = parse_entries_from(base);
        assert_eq!(entries.len(), 1, "file untouched for 30 days is skipped");
    }

    #[test]
    fn snapshot_unavailable_when_no_dir() {
        let _env = crate::usage::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _env = crate::usage::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
