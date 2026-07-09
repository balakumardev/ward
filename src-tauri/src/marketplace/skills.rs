//! Plan 22 — Skills catalog for the Marketplace.
//!
//! Mirrors `registry.rs` (the MCP side): a **pure** parser fully unit-tested
//! against a pinned synthetic fixture, plus **thin `ureq` wrappers** (network,
//! not unit-tested) that only supply the body. Network is user-triggered only
//! (search on demand, install/preview on click) — never a background poll.
//!
//! Source shape — a Claude `.claude-plugin/marketplace.json` lists **plugins**,
//! each of which bundles one or more skills. The real anthropic/superpowers
//! manifests express `plugins[].skills[]` as an array of **relative dir-path
//! strings** (e.g. `"./skills/xlsx"`); Ward also tolerates the richer **object**
//! form (`{name, description, path, version}`). Each skill is unpacked into its
//! own [`MarketEntry`] (`kind: "skill"`), and the per-skill `SKILL.md` URL is
//! resolved so the install path can fetch it directly.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::MarketEntry;
use crate::error::WardError;

const TIMEOUT_SECS: u64 = 10;

/// A small, in-binary list of trusted Claude skill/plugin marketplaces —
/// `(display_name, raw marketplace.json URL)`. No network is used to *discover*
/// marketplaces; `marketplace_search(kind:"skill")` fetches exactly these.
pub const CURATED_MARKETPLACES: &[(&str, &str)] = &[
    (
        "Anthropic Agent Skills",
        "https://raw.githubusercontent.com/anthropics/skills/main/.claude-plugin/marketplace.json",
    ),
    (
        "Superpowers",
        "https://raw.githubusercontent.com/obra/superpowers/main/.claude-plugin/marketplace.json",
    ),
];

/// The fetched-and-parsed preview of a `SKILL.md` shown BEFORE install so the
/// user approves the actual content (spec §9.5 — bind approval to definition).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillPreview {
    /// Frontmatter `name` (falls back to the catalog entry's name).
    pub name: String,
    /// Frontmatter `description` (falls back to the catalog entry's).
    pub description: String,
    /// The raw `SKILL.md` body — rendered verbatim in the pre-install preview.
    pub body: String,
}

// ── Pure parse ───────────────────────────────────────────────────────────

/// Parse a `.claude-plugin/marketplace.json` body into skill [`MarketEntry`]s.
/// Pure and fully unit-tested. `marketplace_url` is the URL the manifest was
/// fetched from — used to resolve relative skill paths to raw `SKILL.md` URLs.
/// Tolerates a missing `plugins` array (→ empty) and skips unusable entries.
pub fn parse_marketplace(body: &str, marketplace_url: &str) -> Result<Vec<MarketEntry>, WardError> {
    let root: Value = serde_json::from_str(body)
        .map_err(|e| WardError::Registry(format!("parse marketplace.json: {e}")))?;
    let repo_base = repo_base_of(marketplace_url);

    let mut out = Vec::new();
    let Some(plugins) = root.get("plugins").and_then(|v| v.as_array()) else {
        return Ok(out);
    };
    for plugin in plugins {
        let plugin_desc = plugin
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let plugin_version = plugin
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        // A plugin's `source` roots its content. An absolute URL is used
        // verbatim (and shown as the repo); `"./"` / relative / missing falls
        // back to the manifest's own repo root.
        let source = plugin.get("source").and_then(|v| v.as_str());
        let base = plugin_base(source, &repo_base);

        match plugin.get("skills").and_then(|v| v.as_array()) {
            Some(skills) => {
                for sk in skills {
                    if let Some(entry) = skill_entry(sk, &plugin_desc, &plugin_version, &base) {
                        out.push(entry);
                    }
                }
            }
            // No enumerated skills → treat the plugin as a single skill only if
            // it points straight at one `SKILL.md`; otherwise it is an
            // un-unpackable bundle and is skipped (we never list a skill we
            // cannot resolve to a concrete SKILL.md to install).
            None => {
                if let Some(entry) = single_skill_plugin(plugin, &plugin_desc, &plugin_version, &base) {
                    out.push(entry);
                }
            }
        }
    }
    Ok(out)
}

