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
struct Delta { ts_ms: i64, input: u64, output: u64, cached: u64 }

fn deltas(events: &[Event]) -> Vec<Delta> {
    let mut out = Vec::with_capacity(events.len());
    let (mut pt, mut pi, mut po, mut pc) = (0u64, 0u64, 0u64, 0u64);
    for e in events {
        // A cumulative total lower than the previous means a new session.
        let reset = e.cumulative_total < pt;
        let base_i = if reset { 0 } else { pi };
        let base_o = if reset { 0 } else { po };
        let base_c = if reset { 0 } else { pc };
        out.push(Delta {
            ts_ms: e.ts_ms,
            input: e.cumulative_input.saturating_sub(base_i),
            output: e.cumulative_output.saturating_sub(base_o),
            cached: e.cumulative_cached.saturating_sub(base_c),
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
        let _env = crate::usage::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CODEX_HOME", dir.path());
        let snap = snapshot().unwrap();
        std::env::remove_var("CODEX_HOME");
        assert!(!snap.available);
        assert_eq!(snap.harness, "codex");
    }

    #[test]
    fn snapshot_reads_real_percent_and_buckets_deltas() {
        let _env = crate::usage::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
