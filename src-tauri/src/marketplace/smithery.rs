//! Smithery MCP registry source (https://api.smithery.ai/servers). No key
//! required for reads. The list endpoint doesn't carry the `deploymentUrl`
//! needed to build an installable remote (that's the per-server detail call),
//! and stdio servers install as `.mcpb` bundles which Ward does not run — so
//! every Smithery search-list entry is `install_shape: "discovery"` with a
//! homepage link for "View".

use std::time::Duration;

use serde_json::Value;

use super::{classify_install_shape, MarketEntry};
use crate::error::WardError;

const SMITHERY_URL: &str = "https://api.smithery.ai/servers";
const TIMEOUT_SECS: u64 = 10;
const PAGE_SIZE: &str = "50";

pub fn parse_servers(body: &str) -> Result<Vec<MarketEntry>, WardError> {
    let root: Value = serde_json::from_str(body)
        .map_err(|e| WardError::Registry(format!("parse smithery response: {e}")))?;
    let mut out = Vec::new();
    if let Some(servers) = root.get("servers").and_then(|v| v.as_array()) {
        for s in servers {
            let name = s.get("qualifiedName").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if name.is_empty() {
                continue;
            }
            let display_name = s
                .get("displayName")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
                .unwrap_or_else(|| name.clone());
            let description = s.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let repo_url = s
                .get("homepage")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string());
            let verified = s.get("verified").and_then(|v| v.as_bool()).unwrap_or(false);
            out.push(MarketEntry {
                kind: "mcp".into(),
                name: name.clone(),
                display_name,
                description,
                source: "smithery".into(),
                version: None,
                verified,
                packages: vec![],
                remotes: vec![],
                install_shape: classify_install_shape(&[], &[]), // "discovery"
                repo_url,
                skill_path: None,
            });
        }
    }
    Ok(out)
}

pub fn search(query: &str) -> Result<Vec<MarketEntry>, WardError> {
    let mut req = ureq::get(SMITHERY_URL)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .query("pageSize", PAGE_SIZE);
    let q = query.trim();
    if !q.is_empty() {
        req = req.query("q", q);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => {
            return Err(WardError::Registry(format!("Smithery returned HTTP {code}")));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(WardError::Registry(format!("network error reaching Smithery: {t}")));
        }
    };
    let body = resp
        .into_string()
        .map_err(|e| WardError::Registry(format!("read smithery response body: {e}")))?;
    parse_servers(&body)
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = include_str!("fixtures/smithery-servers.json");

    #[test]
    fn parses_smithery_entries_as_discovery() {
        let entries = parse_servers(FIXTURE).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.name, "@acme/weather");
        assert_eq!(e.display_name, "Weather");
        assert_eq!(e.source, "smithery");
        assert_eq!(e.install_shape, "discovery");
        assert_eq!(e.repo_url.as_deref(), Some("https://smithery.ai/server/@acme/weather"));
    }
}
