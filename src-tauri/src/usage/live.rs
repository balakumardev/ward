//! Plan 16 — live Claude subscription usage via the Anthropic rate-limit endpoint.
//!
//! Claude Code writes only raw token counts to `~/.claude`; the real 5-hour and
//! weekly limit percentages live server-side. This module fetches them the same
//! way Claude Code's own `/usage` view (and monitors like claude-monitor) do:
//! one gated `POST /v1/messages` carrying the user's OAuth token, then read the
//! `anthropic-ratelimit-unified-*` response headers (present on 200 AND 429).
//!
//! This is a deliberate, user-opted-in exception to Ward's no-network /
//! no-credential-reading rule, kept tightly gated:
//!   * It runs ONLY when the user enabled it (`~/.ward/live-usage-enabled`
//!     sentinel) AND macOS has granted Ward access to the Keychain item.
//!   * It is triggered by the user (popover open / Refresh) — never on a timer.
//!   * The OAuth token is read from the macOS Keychain, used only as the
//!     Authorization header to api.anthropic.com, and never logged or persisted.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::{TokenTotals, UsageSnapshot, UsageSource, UsageWindow};
use crate::error::WardError;

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Cheapest current model — we send a 1-token ping purely to get the rate-limit
/// headers back; the response body is discarded.
const PING_MODEL: &str = "claude-haiku-4-5-20251001";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// The rate-limit header names (all present on 200 and 429). Utilization values
/// are fractions 0.0..=1.0; reset values are epoch SECONDS.
const H_5H_UTIL: &str = "anthropic-ratelimit-unified-5h-utilization";
const H_5H_RESET: &str = "anthropic-ratelimit-unified-5h-reset";
const H_5H_STATUS: &str = "anthropic-ratelimit-unified-5h-status";
const H_7D_UTIL: &str = "anthropic-ratelimit-unified-7d-utilization";
const H_7D_RESET: &str = "anthropic-ratelimit-unified-7d-reset";
const H_7D_STATUS: &str = "anthropic-ratelimit-unified-7d-status";

// ── Opt-in sentinel ─────────────────────────────────────────────────────────

fn sentinel_in(home: &Path) -> PathBuf {
    home.join(".ward").join("live-usage-enabled")
}

fn live_enabled_at(home: &Path) -> bool {
    sentinel_in(home).exists()
}

fn set_live_enabled_at(home: &Path, on: bool) -> Result<(), WardError> {
    let p = sentinel_in(home);
    if on {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, b"1")?;
    } else if p.exists() {
        std::fs::remove_file(&p)?;
    }
    Ok(())
}

/// Whether the user has enabled the live (network) usage path.
pub fn live_enabled() -> bool {
    dirs::home_dir().map(|h| live_enabled_at(&h)).unwrap_or(false)
}

/// Enable or disable the live usage path (creates/removes the sentinel).
pub fn set_live_enabled(on: bool) -> Result<(), WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::Live("no home directory".into()))?;
    set_live_enabled_at(&home, on)
}

// ── Credentials (macOS Keychain) ────────────────────────────────────────────

/// The OAuth token + plan tier + expiry read from the macOS Keychain.
struct Credentials {
    token: String,
    plan: Option<String>,
    expires_at_ms: Option<i64>,
}

/// Read the Claude Code OAuth credentials from the macOS login Keychain by
/// shelling out to `/usr/bin/security`. macOS shows its own access prompt the
/// first time Ward reads the item — that prompt IS the per-app authorization.
#[cfg(target_os = "macos")]
fn read_credentials() -> Result<Credentials, WardError> {
    let out = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .output()
        .map_err(|e| WardError::Live(format!("could not run security: {e}")))?;
    if !out.status.success() {
        return Err(WardError::Live(
            "Keychain access denied, or no Claude Code credentials found. Sign in to Claude Code first.".into(),
        ));
    }
    let blob = String::from_utf8_lossy(&out.stdout);
    parse_credentials(blob.trim())
}

#[cfg(not(target_os = "macos"))]
fn read_credentials() -> Result<Credentials, WardError> {
    Err(WardError::Live("live usage is only supported on macOS".into()))
}

/// Parse the Keychain blob JSON into credentials. Split out from the Keychain
/// I/O so it can be unit-tested without touching the real Keychain.
fn parse_credentials(blob: &str) -> Result<Credentials, WardError> {
    let v: serde_json::Value =
        serde_json::from_str(blob).map_err(|e| WardError::Live(format!("credentials parse: {e}")))?;
    // Claude Code nests the token under `claudeAiOauth`; tolerate a flat object.
    let oauth = v.get("claudeAiOauth").unwrap_or(&v);
    let token = oauth
        .get("accessToken")
        .and_then(|t| t.as_str())
        .ok_or_else(|| WardError::Live("no access token in credentials".into()))?
        .to_string();
    let plan = oauth.get("subscriptionType").and_then(|t| t.as_str()).map(str::to_string);
    let expires_at_ms = oauth.get("expiresAt").and_then(|t| t.as_i64());
    Ok(Credentials { token, plan, expires_at_ms })
}

