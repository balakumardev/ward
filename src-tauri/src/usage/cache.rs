//! Plan 17 — persistent usage cache: the last-known [`UsageSnapshot`] per
//! harness, stored at `~/.ward/usage-cache.json`, so the tray popover can paint
//! the previous gauges INSTANTLY while a fresh snapshot loads in the background
//! (stale-while-revalidate). The popover used to open with empty gauges and
//! block on a 2–3 s live round-trip; a warm cache removes both.
//!
//! Two hard guarantees:
//!   * Reads NEVER error — a missing or corrupt file yields an empty cache, so a
//!     first run or a truncated write simply falls back to the live path.
//!   * Writes are ATOMIC — a temp file is written inside `~/.ward/` then renamed
//!     over the target. The rename is atomic on one filesystem, so a crash
//!     mid-write leaves either the old cache or the new one, never a torn file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::UsageSnapshot;
use crate::error::WardError;

/// One cached snapshot plus when it was fetched (ISO-8601, informational).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CacheEntry {
    pub snapshot: UsageSnapshot,
    pub fetched_at: String,
}

/// Last-known snapshot per harness. Either side is `None` until first fetched.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageCache {
    #[serde(default)]
    pub claude: Option<CacheEntry>,
    #[serde(default)]
    pub codex: Option<CacheEntry>,
}

impl UsageCache {
    /// The cached entry for `harness`, if any. Unknown harness → `None`.
    pub fn entry(&self, harness: &str) -> Option<&CacheEntry> {
        match harness {
            "claude" => self.claude.as_ref(),
            "codex" => self.codex.as_ref(),
            _ => None,
        }
    }

    /// Replace the entry for `harness`. Unknown harness is a no-op.
    fn set(&mut self, harness: &str, entry: CacheEntry) {
        match harness {
            "claude" => self.claude = Some(entry),
            "codex" => self.codex = Some(entry),
            _ => {}
        }
    }
}

/// `~/.ward/usage-cache.json` under `home`. Mirrors how the `first-run` /
/// `live-usage-enabled` sentinels resolve their `~/.ward/` dir.
fn cache_path_in(home: &Path) -> PathBuf {
    home.join(".ward").join("usage-cache.json")
}

/// Read the cache under `home`. Missing OR corrupt file → empty cache; this
/// never errors, so callers always get a usable value.
fn read_cache_at(home: &Path) -> UsageCache {
    match std::fs::read_to_string(cache_path_in(home)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => UsageCache::default(),
    }
}

/// Atomically persist `snap` as the last-known entry for `harness` under `home`:
/// merge it into the existing cache, write a temp file inside `~/.ward/`, then
/// rename it over the target. Same-dir rename → same filesystem → atomic, so a
/// crash mid-write can't corrupt the cache. Creates `~/.ward/` if absent;
/// unknown harness is a no-op.
fn write_entry_at(home: &Path, harness: &str, snap: &UsageSnapshot) -> Result<(), WardError> {
    let dir = home.join(".ward");
    std::fs::create_dir_all(&dir)?;

    let mut cache = read_cache_at(home);
    cache.set(
        harness,
        CacheEntry { snapshot: snap.clone(), fetched_at: chrono::Utc::now().to_rfc3339() },
    );

    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    // Temp file in the SAME dir so the rename stays on one filesystem (atomic).
    // The PID keeps two concurrent writers from clobbering each other's temp.
    let tmp = dir.join(format!("usage-cache.json.tmp.{}", std::process::id()));
    std::fs::write(&tmp, json.as_bytes())?;
    std::fs::rename(&tmp, cache_path_in(home))?;
    Ok(())
}

// ── Public API (resolves the real ~/.ward/) ─────────────────────────────────

/// Read the whole cache from `~/.ward/usage-cache.json`. Empty on any error
/// (no home dir, missing/corrupt file).
pub fn read_cache() -> UsageCache {
    dirs::home_dir().map(|h| read_cache_at(&h)).unwrap_or_default()
}

/// The last-known snapshot for `harness`, if the cache holds one.
pub fn cached_snapshot(harness: &str) -> Option<UsageSnapshot> {
    read_cache().entry(harness).map(|e| e.snapshot.clone())
}

