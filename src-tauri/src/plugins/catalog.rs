//! Plan 28 — on-disk catalog readers for Claude Code's `~/.claude/plugins/**`.
//!
//! Turns the four JSON files Claude Code writes under `~/.claude/plugins/`
//! into the [`PluginScan`] wire model. Every parser is pure (`&Value` in,
//! owned value out) so it can be unit-tested against the synthetic fixtures
//! in `plugins/fixtures/`; [`scan_plugins`] is the thin `home: &Path` reader
//! that loads each file and merges the pieces.
//!
//! Tolerance mirrors `harness::adapters::claude::scan_plugins` — a missing
//! file, unparseable JSON, or an unexpected shape yields an empty result
//! rather than a panic, so a half-written cache never breaks the scan.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::error::WardError;
use crate::plugins::{plugin_key, ComponentCounts, MarketplaceRef, PluginEntry, PluginScan};

/// Collapsed install record for one plugin key, distilled from the (possibly
/// multiple) entries under `installed_plugins.json → plugins[<key>]`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InstalledInfo {
    /// Installed version string (`None` when absent/empty).
    pub version: Option<String>,
    /// Install scope (`"user"` / `"project"` / `"local"`; `None` when absent).
    pub scope: Option<String>,
    /// Absolute cache install path — always non-empty (records with an empty
    /// `installPath` are dropped, matching the reference scanner).
    pub install_path: String,
}

/// Catalog-only metadata for one plugin key, distilled from
/// `plugin-catalog-cache.json`. Every field is optional because the cache
/// only covers plugins Claude Code has fetched metadata for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CatalogMeta {
    pub unique_installs: Option<u64>,
    pub always_on_tokens: Option<u64>,
    pub on_invoke_tokens: Option<u64>,
    pub component_counts: Option<ComponentCounts>,
}

// ---------------------------------------------------------------------------
// Pure parsers.
// ---------------------------------------------------------------------------

/// Parse `installed_plugins.json`. A plugin key may hold multiple install
/// records (different scopes / versions) — collapse each to the newest by
/// `lastUpdated` (falling back to `installedAt`), dropping any record with an
/// empty `installPath`. Mirrors the collapse logic in the reference scanner
/// (`harness::adapters::claude::scan_plugins`).
pub(crate) fn parse_installed(v: &Value) -> HashMap<String, InstalledInfo> {
    let mut map = HashMap::new();
    let Some(plugins) = v.get("plugins").and_then(|p| p.as_object()) else {
        return map;
    };
    for (key, installs) in plugins {
        let Some(arr) = installs.as_array() else { continue };
        let newest = arr.iter().max_by_key(|i| {
            i.get("lastUpdated")
                .and_then(|x| x.as_str())
                .or_else(|| i.get("installedAt").and_then(|x| x.as_str()))
                .unwrap_or("")
                .to_string()
        });
        let Some(install) = newest else { continue };
        let install_path = install
            .get("installPath")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if install_path.is_empty() {
            continue;
        }
        map.insert(
            key.clone(),
            InstalledInfo {
                version: nonempty_str(install.get("version")),
                scope: nonempty_str(install.get("scope")),
                install_path: install_path.to_string(),
            },
        );
    }
    map
}

