//! Plan 28 — Plugins mode wire models.
//!
//! The wire shapes Ward exposes to the frontend for the Plugins mode.
//! These are Ward's own normalised view — constructed in Rust from three
//! on-disk Claude Code files and serialized out over `invoke`. They are
//! NOT 1:1 with any single on-disk file.
//!
//! Real on-disk shapes these are distilled from (verified on this machine,
//! CC v2.1.206):
//!   - `installed_plugins.json` =
//!     `{version, plugins:{"<name>@<mkt>":[{scope,projectPath?,installPath,
//!     version,installedAt,lastUpdated}]}}`
//!   - `known_marketplaces.json` keyed by marketplace name →
//!     `{source:{source,repo},installLocation,lastUpdated}`
//!   - `plugin-catalog-cache.json` =
//!     `{version,fetchedAt,catalog:{generated_at,models:[…],
//!     plugins:{"<name>@<mkt>":{plugin,tokens:{"<model>":{always_on,
//!     on_invoke}},components:{commands[],agents[],skills[],hooks[],
//!     mcpServers[],lspServers[]},unique_installs,last_updated,
//!     marketplace_entry,version,source,…}}}}` (cache only covers
//!     `claude-plugins-official`).

use serde::{Deserialize, Serialize};

pub mod catalog;

/// A single plugin as Ward presents it — merges an installed record
/// (`installed_plugins.json`) with catalog metadata
/// (`plugin-catalog-cache.json`) when available. `installed` /
/// `enabled` reflect on-disk state; the token/component/`unique_installs`
/// fields are catalog-only and stay `None` for plugins Ward has no
/// catalog entry for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginEntry {
    /// Discriminator for the frontend — always `"plugin"`.
    pub kind: String,
    /// Bare plugin name (the part before `@` in the catalog key).
    pub name: String,
    /// Marketplace this plugin belongs to (the part after `@`).
    pub marketplace: String,
    /// Human-facing name from the catalog, falling back to `name`.
    pub display_name: String,
    /// Short description from the catalog.
    pub description: String,
    /// Version string (installed version, else catalog version). `None`
    /// when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Source blob — mirrors the marketplace `source` object
    /// (`{source, repo}` for github). Kept opaque so new source kinds
    /// pass through untouched.
    pub source: serde_json::Value,
    /// Plugin author, catalog-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Catalog category, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Catalog tags; empty when none.
    pub tags: Vec<String>,
    /// Whether this plugin is present in `installed_plugins.json`.
    pub installed: bool,
    /// Whether the installed plugin is enabled (vs. installed-but-off).
    pub enabled: bool,
    /// Install scope (`"user"` / `"project"` / …) from the installed
    /// record; `None` for catalog-only (not installed) entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Catalog install count, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique_installs: Option<u64>,
    /// Tokens this plugin always loads into context (catalog `always_on`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub always_on_tokens: Option<u64>,
    /// Tokens loaded only when the plugin is invoked (catalog `on_invoke`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_invoke_tokens: Option<u64>,
    /// Per-kind component tallies from the catalog; `None` when uncatalogued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_counts: Option<ComponentCounts>,
}

/// Count of each component kind a plugin ships, from the catalog's
/// `components` object (each field is the length of the corresponding
/// array).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComponentCounts {
    pub commands: u64,
    pub agents: u64,
    pub skills: u64,
    pub hooks: u64,
    pub mcp_servers: u64,
    pub lsp_servers: u64,
}

/// A marketplace Claude Code knows about, from `known_marketplaces.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceRef {
    /// Marketplace name (the map key in `known_marketplaces.json`).
    pub name: String,
    /// Source blob for the marketplace (`{source, repo}`), kept opaque.
    pub source: serde_json::Value,
    /// Local install location of the marketplace clone, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_location: Option<String>,
    /// Last-updated timestamp from the marketplace record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
}

/// Full result of a plugins scan — the marketplaces Claude Code knows,
/// the merged plugin list, and whether the `claude` CLI is available
/// (gates enable/disable/install actions in the frontend).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginScan {
    pub marketplaces: Vec<MarketplaceRef>,
    pub plugins: Vec<PluginEntry>,
    pub cli_available: bool,
}

/// Canonical `<name>@<marketplace>` key — matches the keys Claude Code
/// uses in `installed_plugins.json` and `plugin-catalog-cache.json`.
pub fn plugin_key(name: &str, marketplace: &str) -> String {
    format!("{name}@{marketplace}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plugin_entry_serializes_camel_case_and_key_joins() {
        let e = PluginEntry {
            kind: "plugin".into(), name: "code-formatter".into(),
            marketplace: "claude-plugins-official".into(),
            display_name: "Code Formatter".into(), description: "fmt".into(),
            version: Some("2.1.0".into()), source: serde_json::json!({"source":"github","repo":"a/b"}),
            author: Some("Anthropic".into()), category: Some("dev".into()),
            tags: vec!["fmt".into()], installed: true, enabled: false,
            scope: Some("user".into()), unique_installs: Some(1234),
            always_on_tokens: Some(1005), on_invoke_tokens: Some(15353),
            component_counts: Some(ComponentCounts { commands: 0, agents: 0, skills: 5, hooks: 0, mcp_servers: 1, lsp_servers: 0 }),
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("\"displayName\":\"Code Formatter\""));
        assert!(j.contains("\"uniqueInstalls\":1234"));
        assert!(j.contains("\"alwaysOnTokens\":1005"));
        assert!(j.contains("\"mcpServers\":1"));
        assert_eq!(plugin_key("code-formatter", "claude-plugins-official"),
                   "code-formatter@claude-plugins-official");
        let back: PluginEntry = serde_json::from_str(&j).unwrap();
        assert_eq!(e, back);
    }
}