/// Persist `snap` as the last-known snapshot for `harness` (atomic write to
/// `~/.ward/usage-cache.json`, creating `~/.ward/` if absent).
pub fn write_entry(harness: &str, snap: &UsageSnapshot) -> Result<(), WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    write_entry_at(&home, harness, snap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{TokenTotals, UsageSource, UsageWindow};

    /// A minimal `available` snapshot whose 5-hour block carries `total` tokens,
    /// so tests can tell two writes apart.
    fn sample(harness: &str, total: u64) -> UsageSnapshot {
        let mut block = UsageWindow::empty();
        block.tokens = TokenTotals { input: total, output: 0, cache_creation: 0, cache_read: 0, total };
        block.is_active = true;
        UsageSnapshot {
            harness: harness.into(),
            block,
            week: UsageWindow::empty(),
            source: UsageSource::Local,
            available: true,
            generated_at: "2026-07-05T00:00:00Z".into(),
        }
    }

    #[test]
    fn missing_file_is_empty_cache_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let c = read_cache_at(dir.path());
        assert_eq!(c, UsageCache::default());
        assert!(c.entry("claude").is_none());
        assert!(c.entry("codex").is_none());
    }

    #[test]
    fn corrupt_file_is_empty_cache_not_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".ward")).unwrap();
        std::fs::write(cache_path_in(dir.path()), b"{ this is not valid json").unwrap();
        // A truncated / garbage cache must degrade to empty, never panic/error.
        assert_eq!(read_cache_at(dir.path()), UsageCache::default());
    }

    #[test]
    fn write_then_read_round_trips_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let snap = sample("claude", 1_244_000);
        write_entry_at(dir.path(), "claude", &snap).unwrap();

        let c = read_cache_at(dir.path());
        let got = c.entry("claude").expect("claude entry present after write");
        assert_eq!(got.snapshot, snap);
        assert!(!got.fetched_at.is_empty(), "fetchedAt stamped");
        // Only the written harness is populated.
        assert!(c.entry("codex").is_none());
    }

    #[test]
    fn writing_one_harness_preserves_the_other() {
        let dir = tempfile::tempdir().unwrap();
        write_entry_at(dir.path(), "claude", &sample("claude", 100)).unwrap();
        write_entry_at(dir.path(), "codex", &sample("codex", 200)).unwrap();

        let c = read_cache_at(dir.path());
        assert_eq!(c.entry("claude").unwrap().snapshot.block.tokens.total, 100);
        assert_eq!(c.entry("codex").unwrap().snapshot.block.tokens.total, 200);
    }

    #[test]
    fn write_overwrites_previous_entry_for_same_harness() {
        let dir = tempfile::tempdir().unwrap();
        write_entry_at(dir.path(), "claude", &sample("claude", 1)).unwrap();
        write_entry_at(dir.path(), "claude", &sample("claude", 999)).unwrap();
        let c = read_cache_at(dir.path());
        assert_eq!(c.entry("claude").unwrap().snapshot.block.tokens.total, 999);
    }

    #[test]
    fn write_creates_ward_dir_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!dir.path().join(".ward").exists(), "~/.ward absent before write");
        write_entry_at(dir.path(), "claude", &sample("claude", 5)).unwrap();

        let ward = dir.path().join(".ward");
        assert!(ward.join("usage-cache.json").exists(), "cache file created");
        // The atomic temp file must have been renamed away, not left behind.
        let leftover: Vec<_> = std::fs::read_dir(&ward)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftover.is_empty(), "no temp file left behind: {leftover:?}");
    }

    #[test]
    fn cache_serializes_camel_case_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        write_entry_at(dir.path(), "claude", &sample("claude", 5)).unwrap();
        let raw = std::fs::read_to_string(cache_path_in(dir.path())).unwrap();
        assert!(raw.contains("\"fetchedAt\""), "camelCase fetchedAt in {raw}");
        assert!(raw.contains("\"snapshot\""));
        assert!(raw.contains("\"claude\""));
    }

    #[test]
    fn entry_unknown_harness_is_none() {
        let dir = tempfile::tempdir().unwrap();
        write_entry_at(dir.path(), "claude", &sample("claude", 5)).unwrap();
        assert!(read_cache_at(dir.path()).entry("nope").is_none());
    }
}
