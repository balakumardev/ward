//! Glama MCP directory source (https://glama.ai/api/mcp/v1/servers). No key
//! required. Glama carries rich metadata but no command/transport, so every
//! entry is `install_shape: "discovery"` with a repo URL for the "View" link.

use std::time::Duration;

use serde_json::Value;

use super::{classify_install_shape, MarketEntry};
use crate::error::WardError;

const GLAMA_URL: &str = "https://glama.ai/api/mcp/v1/servers";
const TIMEOUT_SECS: u64 = 10;
const PAGE_LIMIT: &str = "50";

/// Parse a Glama `/servers` response body into discovery `MarketEntry`s.
pub fn parse_servers(body: &str) -> Result<Vec<MarketEntry>, WardError> {
    let root: Value = serde_json::from_str(body)
        .map_err(|e| WardError::Registry(format!("parse glama response: {e}")))?;
    let mut out = Vec::new();
    if let Some(servers) = root.get("servers").and_then(|v| v.as_array()) {
        for s in servers {
            let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if name.is_empty() {
                continue;
            }
            let description = s.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let repo_url = s
                .get("repository")
                .and_then(|r| r.get("url"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string());
            out.push(MarketEntry {
                kind: "mcp".into(),
                name: name.clone(),
                display_name: name,
                description,
                source: "glama".into(),
                version: None,
                verified: false, // Glama is a community directory, not the signed registry
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

/// Server-side search over Glama (query filters by name/description).
pub fn search(query: &str) -> Result<Vec<MarketEntry>, WardError> {
    let mut req = ureq::get(GLAMA_URL)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .query("first", PAGE_LIMIT);
    let q = query.trim();
    if !q.is_empty() {
        req = req.query("query", q);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => {
            return Err(WardError::Registry(format!("Glama returned HTTP {code}")));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(WardError::Registry(format!("network error reaching Glama: {t}")));
        }
    };
    let body = resp
        .into_string()
        .map_err(|e| WardError::Registry(format!("read glama response body: {e}")))?;
    parse_servers(&body)
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = include_str!("fixtures/glama-servers.json");

    #[test]
    fn parses_glama_entries_as_discovery_with_repo_url() {
        let entries = parse_servers(FIXTURE).unwrap();
        assert_eq!(entries.len(), 1); // the empty-name row is skipped
        let e = &entries[0];
        assert_eq!(e.name, "acme-notes");
        assert_eq!(e.source, "glama");
        assert_eq!(e.install_shape, "discovery");
        assert_eq!(e.repo_url.as_deref(), Some("https://github.com/acme/notes-mcp"));
        assert!(e.packages.is_empty() && e.remotes.is_empty());
    }
}