/// Parse `known_marketplaces.json` — the top-level object is keyed by
/// marketplace name. Sorted by name for a stable scan order.
pub(crate) fn parse_known_marketplaces(v: &Value) -> Vec<MarketplaceRef> {
    let Some(obj) = v.as_object() else {
        return Vec::new();
    };
    let mut out: Vec<MarketplaceRef> = obj
        .iter()
        .map(|(name, rec)| MarketplaceRef {
            name: name.clone(),
            source: rec.get("source").cloned().unwrap_or(Value::Null),
            install_location: nonempty_str(rec.get("installLocation")),
            last_updated: nonempty_str(rec.get("lastUpdated")),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Parse a cloned marketplace's `.claude-plugin/marketplace.json`. Produces
/// one [`PluginEntry`] per listed plugin with `installed`/`enabled` left
/// false — the merge step in [`scan_plugins`] fills on-disk state.
///
/// Tolerant of the real official-catalog shape: `author` may be a bare
/// string or an object `{name}`, and `displayName`/`version`/`tags` are
/// commonly absent (`display_name` falls back to `name`, `version` stays
/// `None`, `tags` falls back to `keywords` then empty).
pub(crate) fn parse_marketplace_manifest(v: &Value, marketplace_name: &str) -> Vec<PluginEntry> {
    let Some(plugins) = v.get("plugins").and_then(|p| p.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for p in plugins {
        let Some(name) = p.get("name").and_then(|x| x.as_str()) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let display_name = nonempty_str(p.get("displayName")).unwrap_or_else(|| name.to_string());
        out.push(PluginEntry {
            kind: "plugin".into(),
            name: name.to_string(),
            marketplace: marketplace_name.to_string(),
            display_name,
            description: p
                .get("description")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            version: nonempty_str(p.get("version")),
            source: p.get("source").cloned().unwrap_or(Value::Null),
            author: parse_author(p.get("author")),
            category: nonempty_str(p.get("category")),
            tags: manifest_tags(p),
            installed: false,
            enabled: false,
            scope: None,
            unique_installs: None,
            always_on_tokens: None,
            on_invoke_tokens: None,
            component_counts: None,
        });
    }
    out
}

/// Parse `plugin-catalog-cache.json` into per-key catalog metadata. Token
/// fields come from the first model listed in `catalog.models`; component
/// counts are the lengths of each `components.<kind>` array.
pub(crate) fn parse_catalog_cache(v: &Value) -> HashMap<String, CatalogMeta> {
    let mut map = HashMap::new();
    let Some(catalog) = v.get("catalog").and_then(|c| c.as_object()) else {
        return map;
    };
    let first_model = catalog
        .get("models")
        .and_then(|m| m.as_array())
        .and_then(|a| a.first())
        .and_then(|m| m.as_str());
    let Some(plugins) = catalog.get("plugins").and_then(|p| p.as_object()) else {
        return map;
    };
    for (key, entry) in plugins {
        let tokens = first_model.and_then(|m| entry.get("tokens").and_then(|t| t.get(m)));
        let component_counts = entry.get("components").and_then(|c| c.as_object()).map(|c| {
            let len = |k: &str| {
                c.get(k)
                    .and_then(|a| a.as_array())
                    .map(|a| a.len() as u64)
                    .unwrap_or(0)
            };
            ComponentCounts {
                commands: len("commands"),
                agents: len("agents"),
                skills: len("skills"),
                hooks: len("hooks"),
                mcp_servers: len("mcpServers"),
                lsp_servers: len("lspServers"),
            }
        });
        map.insert(
            key.clone(),
            CatalogMeta {
                unique_installs: entry.get("unique_installs").and_then(|x| x.as_u64()),
                always_on_tokens: tokens.and_then(|t| t.get("always_on")).and_then(|x| x.as_u64()),
                on_invoke_tokens: tokens.and_then(|t| t.get("on_invoke")).and_then(|x| x.as_u64()),
                component_counts,
            },
        );
    }
    map
}

// ---------------------------------------------------------------------------
// Reader + merge.
// ---------------------------------------------------------------------------

/// Read + parse all four `~/.claude/plugins/**` files and merge them into a
/// [`PluginScan`]. `cli_available` is threaded in by the command layer (the
/// `plugins::cli` availability probe lands in a later task).
///
/// Merge order: every plugin a known marketplace's manifest lists comes
/// first (installed/enabled/version/scope from `installed_plugins.json` +
/// `enabledPlugins`; unique-installs/token/component fields from the catalog
/// cache), then any *installed* plugin whose marketplace manifest is absent —
/// so nothing installed is ever hidden.
pub fn scan_plugins(home: &Path, cli_available: bool) -> PluginScan {
    let plugins_dir = home.join(".claude").join("plugins");

    let installed = read_json(&plugins_dir.join("installed_plugins.json"))
        .map(|v| parse_installed(&v))
        .unwrap_or_default();
    let marketplaces = read_json(&plugins_dir.join("known_marketplaces.json"))
        .map(|v| parse_known_marketplaces(&v))
        .unwrap_or_default();
    let catalog = read_json(&plugins_dir.join("plugin-catalog-cache.json"))
        .map(|v| parse_catalog_cache(&v))
        .unwrap_or_default();
    let enabled_map = crate::harness::adapters::claude::plugin_enabled_map(home);

    let mut entries: Vec<PluginEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // 1) Plugins listed by each known marketplace's cloned manifest.
    for mkt in &marketplaces {
        let manifest_path = plugins_dir
            .join("marketplaces")
            .join(&mkt.name)
            .join(".claude-plugin")
            .join("marketplace.json");
        let Some(v) = read_json(&manifest_path) else { continue };
        for mut entry in parse_marketplace_manifest(&v, &mkt.name) {
            let key = plugin_key(&entry.name, &entry.marketplace);
            apply_installed(&mut entry, &key, &installed, &enabled_map);
            apply_catalog(&mut entry, &key, &catalog);
            seen.insert(key);
            entries.push(entry);
        }
    }
    entries.sort_by(|a, b| {
        plugin_key(&a.name, &a.marketplace).cmp(&plugin_key(&b.name, &b.marketplace))
    });

    // 2) Installed plugins no manifest covered — appended last, sorted.
    let mut orphans: Vec<PluginEntry> = Vec::new();
    for key in installed.keys() {
        if seen.contains(key) {
            continue;
        }
        let (name, marketplace) = match key.rsplit_once('@') {
            Some((n, m)) => (n.to_string(), m.to_string()),
            None => (key.clone(), String::new()),
        };
        let mut entry = PluginEntry {
            kind: "plugin".into(),
            name: name.clone(),
            marketplace,
            display_name: name,
            description: String::new(),
            version: None,
            source: Value::Null,
            author: None,
            category: None,
            tags: Vec::new(),
            installed: false,
            enabled: false,
            scope: None,
            unique_installs: None,
            always_on_tokens: None,
            on_invoke_tokens: None,
            component_counts: None,
        };
        apply_installed(&mut entry, key, &installed, &enabled_map);
        apply_catalog(&mut entry, key, &catalog);
        orphans.push(entry);
    }
    orphans.sort_by(|a, b| {
        plugin_key(&a.name, &a.marketplace).cmp(&plugin_key(&b.name, &b.marketplace))
    });
    entries.extend(orphans);

    PluginScan { marketplaces, plugins: entries, cli_available }
}

// ---------------------------------------------------------------------------
// Remote fetch (thin `ureq` wrapper, not unit-tested).
// ---------------------------------------------------------------------------

/// Fetch a raw `marketplace.json` from `url` (e.g. a GitHub raw link) and
/// parse it into [`PluginEntry`]s via the pure [`parse_marketplace_manifest`].
/// User-triggered only (add-a-marketplace-by-URL) — never a background poll.
///
/// 10 s timeout; any network / HTTP / body-read / JSON-parse failure maps to
/// [`WardError::Plugin`]. The marketplace name is taken from the manifest's
/// top-level `name` field, falling back to `"remote"` when absent or empty.
///
/// Thin network wrapper mirroring `marketplace::registry::fetch_servers`: the
/// glue is not unit-tested; the parse it delegates to is covered by
/// [`parse_marketplace_manifest`]'s tests.
pub fn fetch_remote_marketplace(url: &str) -> Result<Vec<PluginEntry>, WardError> {
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| WardError::Plugin(format!("fetch {url}: {e}")))?;
    let body = resp
        .into_string()
        .map_err(|e| WardError::Plugin(format!("read {url}: {e}")))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| WardError::Plugin(format!("parse {url}: {e}")))?;
    let name = nonempty_str(value.get("name")).unwrap_or_else(|| "remote".to_string());
    Ok(parse_marketplace_manifest(&value, &name))
}

// ---------------------------------------------------------------------------
// Merge + JSON helpers.
// ---------------------------------------------------------------------------

/// Overlay on-disk install state onto `entry`. A plugin absent from
/// `enabledPlugins` defaults to enabled (Claude Code treats a plugin as on
/// unless explicitly disabled). Installed version/scope win over the manifest.
fn apply_installed(
    entry: &mut PluginEntry,
    key: &str,
    installed: &HashMap<String, InstalledInfo>,
    enabled_map: &HashMap<String, bool>,
) {
    if let Some(info) = installed.get(key) {
        entry.installed = true;
        entry.enabled = enabled_map.get(key).copied().unwrap_or(true);
        if info.version.is_some() {
            entry.version = info.version.clone();
        }
        entry.scope = info.scope.clone();
    }
}

/// Overlay catalog metadata (installs / tokens / component counts) onto
/// `entry`. No-op for keys the catalog cache has no entry for.
fn apply_catalog(entry: &mut PluginEntry, key: &str, catalog: &HashMap<String, CatalogMeta>) {
    if let Some(meta) = catalog.get(key) {
        entry.unique_installs = meta.unique_installs;
        entry.always_on_tokens = meta.always_on_tokens;
        entry.on_invoke_tokens = meta.on_invoke_tokens;
        entry.component_counts = meta.component_counts.clone();
    }
}

/// Read + parse a JSON file, returning `None` on any error (missing file,
/// unreadable, or unparseable) so callers degrade to an empty result.
fn read_json(path: &Path) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Manifest `author` is either a bare string or an object `{name}` (the
/// official catalog uses the object form). Map both to `Option<String>`;
/// anything else → `None`.
fn parse_author(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Object(o)) => o
            .get("name")
            .and_then(|n| n.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// Manifest tags come from the `tags` array when present, else the legacy
/// `keywords` array, else empty (the official catalog ships neither).
fn manifest_tags(p: &Value) -> Vec<String> {
    let tags = string_array(p.get("tags"));
    if !tags.is_empty() {
        return tags;
    }
    string_array(p.get("keywords"))
}

/// A JSON string field mapped to `Some(String)` only when present and
/// non-empty; `None` otherwise.
fn nonempty_str(v: Option<&Value>) -> Option<String> {
    v.and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// A JSON array of strings mapped to `Vec<String>` (non-string elements
/// skipped); empty when the field is absent or not an array.
fn string_array(v: Option<&Value>) -> Vec<String> {
    v.and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|t| t.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const INSTALLED: &str = include_str!("fixtures/installed_plugins.json");
    const KNOWN: &str = include_str!("fixtures/known_marketplaces.json");
    const MANIFEST: &str = include_str!("fixtures/marketplace.json");
    const CACHE: &str = include_str!("fixtures/plugin-catalog-cache.json");

    fn v(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn parse_installed_collapses_multi_install_to_newest() {
        let map = parse_installed(&v(INSTALLED));
        let info = map
            .get("code-formatter@claude-plugins-official")
            .expect("code-formatter installed record present");
        // Two installs: 2026-01-01 (v1.0.0, user) and 2026-06-15 (v2.1.0,
        // project). Newest by lastUpdated wins.
        assert_eq!(info.version.as_deref(), Some("2.1.0"));
        assert_eq!(info.scope.as_deref(), Some("project"));
        assert!(info.install_path.contains("code-formatter/2.1.0"));
        assert!(map.contains_key("orphan-tool@side-marketplace"));
    }

    #[test]
    fn parse_known_marketplaces_reads_name_and_source() {
        let mkts = parse_known_marketplaces(&v(KNOWN));
        assert_eq!(mkts.len(), 1);
        let m = &mkts[0];
        assert_eq!(m.name, "claude-plugins-official");
        assert_eq!(
            m.source.get("repo").and_then(|r| r.as_str()),
            Some("anthropics/claude-plugins-official")
        );
        assert_eq!(
            m.install_location.as_deref(),
            Some("/home/tester/.claude/plugins/marketplaces/claude-plugins-official")
        );
        assert_eq!(m.last_updated.as_deref(), Some("2026-06-01T00:00:00.000Z"));
    }

    #[test]
    fn parse_marketplace_manifest_maps_fields() {
        let entries = parse_marketplace_manifest(&v(MANIFEST), "claude-plugins-official");
        assert_eq!(entries.len(), 3);

        // code-formatter — the REAL official shape: object `author` {name},
        // `git-subdir` source, and NO displayName/version/tags.
        let cf = entries.iter().find(|e| e.name == "code-formatter").unwrap();
        assert_eq!(cf.kind, "plugin");
        assert_eq!(cf.marketplace, "claude-plugins-official");
        assert_eq!(cf.description, "Formats your source code on save.");
        // author is an object {name} → extracted to a String.
        assert_eq!(cf.author.as_deref(), Some("Anthropic"));
        assert_eq!(cf.category.as_deref(), Some("dev"));
        // displayName absent → falls back to the bare name (never empty).
        assert_eq!(cf.display_name, "code-formatter");
        // version absent → None (not fabricated).
        assert!(cf.version.is_none());
        // neither tags nor keywords → empty.
        assert!(cf.tags.is_empty());
        // git-subdir source blob passes through opaque.
        assert_eq!(cf.source.get("source").and_then(|s| s.as_str()), Some("git-subdir"));
        assert_eq!(cf.source.get("path").and_then(|s| s.as_str()), Some("plugins/code-formatter"));
        // installed/enabled left false until the merge step fills them.
        assert!(!cf.installed);
        assert!(!cf.enabled);
        assert!(cf.scope.is_none());
        assert!(cf.unique_installs.is_none());

        // doc-writer — fully populated: string `author`, explicit
        // displayName, version, and a `tags` array.
        let dw = entries.iter().find(|e| e.name == "doc-writer").unwrap();
        assert_eq!(dw.author.as_deref(), Some("Anthropic"));
        assert_eq!(dw.display_name, "Doc Writer");
        assert_eq!(dw.version.as_deref(), Some("1.2.0"));
        assert_eq!(dw.tags, vec!["docs".to_string(), "writing".to_string()]);

        // legacy-linter — object author + `keywords` (no `tags`) → tags come
        // from the keywords fallback; displayName absent → name fallback.
        let ll = entries.iter().find(|e| e.name == "legacy-linter").unwrap();
        assert_eq!(ll.author.as_deref(), Some("Community"));
        assert_eq!(ll.display_name, "legacy-linter");
        assert!(ll.version.is_none());
        assert_eq!(ll.tags, vec!["lint".to_string(), "legacy".to_string()]);
    }

    #[test]
    fn parse_catalog_cache_reads_installs_tokens_components() {
        let map = parse_catalog_cache(&v(CACHE));
        let meta = map
            .get("code-formatter@claude-plugins-official")
            .expect("catalog entry present");
        assert_eq!(meta.unique_installs, Some(682));
        // First model in catalog.models is claude-opus-4-7 → its token fields.
        assert_eq!(meta.always_on_tokens, Some(1005));
        assert_eq!(meta.on_invoke_tokens, Some(15353));
        let cc = meta.component_counts.as_ref().expect("component counts present");
        assert_eq!(cc.commands, 1);
        assert_eq!(cc.agents, 0);
        assert_eq!(cc.skills, 2);
        assert_eq!(cc.hooks, 0);
        assert_eq!(cc.mcp_servers, 1);
        assert_eq!(cc.lsp_servers, 0);
    }

    #[test]
    fn fetch_marketplace_uses_manifest_parser() {
        // `fetch_remote_marketplace` is a thin `ureq` wrapper and is NOT
        // unit-tested (network) — mirroring `registry.rs`'s tested-parser /
        // untested-fetch split. This locks the parse path the fetch delegates
        // to: the same top-level `name` derivation + `parse_marketplace_manifest`
        // it runs on the fetched body. (The `ureq` call is exercised in the
        // hands-on smoke.)
        let value = v(MANIFEST);
        let name = nonempty_str(value.get("name")).unwrap_or_else(|| "remote".to_string());
        assert_eq!(name, "claude-plugins-official");
        let entries = parse_marketplace_manifest(&value, &name);
        assert!(!entries.is_empty(), "remote manifest yields >=1 plugin entry");
        let cf = entries
            .iter()
            .find(|e| e.name == "code-formatter")
            .expect("code-formatter present in remote manifest");
        assert_eq!(cf.marketplace, "claude-plugins-official");
    }

    #[test]
    fn parse_handles_garbage_without_panicking() {
        assert!(parse_installed(&json!({ "nope": 1 })).is_empty());
        assert!(parse_installed(&json!("string")).is_empty());
        assert!(parse_known_marketplaces(&json!([])).is_empty());
        assert!(parse_marketplace_manifest(&json!({}), "x").is_empty());
        assert!(parse_catalog_cache(&json!({ "catalog": 5 })).is_empty());
    }

    fn seed_home() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let plugins = home.join(".claude").join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("installed_plugins.json"), INSTALLED).unwrap();
        std::fs::write(plugins.join("known_marketplaces.json"), KNOWN).unwrap();
        std::fs::write(plugins.join("plugin-catalog-cache.json"), CACHE).unwrap();
        let mani_dir = plugins
            .join("marketplaces")
            .join("claude-plugins-official")
            .join(".claude-plugin");
        std::fs::create_dir_all(&mani_dir).unwrap();
        std::fs::write(mani_dir.join("marketplace.json"), MANIFEST).unwrap();
        // enabledPlugins carries an explicit false so we exercise the
        // "installed but disabled" path (vs. the default-enabled orphan).
        std::fs::write(
            home.join(".claude").join("settings.json"),
            r#"{"enabledPlugins":{"code-formatter@claude-plugins-official":false}}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn scan_plugins_merges_installed_and_enabled() {
        let dir = seed_home();
        let scan = scan_plugins(dir.path(), true);
        assert!(scan.cli_available);
        assert_eq!(scan.marketplaces.len(), 1);
        assert_eq!(scan.marketplaces[0].name, "claude-plugins-official");

        let by = |n: &str| {
            scan.plugins
                .iter()
                .find(|p| p.name == n)
                .unwrap_or_else(|| panic!("{n} expected in scan"))
        };

        // code-formatter: installed + explicitly disabled + catalog metadata,
        // installed version/scope override the manifest.
        let cf = by("code-formatter");
        assert!(cf.installed);
        assert!(!cf.enabled);
        assert_eq!(cf.version.as_deref(), Some("2.1.0"));
        assert_eq!(cf.scope.as_deref(), Some("project"));
        assert_eq!(cf.unique_installs, Some(682));
        assert_eq!(cf.always_on_tokens, Some(1005));
        assert_eq!(cf.on_invoke_tokens, Some(15353));
        assert_eq!(cf.component_counts.as_ref().unwrap().skills, 2);
        assert_eq!(cf.component_counts.as_ref().unwrap().mcp_servers, 1);

        // doc-writer: listed in the manifest but not installed → catalog
        // fields stay None, version from the manifest.
        let dw = by("doc-writer");
        assert!(!dw.installed);
        assert!(!dw.enabled);
        assert_eq!(dw.version.as_deref(), Some("1.2.0"));
        assert_eq!(dw.display_name, "Doc Writer");
        assert!(dw.unique_installs.is_none());

        // legacy-linter: manifest-only, object author + `keywords` fallback →
        // tags survive the merge; version absent stays None.
        let ll = by("legacy-linter");
        assert!(!ll.installed);
        assert_eq!(ll.author.as_deref(), Some("Community"));
        assert_eq!(ll.tags, vec!["lint".to_string(), "legacy".to_string()]);
        assert!(ll.version.is_none());

        // orphan-tool: installed but its marketplace manifest is absent →
        // still surfaced (nothing installed is hidden) and default-enabled.
        let ot = by("orphan-tool");
        assert!(ot.installed);
        assert!(ot.enabled);
        assert_eq!(ot.marketplace, "side-marketplace");
        assert_eq!(ot.version.as_deref(), Some("0.3.0"));

        // Order: marketplace plugins first, then absent-manifest installed.
        let names: Vec<&str> = scan.plugins.iter().map(|p| p.name.as_str()).collect();
        let cf_i = names.iter().position(|n| *n == "code-formatter").unwrap();
        let dw_i = names.iter().position(|n| *n == "doc-writer").unwrap();
        let ll_i = names.iter().position(|n| *n == "legacy-linter").unwrap();
        let ot_i = names.iter().position(|n| *n == "orphan-tool").unwrap();
        assert!(
            cf_i < ot_i && dw_i < ot_i && ll_i < ot_i,
            "orphan (absent-manifest installed) must sort after marketplace plugins"
        );
    }

    #[test]
    fn scan_plugins_missing_files_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let scan = scan_plugins(dir.path(), false);
        assert!(!scan.cli_available);
        assert!(scan.marketplaces.is_empty());
        assert!(scan.plugins.is_empty());
    }
}
