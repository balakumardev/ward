//! Plan 14 — local usage engine: Claude & Codex token/cost/reset from the
//! session files on disk. No network, no credential reading.
//!
//! `usage_snapshot(harness)` (Task 4) dispatches to `claude` / `codex`.

pub mod blocks;
pub mod claude;
pub mod codex;
pub mod live;

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// Session files older than this are skipped when reconstructing usage: the
/// current 5-hour block and the rolling 7-day week can only reference recent
/// files. 8 days = the 7-day week plus a day of slack for clock/timezone skew.
/// Heavy Claude users accumulate thousands of session files; without this the
/// popover re-read every one on each open (the menu-bar lag the user hit).
pub(crate) const RECENT_WINDOW_SECS: u64 = 8 * 24 * 60 * 60;

/// The mtime floor for [`file_is_recent`]: files modified before this are
/// skipped. `now` is threaded in (not read here) so every caller in one scan
/// shares a single clock and tests stay deterministic. Saturates at the UNIX
/// epoch (an 8-day subtraction never underflows, but be safe).
pub(crate) fn recent_cutoff(now: SystemTime) -> SystemTime {
    now.checked_sub(Duration::from_secs(RECENT_WINDOW_SECS)).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// True if `meta`'s modification time is at or after `cutoff`. A file whose
/// mtime can't be read is KEPT (returns `true`): we never drop a file we can't
/// stat — correct totals matter more than the perf shortcut.
pub(crate) fn file_is_recent(meta: &std::fs::Metadata, cutoff: SystemTime) -> bool {
    meta.modified().map(|m| m >= cutoff).unwrap_or(true)
}

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

/// Where a window's numbers came from: reconstructed locally from session files
/// (Claude local), the harness's own on-disk rate-limit payload (Codex), or a
/// live gated call to the provider's rate-limit endpoint (Claude live).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UsageSource {
    Local,
    RateLimits,
    Live,
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

/// Test-only lock serializing env-var mutation across the usage submodule
/// tests (`CLAUDE_CONFIG_DIR` / `CODEX_HOME`) so Rust's parallel test runner
/// can't race two tests that set/read the same process-global var.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    #[test]
    fn file_is_recent_filters_on_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let recent = dir.path().join("recent.txt");
        std::fs::write(&recent, "x").unwrap();
        let old = dir.path().join("old.txt");
        std::fs::write(&old, "x").unwrap();
        // Backdate `old` to 30 days ago (well outside the 8-day window).
        let thirty_days_ago = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(thirty_days_ago).unwrap();

        let cutoff = recent_cutoff(SystemTime::now());
        assert!(file_is_recent(&std::fs::metadata(&recent).unwrap(), cutoff), "just-written file is recent");
        assert!(!file_is_recent(&std::fs::metadata(&old).unwrap(), cutoff), "30-day-old file is filtered out");
    }

    #[test]
    fn dispatch_unknown_harness_errors() {
        let err = usage_snapshot("nope").unwrap_err();
        assert!(matches!(err, crate::error::WardError::HarnessUnavailable(_)));
    }

    #[test]
    fn dispatch_codex_no_dir_is_unavailable_not_error() {
        let _env = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CODEX_HOME", dir.path());
        let snap = usage_snapshot("codex").unwrap();
        std::env::remove_var("CODEX_HOME");
        assert_eq!(snap.harness, "codex");
        assert!(!snap.available);
    }
}
