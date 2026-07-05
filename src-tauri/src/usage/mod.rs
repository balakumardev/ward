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