/// Extract the frontmatter `name` / `description` from a `SKILL.md` body.
/// Pure; reuses the shared [`crate::fs_utils::parse_frontmatter`]. Missing
/// fields come back as empty strings.
pub fn parse_skill_md_meta(body: &str) -> (String, String) {
    let fm = crate::fs_utils::parse_frontmatter(body);
    let name = fm.get("name").cloned().unwrap_or_default();
    let description = fm.get("description").cloned().unwrap_or_default();
    (name, description)
}

/// Build one skill [`MarketEntry`] from a `plugins[].skills[]` element —
/// either a relative dir-path **string** (`"./skills/foo"`) or the richer
/// **object** form (`{name, description, path, version}`). Returns `None` when
/// no usable skill name can be derived.
fn skill_entry(
    sk: &Value,
    plugin_desc: &str,
    plugin_version: &Option<String>,
    base: &str,
) -> Option<MarketEntry> {
    let (name, description, version, raw_path, display) = if let Some(s) = sk.as_str() {
        // String form: the dir path is both the SKILL.md locator and the name.
        (name_from_skill_path(s), None, None, s.to_string(), None)
    } else if let Some(obj) = sk.as_object() {
        let raw_path = obj
            .get("path")
            .or_else(|| obj.get("source"))
            .or_else(|| obj.get("skillMd"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| name_from_skill_path(&raw_path));
        let description = obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let version = obj
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let display = obj
            .get("displayName")
            .or_else(|| obj.get("title"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        // An object with a name but no path still resolves under `<base>/<name>`.
        let locator = if raw_path.is_empty() { name.clone() } else { raw_path };
        (name, description, version, locator, display)
    } else {
        return None;
    };

    if name.is_empty() {
        return None;
    }
    let skill_md = skill_md_url(base, &raw_path);
    Some(build_skill_entry(
        name.clone(),
        display.unwrap_or_else(|| name.clone()),
        description.unwrap_or_else(|| plugin_desc.to_string()),
        version.or_else(|| plugin_version.clone()),
        base.to_string(),
        skill_md,
    ))
}

/// A plugin with no `skills[]` array but an explicit single-skill path
/// (`path` / `skillMd`) → one entry named after the plugin. Otherwise `None`.
fn single_skill_plugin(
    plugin: &Value,
    plugin_desc: &str,
    plugin_version: &Option<String>,
    base: &str,
) -> Option<MarketEntry> {
    let raw_path = plugin
        .get("path")
        .or_else(|| plugin.get("skillMd"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let name = plugin
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| name_from_skill_path(raw_path));
    if name.is_empty() {
        return None;
    }
    let skill_md = skill_md_url(base, raw_path);
    Some(build_skill_entry(
        name.clone(),
        name,
        plugin_desc.to_string(),
        plugin_version.clone(),
        base.to_string(),
        skill_md,
    ))
}

/// Assemble the canonical skill [`MarketEntry`] — always `kind:"skill"`,
/// `source:"marketplace"`, `verified:true`, no packages/remotes.
fn build_skill_entry(
    name: String,
    display_name: String,
    description: String,
    version: Option<String>,
    repo_url: String,
    skill_path: String,
) -> MarketEntry {
    MarketEntry {
        kind: "skill".into(),
        name,
        display_name,
        description,
        source: "marketplace".into(),
        version,
        verified: true,
        packages: vec![],
        remotes: vec![],
        install_shape: "discovery".into(),
        repo_url: Some(repo_url),
        skill_path: Some(skill_path),
    }
}

/// Reduce a manifest URL to its repo content root by stripping the trailing
/// `/.claude-plugin/marketplace.json` (or, failing that, the last path
/// segment). No trailing slash.
fn repo_base_of(marketplace_url: &str) -> String {
    let url = marketplace_url.trim_end_matches('/');
    if let Some(i) = url.find("/.claude-plugin/") {
        return url[..i].to_string();
    }
    match url.rfind('/') {
        Some(i) => url[..i].to_string(),
        None => url.to_string(),
    }
}

/// Resolve a plugin's content base: an absolute-URL `source` is used verbatim
/// (trailing slash trimmed); `"./"` / relative / missing → the repo root.
fn plugin_base(source: Option<&str>, repo_base: &str) -> String {
    match source {
        Some(s) if is_absolute_url(s) => s.trim_end_matches('/').to_string(),
        _ => repo_base.to_string(),
    }
}

/// The last path segment of a skill dir/file path, dropping a trailing
/// `SKILL.md`. `"./skills/foo"` → `foo`; `"skills/foo/SKILL.md"` → `foo`.
fn name_from_skill_path(raw: &str) -> String {
    let p = raw.trim().trim_start_matches("./").trim_matches('/');
    let dir = p
        .strip_suffix("/SKILL.md")
        .or_else(|| p.strip_suffix("/skill.md"))
        .unwrap_or(p);
    dir.rsplit('/').next().unwrap_or(dir).trim().to_string()
}

/// Resolve a skill's raw `SKILL.md` URL from `base` + a dir-or-file path. A
/// bare directory gets `/SKILL.md` appended; an absolute path is used verbatim.
fn skill_md_url(base: &str, raw_path: &str) -> String {
    let p = raw_path.trim().trim_start_matches("./").trim_matches('/');
    let file = if p.is_empty() {
        "SKILL.md".to_string()
    } else if ends_with_skill_md(p) {
        p.to_string()
    } else {
        format!("{p}/SKILL.md")
    };
    if is_absolute_url(&file) {
        file
    } else {
        format!("{}/{}", base.trim_end_matches('/'), file)
    }
}

fn ends_with_skill_md(p: &str) -> bool {
    let lower = p.to_ascii_lowercase();
    lower.ends_with("/skill.md") || lower == "skill.md"
}

fn is_absolute_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

// ── Network (thin wrappers, not unit-tested) ─────────────────────────────

/// Fetch + parse one marketplace manifest. Thin `ureq` GET (10 s); transport /
/// HTTP errors map to [`WardError::Registry`]. Delegates to [`parse_marketplace`].
pub fn fetch_marketplace(url: &str) -> Result<Vec<MarketEntry>, WardError> {
    let body = http_get(url)?;
    parse_marketplace(&body, url)
}

/// Fetch one raw `SKILL.md` body. Thin `ureq` GET (10 s); errors →
/// [`WardError::Registry`]. Used by both the pre-install preview and install.
pub fn fetch_skill_md(raw_url: &str) -> Result<String, WardError> {
    http_get(raw_url)
}

/// Fetch a skill's `SKILL.md` and parse its frontmatter into a [`SkillPreview`].
/// Wires the network fetch to the pure meta parse; the frontmatter wins, with
/// the catalog entry's name/description as the fallback.
pub fn preview_skill(entry: &MarketEntry) -> Result<SkillPreview, WardError> {
    let url = entry
        .skill_path
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| WardError::Registry(format!("skill '{}' has no source URL to fetch", entry.name)))?;
    let body = fetch_skill_md(url)?;
    let (name, description) = parse_skill_md_meta(&body);
    let name = if name.is_empty() { entry.name.clone() } else { name };
    let description = if description.is_empty() { entry.description.clone() } else { description };
    Ok(SkillPreview { name, description, body })
}

/// Shared thin GET used by both fetchers — 10 s timeout; transport / HTTP
/// status errors map to [`WardError::Registry`].
fn http_get(url: &str) -> Result<String, WardError> {
    let resp = match ureq::get(url).timeout(Duration::from_secs(TIMEOUT_SECS)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => {
            return Err(WardError::Registry(format!("fetching {url} returned HTTP {code}")));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(WardError::Registry(format!("network error fetching {url}: {t}")));
        }
    };
    resp.into_string()
        .map_err(|e| WardError::Registry(format!("read response body from {url}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("fixtures/marketplace.json");
    const MKT_URL: &str =
        "https://raw.githubusercontent.com/acme/agent-skills/main/.claude-plugin/marketplace.json";
    const BASE: &str = "https://raw.githubusercontent.com/acme/agent-skills/main";

    fn find<'a>(entries: &'a [MarketEntry], name: &str) -> &'a MarketEntry {
        entries.iter().find(|e| e.name == name).unwrap_or_else(|| panic!("no entry named {name}"))
    }

    #[test]
    fn unpacks_every_string_and_object_skill() {
        let entries = parse_marketplace(FIXTURE, MKT_URL).unwrap();
        // 2 (writing) + 1 (debug) + 1 (rich) = 4 skills across 3 plugins.
        assert_eq!(entries.len(), 4);
        for e in &entries {
            assert_eq!(e.kind, "skill");
            assert_eq!(e.source, "marketplace");
            assert!(e.verified);
            assert!(e.packages.is_empty() && e.remotes.is_empty());
            assert!(e.skill_path.is_some(), "every skill resolves a SKILL.md URL");
        }
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"brainstorming"));
        assert!(names.contains(&"writing-plans"));
        assert!(names.contains(&"systematic-debugging"));
        assert!(names.contains(&"verification"));
    }

    #[test]
    fn string_skill_resolves_raw_skill_md_url_and_inherits_plugin_desc() {
        let entries = parse_marketplace(FIXTURE, MKT_URL).unwrap();
        let e = find(&entries, "brainstorming");
        assert_eq!(e.display_name, "brainstorming");
        // A string-form skill carries no per-skill description → inherits the
        // plugin's, and no version.
        assert_eq!(e.description, "Writing and planning helpers bundle.");
        assert!(e.version.is_none());
        // source "./" → repo_url is the manifest's repo root; SKILL.md sits under it.
        assert_eq!(e.repo_url.as_deref(), Some(BASE));
        assert_eq!(
            e.skill_path.as_deref(),
            Some("https://raw.githubusercontent.com/acme/agent-skills/main/skills/brainstorming/SKILL.md")
        );
    }

    #[test]
    fn debug_skill_inherits_its_plugin_description() {
        let entries = parse_marketplace(FIXTURE, MKT_URL).unwrap();
        let e = find(&entries, "systematic-debugging");
        assert_eq!(e.description, "Systematic debugging technique.");
        assert_eq!(
            e.skill_path.as_deref(),
            Some("https://raw.githubusercontent.com/acme/agent-skills/main/skills/systematic-debugging/SKILL.md")
        );
    }

    #[test]
    fn object_skill_uses_its_own_metadata_and_plugin_source() {
        let entries = parse_marketplace(FIXTURE, MKT_URL).unwrap();
        let e = find(&entries, "verification");
        assert_eq!(e.description, "Verify work end-to-end before claiming completion.");
        assert_eq!(e.version.as_deref(), Some("2.0.0"));
        // Plugin source is an absolute URL → it becomes the base + repo_url.
        assert_eq!(e.repo_url.as_deref(), Some("https://github.com/acme/rich-skills"));
        assert_eq!(
            e.skill_path.as_deref(),
            Some("https://github.com/acme/rich-skills/skills/verification/SKILL.md")
        );
    }

    #[test]
    fn parse_skill_md_meta_reads_frontmatter() {
        let body = "---\nname: brainstorming\ndescription: Explore intent before building.\n---\n\n# Brainstorming\n\nBody.\n";
        let (name, desc) = parse_skill_md_meta(body);
        assert_eq!(name, "brainstorming");
        assert_eq!(desc, "Explore intent before building.");
    }

    #[test]
    fn parse_skill_md_meta_missing_frontmatter_is_empty() {
        let (name, desc) = parse_skill_md_meta("# No frontmatter\n\nJust a body.\n");
        assert!(name.is_empty());
        assert!(desc.is_empty());
    }

    #[test]
    fn tolerates_missing_plugins_and_bad_json() {
        assert!(parse_marketplace("{}", MKT_URL).unwrap().is_empty());
        assert!(parse_marketplace(r#"{"plugins":[]}"#, MKT_URL).unwrap().is_empty());
        let err = parse_marketplace("not json", MKT_URL).unwrap_err();
        assert!(matches!(err, WardError::Registry(_)));
    }

    #[test]
    fn single_skill_plugin_without_skills_array_uses_explicit_path() {
        // A plugin that ships one skill and points straight at its SKILL.md.
        let body = r#"{"plugins":[
            {"name":"solo","description":"A single-skill plugin.","source":"./",
             "path":"skills/solo/SKILL.md"}
        ]}"#;
        let entries = parse_marketplace(body, MKT_URL).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "solo");
        assert_eq!(
            entries[0].skill_path.as_deref(),
            Some("https://raw.githubusercontent.com/acme/agent-skills/main/skills/solo/SKILL.md")
        );
    }

    #[test]
    fn skips_unusable_skill_and_plugin_entries() {
        // Empty/only-dot skill path → no name → skipped. Plugin with no skills
        // and no path → skipped. A nameless plugin is skipped for its fallback.
        let body = r#"{"plugins":[
            {"name":"bundle","description":"d","source":"./","skills":["./","  "]},
            {"name":"bare","description":"no skills, no path","source":"./"}
        ]}"#;
        let entries = parse_marketplace(body, MKT_URL).unwrap();
        assert!(entries.is_empty(), "got {entries:?}");
    }
}
