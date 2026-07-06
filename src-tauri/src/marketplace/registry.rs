//! Plan 21 Task 2 — official MCP Registry client.
//!
//! Split into a pure `parse_servers` (fully unit-tested against a pinned
//! synthetic fixture) and a thin `fetch_servers` `ureq` wrapper (network,
//! not unit-tested), mirroring `usage/live.rs`.
//!
//! Real registry shape (`GET https://registry.modelcontextprotocol.io/v0/servers`):
//! ```json
//! { "servers": [ { "server": { <server.json> }, "_meta": {…} } ],
//!   "metadata": { "nextCursor": "…", "count": N } }
//! ```
//! Each item nests the `server.json` under `.server` with sibling `._meta`.
//! (The plan's brief described the shape as a flat `servers: [server.json]`;
//! the live API nests it — `parse_servers` handles the real nesting AND
//! tolerates a flat entry.) A `server.json` carries `name`, `description`,
//! optional `title`/`version`, and either `packages[]` (npm/pypi/oci, each
//! with `transport{type}`, `environmentVariables[]{name,isRequired,isSecret}`,
//! optional `runtimeHint`) or `remotes[]{type, url, headers[]}`.

use std::time::Duration;

use serde_json::Value;

use super::{EnvVar, MarketEntry, MarketPage, Package, Remote};
use crate::error::WardError;

const REGISTRY_URL: &str = "https://registry.modelcontextprotocol.io/v0/servers";
const PAGE_LIMIT: &str = "100";
const TIMEOUT_SECS: u64 = 10;

// ── Pure parse ───────────────────────────────────────────────────────────

/// Parse the official registry JSON body into a [`MarketPage`]. Pure and
/// fully unit-tested — the network glue just supplies the body. Tolerates
/// missing `packages`/`remotes`/`metadata`, an absent `title`, and a flat
/// (un-nested) `server.json` entry. Servers without a `name` are skipped.
pub fn parse_servers(body: &str) -> Result<MarketPage, WardError> {
    let root: Value = serde_json::from_str(body)
        .map_err(|e| WardError::Registry(format!("parse registry response: {e}")))?;

    let mut entries = Vec::new();
    if let Some(servers) = root.get("servers").and_then(|v| v.as_array()) {
        for item in servers {
            // Real shape nests the server.json under `.server` (with sibling
            // `._meta`); tolerate a flat server.json entry too.
            let server = item.get("server").unwrap_or(item);
            if let Some(entry) = parse_one(server) {
                entries.push(entry);
            }
        }
    }

    // The registry lists every published VERSION of a server as its own row
    // (e.g. `anki-mcp-server` appears a dozen times, once per version). For the
    // marketplace UI that's noise — the cards key by `name`, so all versions of
    // one server render as duplicate rows that select together. Collapse to one
    // row per name, keeping the highest semver (the version the user wants to
    // install). Order of first appearance is otherwise preserved.
    let entries = dedupe_by_name(entries);

    let next_cursor = root
        .get("metadata")
        .and_then(|m| m.get("nextCursor"))
        .and_then(|c| c.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    Ok(MarketPage { entries, next_cursor })
}

/// Collapse multiple version rows of the same server name into one, keeping
/// the highest version. Preserves first-appearance order of the surviving
/// entries so the list stays stable. When versions are equal/unparseable the
/// first-seen entry wins.
fn dedupe_by_name(entries: Vec<MarketEntry>) -> Vec<MarketEntry> {
    // index in `order` → surviving entry; `pos` maps name → that index.
    let mut order: Vec<MarketEntry> = Vec::with_capacity(entries.len());
    let mut pos: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for e in entries {
        if let Some(&i) = pos.get(&e.name) {
            // Keep whichever version is higher.
            if version_gt(e.version.as_deref(), order[i].version.as_deref()) {
                order[i] = e;
            }
        } else {
            pos.insert(e.name.clone(), order.len());
            order.push(e);
        }
    }
    order
}

/// True when version `a` is strictly greater than `b` under a lenient semver
/// compare (dot-separated numeric components, missing components treated as 0;
/// non-numeric suffixes ignored). `None`/unparseable sorts lowest.
fn version_gt(a: Option<&str>, b: Option<&str>) -> bool {
    version_key(a) > version_key(b)
}

/// Parse a version string into comparable numeric components. Strips a leading
/// `v` and any pre-release/build suffix on each component (e.g. `1.2.0-rc1` →
/// `[1,2,0]`). Returns an empty vec (sorts lowest) for `None`/empty.
fn version_key(v: Option<&str>) -> Vec<u64> {
    let Some(v) = v else { return Vec::new() };
    let v = v.trim().trim_start_matches(['v', 'V']);
    if v.is_empty() { return Vec::new(); }
    v.split('.')
        .map(|part| {
            // Take the leading run of digits (so `0-rc1` → 0, `2beta` → 2).
            let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<u64>().unwrap_or(0)
        })
        .collect()
}

/// Map one `server.json` object into a [`MarketEntry`] (`kind: "mcp"`).
/// Returns `None` when there is no usable `name`.
fn parse_one(server: &Value) -> Option<MarketEntry> {
    let name = server.get("name").and_then(|v| v.as_str())?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let description = server
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Prefer the human `title`; fall back to the registry id.
    let display_name = server
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.clone());
    let version = server
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let packages = server
        .get("packages")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_package).collect())
        .unwrap_or_default();
    let remotes = server
        .get("remotes")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_remote).collect())
        .unwrap_or_default();

    Some(MarketEntry {
        kind: "mcp".into(),
        name,
        display_name,
        description,
        source: "registry".into(),
        version,
        verified: true, // every registry-listed server is registry-verified
        packages,
        remotes,
        repo_url: None,
        skill_path: None,
    })
}