// ── Header → snapshot mapping (pure) ─────────────────────────────────────────

fn iso_secs(secs: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp(secs, 0).map(|dt| dt.to_rfc3339())
}

/// Build one usage window from a utilization (0..1), reset epoch-seconds, and
/// status header. `plan` labels the window with the subscription tier.
fn window(util: Option<f64>, reset_secs: Option<i64>, status: Option<&str>, now_ms: i64, plan: Option<String>) -> UsageWindow {
    let percent = util.map(|u| u.clamp(0.0, 1.0));
    let resets_at = reset_secs.and_then(iso_secs);
    let resets_in_secs = reset_secs.map(|r| (r - now_ms / 1000).max(0));
    let is_active = match status {
        Some(s) => s.eq_ignore_ascii_case("allowed"),
        None => percent.map(|p| p < 1.0).unwrap_or(false),
    };
    UsageWindow {
        tokens: TokenTotals::default(), // live headers carry no token counts
        cost_usd: 0.0,
        percent,
        resets_at,
        resets_in_secs,
        is_active,
        started_at: None,
        plan_type: plan,
    }
}

/// Map the rate-limit response headers into a Claude usage snapshot. Pure and
/// fully unit-tested — the network glue just supplies the header map.
fn snapshot_from_headers(headers: &HashMap<String, String>, plan: Option<String>, now_ms: i64) -> UsageSnapshot {
    let get_f = |k: &str| headers.get(k).and_then(|s| s.trim().parse::<f64>().ok());
    let get_i = |k: &str| headers.get(k).and_then(|s| s.trim().parse::<i64>().ok());
    let get_s = |k: &str| headers.get(k).map(|s| s.trim());

    let block = window(get_f(H_5H_UTIL), get_i(H_5H_RESET), get_s(H_5H_STATUS), now_ms, plan.clone());
    let week = window(get_f(H_7D_UTIL), get_i(H_7D_RESET), get_s(H_7D_STATUS), now_ms, plan);
    // Available only if the server actually gave us at least one utilization.
    let available = block.percent.is_some() || week.percent.is_some();
    UsageSnapshot {
        harness: "claude".into(),
        block,
        week,
        source: UsageSource::Live,
        available,
        generated_at: DateTime::<Utc>::from_timestamp_millis(now_ms).map(|d| d.to_rfc3339()).unwrap_or_default(),
    }
}

// ── Network ─────────────────────────────────────────────────────────────────

/// Fetch the rate-limit headers with one gated ping. Reads them on 200 and 429.
fn fetch_headers(token: &str) -> Result<HashMap<String, String>, WardError> {
    let body = serde_json::json!({
        "model": PING_MODEL,
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "hi"}],
    })
    .to_string();

    let resp = ureq::post(ENDPOINT)
        .set("authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", OAUTH_BETA)
        .set("anthropic-version", ANTHROPIC_VERSION)
        .set("content-type", "application/json")
        .send_string(&body);

    let response = match resp {
        Ok(r) => r,
        // A 429 (rate limited) still carries the utilization headers — that's
        // exactly the state we want to show, so read them rather than erroring.
        Err(ureq::Error::Status(code, r)) => {
            if code == 401 || code == 403 {
                return Err(WardError::Live(
                    "Claude session unauthorized. Open Claude Code to refresh your login, then try again.".into(),
                ));
            }
            r
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(WardError::Live(format!("network error reaching Anthropic: {t}")));
        }
    };

    let mut map = HashMap::new();
    for name in [H_5H_UTIL, H_5H_RESET, H_5H_STATUS, H_7D_UTIL, H_7D_RESET, H_7D_STATUS] {
        if let Some(v) = response.header(name) {
            map.insert(name.to_string(), v.to_string());
        }
    }
    if map.is_empty() {
        return Err(WardError::Live("no rate-limit headers in the response".into()));
    }
    Ok(map)
}

