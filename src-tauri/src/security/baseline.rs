//! security/baseline.rs — Layer 3 of the 4-layer security pipeline.
//!
//! Stashes a SHA-256 baseline of each MCP server's tool set under
//! `~/.ward/security/baselines.json` and diffs current scans against
//! it to surface changed, added, or removed tools. JSON shape and
//! semantics mirror CCO `baselines.json` so dashboards stay portable.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::WardError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BaselineEntry {
    pub tool_hashes: HashMap<String, String>,
    pub accepted_at: DateTime<Utc>,
    pub accepted_findings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Baseline {
    #[serde(default)]
    pub servers: HashMap<String, BaselineEntry>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BaselineChange {
    Added,
    Removed,
    Changed,
    Unchanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BaselineDiff {
    pub server: String,
    pub tool: String,
    pub change: BaselineChange,
}

/// Default canonical path: `~/.ward/security/baselines.json`.
/// Distinct from CCO which uses `~/.claude/.cco-security` — we keep
/// Ward state under `~/.ward/`.
pub fn default_path() -> Result<PathBuf, WardError> {
    let home = dirs::home_dir()
        .ok_or_else(|| WardError::NotFound("home directory".into()))?;
    Ok(home.join(".ward").join("security").join("baselines.json"))
}

/// Load the baseline file at `path`. A missing or empty file returns
/// a default (empty) baseline — no error.
pub fn load(path: &Path) -> Result<Baseline, WardError> {
    if !path.exists() { return Ok(Baseline::default()); }
    let bytes = fs::read(path)?;
    if bytes.is_empty() { return Ok(Baseline::default()); }
    serde_json::from_slice(&bytes).map_err(|e| WardError::NotFound(format!("json: {e}")))
}

/// Atomically write the baseline. Creates `path.parent()` as needed.
pub fn save(path: &Path, baseline: &Baseline) -> Result<(), WardError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(baseline).map_err(|e| WardError::NotFound(format!("json: {e}")))?;
    fs::write(path, bytes)?;
    Ok(())
}

/// Compare two baselines. Returns one `BaselineDiff` per tool across
/// every server in either input. Tools not in either side classify as
/// Added or Removed; tools whose hash changed classify as Changed.
pub fn diff(old: &Baseline, new: &Baseline) -> Vec<BaselineDiff> {
    let mut out = Vec::new();
    let servers: BTreeSet<&String> =
        old.servers.keys().chain(new.servers.keys()).collect();
    for server in servers {
        let prev = old.servers.get(server).map(|e| &e.tool_hashes);
        let curr = new.servers.get(server).map(|e| &e.tool_hashes);
        match (prev, curr) {
            (None, Some(c)) => {
                for t in c.keys() {
                    out.push(BaselineDiff {
                        server: server.clone(),
                        tool: t.clone(),
                        change: BaselineChange::Added,
                    });
                }
            }
            (Some(p), None) => {
                for t in p.keys() {
                    out.push(BaselineDiff {
                        server: server.clone(),
                        tool: t.clone(),
                        change: BaselineChange::Removed,
                    });
                }
            }
            (Some(p), Some(c)) => {
                let tools: BTreeSet<&String> =
                    p.keys().chain(c.keys()).collect();
                for t in tools {
                    let change = match (p.get(t), c.get(t)) {
                        (Some(ph), Some(ch)) if ph != ch => BaselineChange::Changed,
                        (Some(_), Some(_)) => BaselineChange::Unchanged,
                        (None, Some(_)) => BaselineChange::Added,
                        (Some(_), None) => BaselineChange::Removed,
                        _ => BaselineChange::Unchanged,
                    };
                    out.push(BaselineDiff {
                        server: server.clone(),
                        tool: t.clone(),
                        change,
                    });
                }
            }
            (None, None) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_serialize_deserialize() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("baselines.json");
        let mut b = Baseline::default();
        let mut hashes = HashMap::new();
        hashes.insert("echo".to_string(), "abc123".to_string());
        b.servers.insert(
            "github".to_string(),
            BaselineEntry { tool_hashes: hashes, accepted_at: Utc::now(), accepted_findings: vec![] },
        );
        save(&path, &b).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.servers.len(), 1);
        assert_eq!(loaded.servers["github"].tool_hashes["echo"], "abc123");
    }

    #[test]
    fn load_missing_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let b = load(&dir.path().join("missing.json")).unwrap();
        assert!(b.servers.is_empty());
    }

    #[test]
    fn diff_detects_added_changed_removed() {
        let mut h1 = HashMap::new();
        h1.insert("echo".into(), "h1".into());
        h1.insert("old".into(), "ho".into());
        let mut h2 = HashMap::new();
        h2.insert("echo".into(), "h2".into());
        h2.insert("new".into(), "hn".into());
        let mut old = Baseline::default();
        old.servers.insert(
            "s".into(),
            BaselineEntry { tool_hashes: h1, accepted_at: Utc::now(), accepted_findings: vec![] },
        );
        let mut new = Baseline::default();
        new.servers.insert(
            "s".into(),
            BaselineEntry { tool_hashes: h2, accepted_at: Utc::now(), accepted_findings: vec![] },
        );
        let d = diff(&old, &new);
        assert!(d.iter().any(|x| x.tool == "echo" && x.change == BaselineChange::Changed));
        assert!(d.iter().any(|x| x.tool == "old" && x.change == BaselineChange::Removed));
        assert!(d.iter().any(|x| x.tool == "new" && x.change == BaselineChange::Added));
    }
}