/// One `environmentVariables[]` / `headers[]` entry → [`EnvVar`]. Missing
/// `isRequired`/`isSecret` default to `false` (never accidentally secret).
fn parse_env_var(v: &Value) -> Option<EnvVar> {
    let name = v.get("name").and_then(|n| n.as_str())?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(EnvVar {
        name,
        is_required: v.get("isRequired").and_then(|b| b.as_bool()).unwrap_or(false),
        is_secret: v.get("isSecret").and_then(|b| b.as_bool()).unwrap_or(false),
    })
}

/// One `packages[]` entry → [`Package`]. `transport` is an object
/// `{type: "stdio"|…}`; tolerate a bare string. Skips packages with no
/// `registryType` or `identifier`.
fn parse_package(v: &Value) -> Option<Package> {
    let registry_type = v.get("registryType").and_then(|s| s.as_str())?.trim().to_string();
    let identifier = v.get("identifier").and_then(|s| s.as_str())?.trim().to_string();
    if registry_type.is_empty() || identifier.is_empty() {
        return None;
    }
    let version = v.get("version").and_then(|s| s.as_str()).unwrap_or("").trim().to_string();
    let transport = v
        .get("transport")
        .and_then(|t| t.get("type").and_then(|s| s.as_str()).or_else(|| t.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("stdio")
        .to_string();
    let env = v
        .get("environmentVariables")
        .and_then(|e| e.as_array())
        .map(|arr| arr.iter().filter_map(parse_env_var).collect())
        .unwrap_or_default();
    let runtime_hint = v
        .get("runtimeHint")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Some(Package { registry_type, identifier, version, transport, env, runtime_hint })
}

/// One `remotes[]` entry → [`Remote`]. The registry names the transport
/// field `type` (e.g. `"streamable-http"`, `"sse"`). Skips remotes with no
/// `url`.
fn parse_remote(v: &Value) -> Option<Remote> {
    let url = v.get("url").and_then(|s| s.as_str())?.trim().to_string();
    if url.is_empty() {
        return None;
    }
    let transport = v
        .get("type")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("http")
        .to_string();
    let headers = v
        .get("headers")
        .and_then(|h| h.as_array())
        .map(|arr| arr.iter().filter_map(parse_env_var).collect())
        .unwrap_or_default();
    Some(Remote { transport, url, headers })
}

// ── Network (thin wrapper, not unit-tested) ──────────────────────────────

/// Fetch one page of registry servers. User-triggered only (search on
/// demand) — never a background poll. 10 s timeout; transport/HTTP errors
/// map to [`WardError::Registry`]. Delegates parsing to [`parse_servers`].
pub fn fetch_servers(cursor: Option<&str>) -> Result<MarketPage, WardError> {
    let mut req = ureq::get(REGISTRY_URL)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .query("limit", PAGE_LIMIT);
    if let Some(c) = cursor {
        if !c.is_empty() {
            req = req.query("cursor", c);
        }
    }

    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => {
            return Err(WardError::Registry(format!("the MCP registry returned HTTP {code}")));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(WardError::Registry(format!("network error reaching the MCP registry: {t}")));
        }
    };

    let body = resp
        .into_string()
        .map_err(|e| WardError::Registry(format!("read registry response body: {e}")))?;
    parse_servers(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("fixtures/registry-servers.json");

    #[test]
    fn parses_three_entries_and_cursor() {
        let page = parse_servers(FIXTURE).unwrap();
        assert_eq!(page.entries.len(), 3);
        assert_eq!(page.next_cursor.as_deref(), Some("com.acme/hosted:3.0.0"));
        // Every registry-listed entry is verified + kind "mcp" + source "registry".
        for e in &page.entries {
            assert!(e.verified);
            assert_eq!(e.kind, "mcp");
            assert_eq!(e.source, "registry");
        }
    }

    #[test]
    fn npm_stdio_package_with_env_flags() {
        let page = parse_servers(FIXTURE).unwrap();
        let e = page.entries.iter().find(|e| e.name == "io.github.acme/notes").unwrap();
        assert_eq!(e.display_name, "Acme Notes"); // from `title`
        assert_eq!(e.version.as_deref(), Some("2.1.0"));
        assert_eq!(e.packages.len(), 1);
        let p = &e.packages[0];
        assert_eq!(p.registry_type, "npm");
        assert_eq!(p.identifier, "@acme/notes-mcp");
        assert_eq!(p.version, "2.1.0");
        assert_eq!(p.transport, "stdio"); // extracted from transport{type}
        assert_eq!(p.env.len(), 2);
        let secret = p.env.iter().find(|v| v.name == "NOTES_API_KEY").unwrap();
        assert!(secret.is_required && secret.is_secret);
        let plain = p.env.iter().find(|v| v.name == "NOTES_REGION").unwrap();
        assert!(!plain.is_required && !plain.is_secret);
    }

    #[test]
    fn pypi_stdio_package_with_runtime_hint() {
        let page = parse_servers(FIXTURE).unwrap();
        let e = page.entries.iter().find(|e| e.name == "io.github.acme/pytools").unwrap();
        // No `title` → display_name falls back to the registry id.
        assert_eq!(e.display_name, "io.github.acme/pytools");
        assert_eq!(e.packages.len(), 1);
        let p = &e.packages[0];
        assert_eq!(p.registry_type, "pypi");
        assert_eq!(p.identifier, "acme-pytools");
        assert_eq!(p.version, "0.4.2");
        assert_eq!(p.transport, "stdio");
        assert_eq!(p.runtime_hint.as_deref(), Some("uvx"));
        assert!(p.env.is_empty());
        assert!(e.remotes.is_empty());
    }

    #[test]
    fn remote_http_with_secret_header() {
        let page = parse_servers(FIXTURE).unwrap();
        let e = page.entries.iter().find(|e| e.name == "com.acme/hosted").unwrap();
        assert!(e.packages.is_empty());
        assert_eq!(e.remotes.len(), 1);
        let r = &e.remotes[0];
        assert_eq!(r.transport, "streamable-http");
        assert_eq!(r.url, "https://mcp.acme.example/v1");
        assert_eq!(r.headers.len(), 1);
        assert_eq!(r.headers[0].name, "X-Acme-Token");
        assert!(r.headers[0].is_secret && r.headers[0].is_required);
    }

    #[test]
    fn tolerates_missing_metadata_packages_and_remotes() {
        // A bare server with neither packages nor remotes, no metadata block.
        let body = r#"{"servers":[{"server":{"name":"x/minimal","description":"d"}}]}"#;
        let page = parse_servers(body).unwrap();
        assert_eq!(page.entries.len(), 1);
        let e = &page.entries[0];
        assert_eq!(e.name, "x/minimal");
        assert!(e.packages.is_empty());
        assert!(e.remotes.is_empty());
        assert!(e.version.is_none());
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn tolerates_flat_unnested_server_entry() {
        // Some callers/tests may present a flat server.json (no `.server`).
        let body = r#"{"servers":[{"name":"x/flat","description":"d","version":"1.0.0",
            "packages":[{"registryType":"npm","identifier":"flat","version":"1.0.0","transport":{"type":"stdio"}}]}]}"#;
        let page = parse_servers(body).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].name, "x/flat");
        assert_eq!(page.entries[0].packages[0].identifier, "flat");
    }

    #[test]
    fn skips_nameless_servers_and_empty_body() {
        let body = r#"{"servers":[{"server":{"description":"no name here"}},{"server":{"name":"ok/one"}}]}"#;
        let page = parse_servers(body).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].name, "ok/one");

        // No `servers` key at all → empty page, not an error.
        let empty = parse_servers("{}").unwrap();
        assert!(empty.entries.is_empty());
        assert!(empty.next_cursor.is_none());
    }

    #[test]
    fn malformed_json_is_a_registry_error() {
        let err = parse_servers("not json at all").unwrap_err();
        assert!(matches!(err, WardError::Registry(_)));
    }

    #[test]
    fn dedupes_multiple_versions_keeping_highest() {
        // Same server name published at three versions — the registry lists
        // each as its own row. parse_servers must collapse to one, keeping the
        // highest semver.
        let body = r#"{"servers":[
            {"server":{"name":"x/note","version":"1.0.0","description":"v1"}},
            {"server":{"name":"x/note","version":"1.2.0","description":"v1.2"}},
            {"server":{"name":"x/note","version":"1.0.1","description":"v1.0.1"}},
            {"server":{"name":"y/other","version":"3.0.0","description":"o"}}
        ]}"#;
        let page = parse_servers(body).unwrap();
        assert_eq!(page.entries.len(), 2, "one row per unique name");
        let note = page.entries.iter().find(|e| e.name == "x/note").unwrap();
        assert_eq!(note.version.as_deref(), Some("1.2.0"), "highest version wins");
        assert_eq!(note.description, "v1.2");
    }

    #[test]
    fn version_key_orders_semver_numerically() {
        // 1.10.0 > 1.9.0 (numeric, not lexical); v-prefix + pre-release tolerated.
        assert!(version_gt(Some("1.10.0"), Some("1.9.0")));
        assert!(version_gt(Some("v2.0.0"), Some("1.99.99")));
        assert!(version_gt(Some("1.0.1"), Some("1.0.0-rc1")));
        assert!(!version_gt(Some("1.0.0"), Some("1.0.0")));
        assert!(version_gt(Some("0.1.0"), None));
    }
}