/// The gated live Claude usage snapshot. Errors if not opted in, if the Keychain
/// read fails, if the token is expired, or if the request fails.
pub fn snapshot() -> Result<UsageSnapshot, WardError> {
    if !live_enabled() {
        return Err(WardError::Live("live usage not enabled".into()));
    }
    let creds = read_credentials()?;
    let now_ms = Utc::now().timestamp_millis();
    // Catch an expired token before spending a request.
    if let Some(exp) = creds.expires_at_ms {
        if exp <= now_ms {
            return Err(WardError::Live(
                "Claude login has expired. Open Claude Code to refresh it, then try again.".into(),
            ));
        }
    }
    let headers = fetch_headers(&creds.token)?;
    Ok(snapshot_from_headers(&headers, creds.plan, now_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdrs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn snapshot_from_headers_maps_5h_and_week() {
        let now = Utc::now().timestamp_millis();
        let reset_5h = now / 1000 + 3600; // 1h out
        let reset_7d = now / 1000 + 4 * 86400; // 4d out
        let h = hdrs(&[
            (H_5H_UTIL, "0.2"),
            (H_5H_RESET, &reset_5h.to_string()),
            (H_5H_STATUS, "allowed"),
            (H_7D_UTIL, "0.43"),
            (H_7D_RESET, &reset_7d.to_string()),
            (H_7D_STATUS, "allowed"),
        ]);
        let snap = snapshot_from_headers(&h, Some("max".into()), now);
        assert!(snap.available);
        assert_eq!(snap.source, UsageSource::Live);
        assert_eq!(snap.harness, "claude");
        assert!((snap.block.percent.unwrap() - 0.2).abs() < 1e-9);
        assert!((snap.week.percent.unwrap() - 0.43).abs() < 1e-9);
        assert_eq!(snap.block.plan_type.as_deref(), Some("max"));
        assert_eq!(snap.week.plan_type.as_deref(), Some("max"));
        assert!(snap.block.resets_in_secs.unwrap() > 0);
        assert!(snap.block.resets_at.is_some());
        assert!(snap.block.is_active);
        // Live snapshots carry no token counts.
        assert_eq!(snap.block.tokens.total, 0);
    }

    #[test]
    fn snapshot_from_headers_clamps_and_handles_missing() {
        let now = Utc::now().timestamp_millis();
        // Over-100% utilization clamps to 1.0; missing week headers → no percent.
        let h = hdrs(&[(H_5H_UTIL, "1.4"), (H_5H_STATUS, "rejected")]);
        let snap = snapshot_from_headers(&h, None, now);
        assert_eq!(snap.block.percent, Some(1.0));
        assert!(!snap.block.is_active, "rejected status → not active");
        assert!(snap.week.percent.is_none());
        assert!(snap.available, "still available: the 5h percent is present");
        assert!(snap.block.resets_in_secs.is_none(), "no reset header → no countdown");
    }

    #[test]
    fn snapshot_from_headers_unavailable_when_empty() {
        let now = Utc::now().timestamp_millis();
        let snap = snapshot_from_headers(&HashMap::new(), None, now);
        assert!(!snap.available);
        assert!(snap.block.percent.is_none());
        assert!(snap.week.percent.is_none());
    }

    #[test]
    fn parse_credentials_extracts_token_plan_and_expiry() {
        let blob = r#"{"claudeAiOauth":{"accessToken":"tok-abc","subscriptionType":"max","expiresAt":1783273865923,"refreshToken":"r","scopes":["user:inference"]}}"#;
        let c = parse_credentials(blob).unwrap();
        assert_eq!(c.token, "tok-abc");
        assert_eq!(c.plan.as_deref(), Some("max"));
        assert_eq!(c.expires_at_ms, Some(1783273865923));
    }

    #[test]
    fn parse_credentials_errors_without_token() {
        let blob = r#"{"claudeAiOauth":{"subscriptionType":"max"}}"#;
        assert!(parse_credentials(blob).is_err());
    }

    #[test]
    fn sentinel_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!live_enabled_at(dir.path()), "disabled by default");
        set_live_enabled_at(dir.path(), true).unwrap();
        assert!(live_enabled_at(dir.path()), "enabled after opt-in");
        set_live_enabled_at(dir.path(), false).unwrap();
        assert!(!live_enabled_at(dir.path()), "disabled after opt-out");
    }

    /// Full-path smoke: reads the real macOS Keychain and hits the real Anthropic
    /// endpoint. Ignored by default (network + credentials); run manually with:
    ///   cargo test --lib usage::live::tests::live_end_to_end_smoke -- --ignored --nocapture
    #[test]
    #[ignore = "hits the real Keychain + Anthropic API"]
    fn live_end_to_end_smoke() {
        let creds = read_credentials().expect("keychain read");
        let headers = fetch_headers(&creds.token).expect("live fetch");
        let snap = snapshot_from_headers(&headers, creds.plan, Utc::now().timestamp_millis());
        assert!(snap.available, "live snapshot should be available");
        assert!(snap.block.percent.is_some(), "5h percent present");
        eprintln!(
            "LIVE: 5h={:?} week={:?} plan={:?} resets5h={:?}",
            snap.block.percent, snap.week.percent, snap.block.plan_type, snap.block.resets_at
        );
    }
}
