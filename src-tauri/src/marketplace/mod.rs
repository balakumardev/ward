//! Plan 21 — Marketplace (MCP servers).
//!
//! A new 6th sidebar mode that **searches** the official MCP Registry and
//! **installs** MCP servers into the chosen harness(es) × scope(s) via the
//! SAME `mcp_upsert_entry` engine (install-once-to-many). Skills are Plan 22;
//! this module handles MCP servers only, but the data models leave a clean
//! seam for skills (`kind: "skill"`, `repo_url`, `skill_path`).
//!
//! Security posture (spec §9.5, enforced downstream):
//!   * Network is **user-triggered only** — search on demand, install on click,
//!     never a background poll. Every fetcher splits into a thin `ureq` wrapper
//!     (not unit-tested) + a pure parse fn (fully unit-tested against a pinned
//!     synthetic fixture), mirroring `usage/live.rs`.
//!   * Version-pinning is enforced in `install::build_mcp_config` (never
//!     `@latest`, never an empty version → `WardError::Registry`).
//!   * Ward never collects secret values: `EnvVar { is_secret: true }` is
//!     written as an empty string (never a typed-in token).

pub mod install;
pub mod registry;

use serde::{Deserialize, Serialize};

/// Unified card model over the wire (camelCase). One entry per registry
/// server (`kind: "mcp"`) or skill (`kind: "skill"`, filled by Plan 22).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarketEntry {
    /// `"mcp"` | `"skill"`.
    pub kind: String,
    /// Registry id, e.g. `"io.github.owner/server"`.
    pub name: String,
    pub display_name: String,
    pub description: String,
    /// `"registry"` | `"github"` | `"marketplace"`.
    pub source: String,
    /// Concrete version if known (never `"latest"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Registry-listed / signed status.
    pub verified: bool,
    /// MCP packages (stdio/http/sse). Empty for pure-remote or skill entries.
    #[serde(default)]
    pub packages: Vec<Package>,
    /// MCP remotes (hosted http/sse). Empty for package-only or skill entries.
    #[serde(default)]
    pub remotes: Vec<Remote>,
    /// Skills only — source repo (Plan 22).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_url: Option<String>,
    /// Skills only — `SKILL.md` path within the repo (Plan 22).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_path: Option<String>,
}

/// One installable package for an MCP server (npm/pypi/oci).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    /// `"npm"` | `"pypi"` | `"oci"`.
    pub registry_type: String,
    pub identifier: String,
    pub version: String,
    /// `"stdio"` | `"http"` | `"sse"`.
    pub transport: String,
    #[serde(default)]
    pub env: Vec<EnvVar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_hint: Option<String>,
}

/// One environment variable (or remote header) the server declares. Ward
/// renders the NAME only; secret values are never collected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnvVar {
    pub name: String,
    pub is_required: bool,
    pub is_secret: bool,
}

/// A hosted remote transport (http/sse) for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    /// `"http"` | `"sse"` | `"streamable-http"` (whatever the registry lists).
    pub transport: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<EnvVar>,
}

/// One install destination — a harness × scope pair. Kept as a data
/// structure (not a hard-coded pair) so `harness: "claude-desktop"` slots
/// in later without a rewrite (spec §12).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstallTarget {
    pub harness: String,
    pub scope_id: String,
}

/// The exact server object that will land on disk, plus a flattened preview
/// and the env-var metadata (so the UI can show which vars are secret).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BuiltConfig {
    pub name: String,
    /// The exact server object (stdio `{command,args,env}` or remote
    /// `{url,type,headers}`) — byte-for-byte what gets upserted.
    pub config: serde_json::Value,
    /// Flattened `[command, ...args]` for stdio, or `[url]` for a remote —
    /// rendered verbatim in the pre-install preview.
    pub command_preview: Vec<String>,
    /// The declared env/header vars, so the install sheet can mark secrets
    /// read-only and let non-secret vars be filled.
    #[serde(default)]
    pub env: Vec<EnvVar>,
}

/// The outcome of installing into one target. The batch never aborts on a
/// single failure, so each target reports independently (and each success
/// carries its own undoable `RestoreInfo`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub target: InstallTarget,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore: Option<crate::model::RestoreInfo>,
}

/// One page of registry results plus the cursor for the next page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarketPage {
    #[serde(default)]
    pub entries: Vec<MarketEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_entry_serializes_camel_case() {
        let e = MarketEntry {
            kind: "mcp".into(),
            name: "io.github.owner/server".into(),
            display_name: "Server".into(),
            description: "does things".into(),
            source: "registry".into(),
            version: Some("1.2.3".into()),
            verified: true,
            packages: vec![Package {
                registry_type: "npm".into(),
                identifier: "@owner/server".into(),
                version: "1.2.3".into(),
                transport: "stdio".into(),
                env: vec![EnvVar { name: "API_KEY".into(), is_required: true, is_secret: true }],
                runtime_hint: None,
            }],
            remotes: vec![],
            repo_url: None,
            skill_path: None,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"displayName\":\"Server\""));
        assert!(json.contains("\"registryType\":\"npm\""));
        assert!(json.contains("\"isSecret\":true"));
        assert!(json.contains("\"isRequired\":true"));
        // Skill-only fields omitted when None.
        assert!(!json.contains("repoUrl"));
        assert!(!json.contains("skillPath"));
        // Round-trips.
        let back: MarketEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn market_page_defaults_tolerate_missing_fields() {
        // A bare object with neither entries nor nextCursor must parse.
        let page: MarketPage = serde_json::from_str("{}").unwrap();
        assert!(page.entries.is_empty());
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn install_result_omits_none_error_and_restore() {
        let r = InstallResult {
            target: InstallTarget { harness: "claude".into(), scope_id: "global".into() },
            ok: true,
            error: None,
            restore: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"scopeId\":\"global\""));
        assert!(json.contains("\"ok\":true"));
        assert!(!json.contains("error"));
        assert!(!json.contains("restore"));
    }
}
