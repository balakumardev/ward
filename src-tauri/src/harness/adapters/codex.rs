//! codex.rs — Codex CLI harness adapter.
//!
//! Second harness; capability-gated parity. Ported from CCO
//! `src/harness/adapters/codex.mjs`. Eleven categories:
//!
//!   config, memory, skill, mcp, profile, rule, plugin,
//!   session, history, shell, runtime
//!
//! Codex capabilities are intentionally minimal — it does not expose
//! context-budget, mcp-controls, mcp-policy, or effective resolution
//! to Ward. The UI must hide those modes when Codex is selected.
//!
//! Storage layout (mirrors CCO):
//!   Global: `~/.codex/`
//!     - config.toml
//!     - AGENTS.md, AGENTS.override.md
//!     - memories/*.md
//!     - skills/<name>/SKILL.md (incl. skills/.system/<name>/SKILL.md)
//!     - rules/<name>.rules
//!     - plugins/<...>/.codex-plugin/plugin.json
//!     - sessions/<rollout-...>.jsonl + session_index.jsonl
//!     - history.jsonl
//!     - shell_snapshots/<uuid>.sh
//!     - version.json, installation_id, models_cache.json,
//!       state_5.sqlite*, logs_2.sqlite*, log/codex-tui.log,
//!       bin/codex-notify-*.sh, .personality_migration
//!   Project: `<repo>/.codex/` (config.toml), `<repo>/AGENTS.md`,
//!     `<repo>/.codex/skills/`, `<repo>/.agents/skills/`.
//!
//! Move / delete: not yet supported. The adapter returns the structured
//! "unsupported" error shape (matches CCO's `unsupportedOperations`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml::Value as TomlValue;

use crate::error::WardError;
use crate::harness::{Ctx, Harness};
use crate::model::{Capabilities, HarnessItem, Scope};

// ── Adapter metadata ───────────────────────────────────────────────────

/// The single Codex harness adapter. Constructed as a unit struct;
/// the trait methods are stateless except for the read-only filesystem
/// in `ctx`.
pub struct CodexAdapter;

impl CodexAdapter {
    /// `~/.codex` — global Codex home.
    fn codex_root(home: &Path) -> PathBuf { home.join(".codex") }

    /// Project scope id — base64url-encoded absolute path (CCO parity).
    fn project_scope_id(repo_dir: &Path) -> String {
        let s = repo_dir.to_string_lossy();
        // base64url without padding — matches Node Buffer.from(...).toString("base64url")
        base64_url_encode(s.as_bytes())
    }

    fn project_scope_label(repo_dir: &Path) -> String {
        repo_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| repo_dir.to_string_lossy().to_string())
    }
}

fn base64_url_encode(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    URL_SAFE_NO_PAD.encode(bytes)
}

// ── Wire struct: parsed config.toml ────────────────────────────────────

/// Parsed `~/.codex/config.toml`. We only model the fields Codex
/// surfaces to Ward; everything else is read through `extra` for the
/// generic TOML view.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CodexConfig {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    approval_policy: Option<String>,
    #[serde(default)]
    sandbox_mode: Option<String>,
    #[serde(default)]
    mcp_servers: HashMap<String, TomlValue>,
    #[serde(default)]
    profiles: HashMap<String, TomlValue>,
    #[serde(default)]
    projects: HashMap<String, TomlValue>,
    #[serde(default)]
    project_root_markers: Vec<String>,
    #[serde(default)]
    project_doc_fallback_filenames: Vec<String>,
}

// ── Trait impl ──────────────────────────────────────────────────────────

impl Harness for CodexAdapter {
    fn id(&self) -> &str { "codex" }
    fn display_name(&self) -> &str { "Codex CLI" }
    fn short_name(&self) -> &str { "Codex" }
    fn icon(&self) -> &str { "◇" }
    fn executable(&self) -> &str { "codex" }

    fn category_ids(&self) -> Vec<&'static str> {
        // Eleven categories, in CCO display order.
        vec![
            "config", "memory", "skill", "mcp", "profile",
            "rule", "plugin", "session", "history", "shell", "runtime",
        ]
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            context_budget: false,
            mcp_controls: false,
            mcp_policy: false,
            mcp_security: true,
            sessions: true,
            effective: false,
            backup: true,
            // Codex MCP stays read-only until an upsert backend exists.
            mcp_editable: false,
        }
    }

    fn discover_scopes(&self, ctx: &Ctx) -> Result<Vec<Scope>, WardError> {
        let mut scopes: Vec<Scope> = vec![Scope {
            id: "global".into(),
            kind: "global".into(),
            label: "Global (~/.codex)".into(),
            root: Self::codex_root(ctx.home).display().to_string(),
        }];

        // 1) Projects declared in the global config.toml via [projects."<path>"]
        let cfg = read_codex_config(ctx);
        for (project_path, project_value) in &cfg.projects {
            if project_path.is_empty() { continue; }
            let repo = PathBuf::from(project_path);
            if !repo.exists() { continue; }
            scopes.push(Scope {
                id: Self::project_scope_id(&repo),
                kind: "project".into(),
                label: Self::project_scope_label(&repo),
                root: repo.display().to_string(),
            });
        }

        // 2) Repo derived from cwd (CCO heuristic: walk up until a project_root_marker)
        if let Some(cwd) = ctx.cwd {
            let markers = if !cfg.project_root_markers.is_empty() {
                cfg.project_root_markers.clone()
            } else {
                vec![".git".to_string()]
            };
            let fallback = cfg.project_doc_fallback_filenames.clone();
            if let Some(repo_root) = find_project_root(cwd, &markers) {
                let candidates = dirs_between_repo_and_cwd(&repo_root, cwd);
                for dir in candidates {
                    if dir == ctx.home { continue; }
                    if !has_codex_project_artifacts(&dir, &fallback) { continue; }
                    if scopes.iter().any(|s| s.root == dir.display().to_string()) { continue; }
                    scopes.push(Scope {
                        id: Self::project_scope_id(&dir),
                        kind: "project".into(),
                        label: Self::project_scope_label(&dir),
                        root: dir.display().to_string(),
                    });
                }
            }
        }

        Ok(scopes)
    }

    fn scan_category(&self, ctx: &Ctx, category: &str, scope: &Scope)
        -> Result<Vec<HarnessItem>, WardError>
    {
        let items = match category {
            "config"  => scan_config(scope, ctx),
            "memory"  => scan_memories(scope, ctx),
            "skill"   => scan_skills(scope, ctx),
            "mcp"     => scan_mcp_servers(scope, ctx),
            "profile" => scan_profiles(scope, ctx),
            "rule"    => scan_rules(scope, ctx),
            "plugin"  => scan_plugins(scope, ctx),
            "session" => scan_sessions(scope, ctx),
            "history" => scan_history(scope, ctx),
            "shell"   => scan_shell_snapshots(scope, ctx),
            "runtime" => scan_runtime(scope, ctx),
            _ => vec![],
        };
        Ok(items)
    }
}

// ── Config.toml reader ──────────────────────────────────────────────────

fn read_codex_config(ctx: &Ctx) -> CodexConfig {
    let path = CodexAdapter::codex_root(ctx.home).join("config.toml");
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    toml::from_str(&content).unwrap_or_default()
}

// ── Project root discovery ─────────────────────────────────────────────

fn find_project_root(start: &Path, markers: &[String]) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        for marker in markers {
            if current.join(marker).exists() {
                return Some(current);
            }
        }
        if !current.pop() { return None; }
    }
}

fn dirs_between_repo_and_cwd(repo: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut current = cwd.to_path_buf();
    let mut out: Vec<PathBuf> = vec![];
    loop {
        out.push(current.clone());
        if current == repo || current.parent().is_none() { break; }
        current = current.parent().unwrap().to_path_buf();
        if current.as_os_str().is_empty() { break; }
    }
    out.reverse();
    out
}

fn has_codex_project_artifacts(dir: &Path, fallback: &[String]) -> bool {
    let candidates = [
        "AGENTS.override.md", "AGENTS.md", ".codex", ".agents/skills",
    ];
    for c in candidates {
        if dir.join(c).exists() { return true; }
    }
    for name in fallback {
        if !name.is_empty() && dir.join(name).exists() { return true; }
    }
    false
}

// ── Category: config ────────────────────────────────────────────────────

fn scan_config(scope: &Scope, ctx: &Ctx) -> Vec<HarnessItem> {
    let mut items = Vec::new();

    if scope.id != "global" {
        // Project scope: surface the project-trust entry + project-local files.
        let cfg_path = CodexAdapter::codex_root(ctx.home).join("config.toml");
        let parsed = read_codex_config(ctx);
        let project_value = parsed.projects.get(&scope.root).cloned();
        if let Some(pv) = project_value {
            let bytes = serde_json::to_string(&pv_to_json(&pv))
                .map(|s| s.len()).unwrap_or(0);
            items.push(HarnessItem {
                category: "config".into(),
                scope_id: scope.id.clone(),
                name: "config.toml project entry".into(),
                description: format!("trust: {} ({})",
                    pv_to_trust(&pv).unwrap_or_default(),
                    scope.root),
                path: cfg_path.display().to_string(),
                movable: false, deletable: false, locked: false,
                effective: None, mcp_config: None,
            });
            let _ = bytes;
        }
        // AGENTS.override.md, AGENTS.md, fallback names, .codex/config.toml
        let cfg_scope = read_codex_config(ctx);
        let fallback = cfg_scope.project_doc_fallback_filenames.clone();
        let candidates: [(&str, &str); 3] = [
            ("AGENTS.override.md", "Project-local override instructions"),
            ("AGENTS.md", "Project instructions"),
            (".codex/config.toml", "Project Codex config layer"),
        ];
        for (name, desc) in candidates.iter() {
            push_config_file(&mut items, scope, &PathBuf::from(scope.root.clone()).join(name), name, desc);
        }
        for fb in &fallback {
            push_config_file(
                &mut items,
                scope,
                &PathBuf::from(scope.root.clone()).join(fb),
                fb.as_str(),
                "Project instruction fallback",
            );
        }
        return items;
    }

    // Global scope: surface config.toml + global AGENTS*.md
    let cfg_path = CodexAdapter::codex_root(ctx.home).join("config.toml");
    if let Some(item) = build_config_toml_item(scope, &cfg_path) {
        items.push(item);
    }
    push_config_file(&mut items, scope,
        &CodexAdapter::codex_root(ctx.home).join("AGENTS.override.md"),
        "AGENTS.override.md", "Global Codex override instructions");
    push_config_file(&mut items, scope,
        &CodexAdapter::codex_root(ctx.home).join("AGENTS.md"),
        "AGENTS.md", "Global Codex instructions");
    items
}

fn pv_to_json(v: &TomlValue) -> serde_json::Value {
    serde_json::to_value(toml_to_json(v)).unwrap_or(serde_json::Value::Null)
}

fn pv_to_trust(v: &TomlValue) -> Option<String> {
    if let Some(t) = v.get("trust_level").and_then(|x| x.as_str()) {
        Some(t.to_string())
    } else {
        None
    }
}

fn toml_to_json(v: &TomlValue) -> serde_json::Value {
    match v {
        TomlValue::String(s) => serde_json::Value::String(s.clone()),
        TomlValue::Integer(i) => serde_json::Value::Number((*i).into()),
        TomlValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
        TomlValue::Boolean(b) => serde_json::Value::Bool(*b),
        TomlValue::Array(arr) => serde_json::Value::Array(
            arr.iter().map(toml_to_json).collect()
        ),
        TomlValue::Table(t) => serde_json::Value::Object(
            t.iter().map(|(k, v)| (k.clone(), toml_to_json(v))).collect()
        ),
        TomlValue::Datetime(d) => serde_json::Value::String(d.to_string()),
    }
}

fn push_config_file(items: &mut Vec<HarnessItem>, scope: &Scope,
                    path: &Path, name: &str, desc: &str) {
    if !path.is_file() { return; }
    items.push(HarnessItem {
        category: "config".into(),
        scope_id: scope.id.clone(),
        name: name.to_string(),
        description: desc.to_string(),
        path: path.display().to_string(),
        movable: false, deletable: false, locked: true,
        effective: None, mcp_config: None,
    });
}

fn build_config_toml_item(scope: &Scope, path: &Path) -> Option<HarnessItem> {
    if !path.is_file() { return None; }
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let parsed: CodexConfig = toml::from_str(&content).ok()?;
    let desc = config_description(&parsed);
    Some(HarnessItem {
        category: "config".into(),
        scope_id: scope.id.clone(),
        name: "config.toml".into(),
        description: desc,
        path: path.display().to_string(),
        movable: false, deletable: false, locked: true,
        effective: None, mcp_config: None,
    })
}

fn config_description(cfg: &CodexConfig) -> String {
    let parts: Vec<String> = [
        cfg.model.as_deref().map(|m| format!("model: {m}")),
        cfg.approval_policy.as_deref().map(|a| format!("approval: {a}")),
        cfg.sandbox_mode.as_deref().map(|s| format!("sandbox: {s}")),
    ].into_iter().flatten().collect();
    if parts.is_empty() {
        "Codex CLI configuration".to_string()
    } else {
        parts.join(", ")
    }
}

// ── Category: memory ────────────────────────────────────────────────────

fn scan_memories(scope: &Scope, ctx: &Ctx) -> Vec<HarnessItem> {
    if scope.id != "global" { return vec![]; }
    let dir = CodexAdapter::codex_root(ctx.home).join("memories");
    let mut items = Vec::new();
    let entries = match std::fs::read_dir(&dir) { Ok(e) => e, Err(_) => return items };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        if path.extension().and_then(|s| s.to_str()) != Some("md") { continue; }
        let name = entry.file_name().to_string_lossy().to_string();
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let fm = crate::fs_utils::parse_frontmatter(&content);
        let display = fm.get("name").cloned()
            .unwrap_or_else(|| name.trim_end_matches(".md").to_string());
        items.push(HarnessItem {
            category: "memory".into(),
            scope_id: scope.id.clone(),
            name: display,
            description: fm.get("description").cloned().unwrap_or_default(),
            path: path.display().to_string(),
            movable: false, deletable: true, locked: false,
            effective: None, mcp_config: None,
        });
    }
    items
}

// ── Category: skill ─────────────────────────────────────────────────────

fn scan_skill_dirs(root: &Path, current: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if depth > 3 || !current.exists() { return out; }
    if current.join("SKILL.md").is_file() {
        out.push(current.to_path_buf());
        return out;
    }
    let entries = match std::fs::read_dir(current) { Ok(e) => e, Err(_) => return out };
    for entry in entries.flatten() {
        let ft = match entry.file_type() { Ok(t) => t, Err(_) => continue };
        if !ft.is_dir() && !ft.is_symlink() { continue; }
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(name.as_str(), "node_modules" | ".git") { continue; }
        out.extend(scan_skill_dirs(root, &current.join(&name), depth + 1));
    }
    out
}

fn skill_root_for(scope: &Scope, ctx: &Ctx) -> Vec<(PathBuf, &'static str, &'static str)> {
    if scope.id == "global" {
        vec![
            (CodexAdapter::codex_root(ctx.home).join("skills"),
             "$CODEX_HOME/skills", "skill"),
            (ctx.home.join(".agents").join("skills"),
             "~/.agents/skills", "skill"),
        ]
    } else {
        vec![
            (PathBuf::from(&scope.root).join(".codex").join("skills"),
             ".codex/skills", "repo-skill"),
            (PathBuf::from(&scope.root).join(".agents").join("skills"),
             ".agents/skills", "repo-skill"),
        ]
    }
}

fn scan_skills(scope: &Scope, ctx: &Ctx) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    for (root, source_label, default_sub) in skill_root_for(scope, ctx) {
        for skill_dir in scan_skill_dirs(&root, &root, 0) {
            let skill_md = skill_dir.join("SKILL.md");
            let rel = skill_dir.strip_prefix(&root).map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| skill_dir.to_string_lossy().to_string());
            let sub = if rel.starts_with(".system/") { "system-skill" } else { default_sub };
            let content = std::fs::read_to_string(&skill_md).unwrap_or_default();
            items.push(HarnessItem {
                category: "skill".into(),
                scope_id: scope.id.clone(),
                name: rel.clone(),
                description: String::new(),
                path: skill_dir.display().to_string(),
                movable: false, deletable: true, locked: false,
                effective: None, mcp_config: None,
            });
            let last = items.last_mut().unwrap();
            last.description = markdown_description(&content);
            let _ = sub;
            let _ = source_label;
        }
    }
    items
}

fn markdown_description(content: &str) -> String {
    let mut past_heading = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# ") { past_heading = true; continue; }
        if !past_heading && trimmed.starts_with("---") { continue; }
        if trimmed.is_empty() || trimmed.starts_with("```")
            || trimmed.starts_with('-') || trimmed.starts_with('|')
            || trimmed.starts_with('#') { continue; }
        if trimmed.contains(": ") && !trimmed.starts_with(' ') {
            let head = trimmed.split(':').next().unwrap_or("");
            if head.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && head.len() <= 30 { continue; }
        }
        return trimmed.chars().take(120).collect();
    }
    String::new()
}

// ── Category: mcp ───────────────────────────────────────────────────────

fn scan_mcp_servers(scope: &Scope, ctx: &Ctx) -> Vec<HarnessItem> {
    // Global scope reads `~/.codex/config.toml`. Project scope reads
    // `<repo>/.codex/config.toml`. CCO's `readScopeConfig` picks the
    // same path.
    let (servers, cfg_path) = if scope.id == "global" {
        let parsed = read_codex_config(ctx);
        (parsed.mcp_servers, CodexAdapter::codex_root(ctx.home).join("config.toml"))
    } else {
        let parsed = read_project_config(scope);
        (parsed.mcp_servers, PathBuf::from(&scope.root).join(".codex").join("config.toml"))
    };
    servers.into_iter()
        .map(|(name, server)| {
            let json = toml_to_json(&server);
            let cmd = json.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let url = json.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let args = json.get("args").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            let desc = if !cmd.is_empty() {
                format!("{} {}", cmd, args).trim().to_string()
            } else if !url.is_empty() {
                url.to_string()
            } else {
                "(MCP server)".to_string()
            };
            HarnessItem {
                category: "mcp".into(),
                scope_id: scope.id.clone(),
                name,
                description: desc,
                path: cfg_path.display().to_string(),
                movable: false, deletable: false, locked: false,
                effective: None,
                mcp_config: Some(json),
            }
        })
        .collect()
}

fn read_project_config(scope: &Scope) -> CodexConfig {
    let path = PathBuf::from(&scope.root).join(".codex").join("config.toml");
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    toml::from_str(&content).unwrap_or_default()
}

// ── Category: profile ───────────────────────────────────────────────────

fn scan_profiles(scope: &Scope, ctx: &Ctx) -> Vec<HarnessItem> {
    let (profiles, cfg_path) = if scope.id == "global" {
        let parsed = read_codex_config(ctx);
        (parsed.profiles, CodexAdapter::codex_root(ctx.home).join("config.toml"))
    } else {
        let parsed = read_project_config(scope);
        (parsed.profiles, PathBuf::from(&scope.root).join(".codex").join("config.toml"))
    };
    profiles.into_iter()
        .map(|(name, profile)| {
            let json = toml_to_json(&profile);
            let model = json.get("model").and_then(|v| v.as_str()).unwrap_or("");
            let approval = json.get("approval_policy").and_then(|v| v.as_str()).unwrap_or("");
            let sandbox = json.get("sandbox_mode").and_then(|v| v.as_str()).unwrap_or("");
            let desc = [
                if !model.is_empty()    { format!("model: {model}") } else { String::new() },
                if !approval.is_empty() { format!("approval: {approval}") } else { String::new() },
                if !sandbox.is_empty()  { format!("sandbox: {sandbox}") } else { String::new() },
            ].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
            let desc = if desc.is_empty() { "Codex profile".to_string() } else { desc };
            HarnessItem {
                category: "profile".into(),
                scope_id: scope.id.clone(),
                name,
                description: desc,
                path: cfg_path.display().to_string(),
                movable: false, deletable: false, locked: false,
                effective: None, mcp_config: None,
            }
        })
        .collect()
}

// ── Category: rule ──────────────────────────────────────────────────────

fn scan_rules(scope: &Scope, ctx: &Ctx) -> Vec<HarnessItem> {
    if scope.id != "global" { return vec![]; }
    let dir = CodexAdapter::codex_root(ctx.home).join("rules");
    let mut items = Vec::new();
    let entries = match std::fs::read_dir(&dir) { Ok(e) => e, Err(_) => return items };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') { continue; }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        items.push(HarnessItem {
            category: "rule".into(),
            scope_id: scope.id.clone(),
            name: name.clone(),
            description: markdown_description(&content),
            path: path.display().to_string(),
            movable: false, deletable: true, locked: false,
            effective: None, mcp_config: None,
        });
    }
    items
}

// ── Category: plugin ────────────────────────────────────────────────────

fn find_plugin_manifests(root: &Path, current: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if depth > 6 || !current.exists() { return out; }
    let manifest = current.join(".codex-plugin").join("plugin.json");
    if manifest.is_file() {
        out.push(manifest);
        return out;
    }
    let entries = match std::fs::read_dir(current) { Ok(e) => e, Err(_) => return out };
    for entry in entries.flatten() {
        let ft = match entry.file_type() { Ok(t) => t, Err(_) => continue };
        if !ft.is_dir() && !ft.is_symlink() { continue; }
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(name.as_str(), "node_modules" | ".git") { continue; }
        out.extend(find_plugin_manifests(root, &current.join(&name), depth + 1));
    }
    out
}

fn scan_plugins(scope: &Scope, ctx: &Ctx) -> Vec<HarnessItem> {
    if scope.id != "global" { return vec![]; }
    let root = CodexAdapter::codex_root(ctx.home).join("plugins");
    let mut items = Vec::new();
    for manifest in find_plugin_manifests(&root, &root, 0) {
        let content = std::fs::read_to_string(&manifest).unwrap_or_default();
        let json: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or(serde_json::Value::Null);
        let plugin_dir = manifest.parent().and_then(|p| p.parent()).unwrap_or(&root);
        let name = json.get("name").and_then(|v| v.as_str())
            .or_else(|| json.get("id").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                plugin_dir.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "plugin".into())
            });
        let desc = json.get("description").and_then(|v| v.as_str())
            .or_else(|| json.get("displayName").and_then(|v| v.as_str()))
            .unwrap_or("Codex plugin")
            .to_string();
        items.push(HarnessItem {
            category: "plugin".into(),
            scope_id: scope.id.clone(),
            name,
            description: desc,
            path: plugin_dir.display().to_string(),
            movable: false, deletable: false, locked: false,
            effective: None, mcp_config: None,
        });
    }
    items
}

// ── Category: session ───────────────────────────────────────────────────

fn extract_session_id(file_name: &str) -> String {
    // rollout-YYYY-MM-DDTHH-MM-SS-<UUID>.jsonl — CCO uses
    // `rollout-\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-([0-9a-f-]{36})\.jsonl$`.
    // Hand-roll the same parse so we don't pull in the regex crate for
    // one use case.
    let stem = match file_name.strip_suffix(".jsonl") {
        Some(s) => s,
        None => return file_name.to_string(),
    };
    // Walk backwards 36 chars and check it looks like a UUID (5 groups).
    if stem.len() >= 36 {
        let tail = &stem[stem.len() - 36..];
        if looks_like_uuid(tail) {
            return tail.to_string();
        }
    }
    stem.to_string()
}

fn looks_like_uuid(s: &str) -> bool {
    if s.len() != 36 { return false; }
    let dash_positions = [8usize, 13, 18, 23];
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if dash_positions.contains(&i) {
            if *b != b'-' { return false; }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn scan_sessions(scope: &Scope, ctx: &Ctx) -> Vec<HarnessItem> {
    if scope.id != "global" { return vec![]; }
    let root = CodexAdapter::codex_root(ctx.home).join("sessions");
    let index_path = CodexAdapter::codex_root(ctx.home).join("session_index.jsonl");
    let mut items = Vec::new();
    if index_path.is_file() {
        items.push(HarnessItem {
            category: "session".into(),
            scope_id: scope.id.clone(),
            name: "session_index.jsonl".into(),
            description: "Codex session index".into(),
            path: index_path.display().to_string(),
            movable: false, deletable: false, locked: false,
            effective: None, mcp_config: None,
        });
    }
    let entries = match std::fs::read_dir(&root) { Ok(e) => e, Err(_) => return items };
    let mut paths: Vec<PathBuf> = entries.flatten()
        .filter(|e| e.path().is_file()
            && e.path().extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .map(|e| e.path())
        .collect();
    paths.sort();
    for path in paths {
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let session_id = extract_session_id(&file_name);
        items.push(HarnessItem {
            category: "session".into(),
            scope_id: scope.id.clone(),
            name: session_id.clone(),
            description: file_name,
            path: path.display().to_string(),
            movable: false, deletable: false, locked: false,
            effective: None, mcp_config: None,
        });
    }
    items
}

// ── Category: history ───────────────────────────────────────────────────

fn scan_history(scope: &Scope, ctx: &Ctx) -> Vec<HarnessItem> {
    if scope.id != "global" { return vec![]; }
    let path = CodexAdapter::codex_root(ctx.home).join("history.jsonl");
    if !path.is_file() { return vec![]; }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    let last = lines.last().copied().unwrap_or("");
    let last_text = last.split("\"text\":").nth(1)
        .map(|s| s.split('"').nth(1).unwrap_or("").to_string())
        .unwrap_or_default();
    let desc = if last_text.is_empty() {
        format!("{} prompt history entries", lines.len())
    } else {
        format!("{} prompt history entries; latest: {}", lines.len(), last_text)
    };
    vec![HarnessItem {
        category: "history".into(),
        scope_id: scope.id.clone(),
        name: "history.jsonl".into(),
        description: desc,
        path: path.display().to_string(),
        movable: false, deletable: false, locked: false,
        effective: None, mcp_config: None,
    }]
}

// ── Category: shell ─────────────────────────────────────────────────────

fn scan_shell_snapshots(scope: &Scope, ctx: &Ctx) -> Vec<HarnessItem> {
    if scope.id != "global" { return vec![]; }
    let dir = CodexAdapter::codex_root(ctx.home).join("shell_snapshots");
    let mut items = Vec::new();
    let entries = match std::fs::read_dir(&dir) { Ok(e) => e, Err(_) => return items };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        if path.extension().and_then(|s| s.to_str()) != Some("sh") { continue; }
        let name = entry.file_name().to_string_lossy().to_string();
        items.push(HarnessItem {
            category: "shell".into(),
            scope_id: scope.id.clone(),
            name: name.clone(),
            description: "Shell environment snapshot".into(),
            path: path.display().to_string(),
            movable: false, deletable: false, locked: false,
            effective: None, mcp_config: None,
        });
    }
    items
}

// ── Category: runtime ───────────────────────────────────────────────────

const RUNTIME_FILES: &[&str] = &[
    "version.json", "installation_id", "models_cache.json",
    "state_5.sqlite", "state_5.sqlite-shm", "state_5.sqlite-wal",
    "logs_2.sqlite", "logs_2.sqlite-shm", "logs_2.sqlite-wal",
    ".personality_migration", "log/codex-tui.log",
    "bin/codex-notify-focus.sh", "bin/codex-notify-turn.sh",
];

fn runtime_description(name: &str) -> String {
    if name == "version.json" { return "Codex version metadata".into(); }
    if name == "models_cache.json" { return "Model cache".into(); }
    if name == "installation_id" { return "Codex installation identifier".into(); }
    if name.ends_with(".sqlite") { return "Codex SQLite state database".into(); }
    if name.ends_with(".sqlite-shm") { return "SQLite shared-memory sidecar".into(); }
    if name.ends_with(".sqlite-wal") { return "SQLite write-ahead log".into(); }
    if name.ends_with(".log") { return "Codex TUI log".into(); }
    if name.ends_with(".sh") { return "Codex notification script".into(); }
    if name == ".personality_migration" { return "Personality migration marker".into(); }
    "Codex runtime file".into()
}

fn scan_runtime(scope: &Scope, ctx: &Ctx) -> Vec<HarnessItem> {
    if scope.id != "global" { return vec![]; }
    let root = CodexAdapter::codex_root(ctx.home);
    let mut items = Vec::new();
    for rel in RUNTIME_FILES {
        let path = root.join(rel);
        if !path.exists() { continue; }
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        items.push(HarnessItem {
            category: "runtime".into(),
            scope_id: scope.id.clone(),
            name: rel.to_string(),
            description: runtime_description(&file_name),
            path: path.display().to_string(),
            movable: false, deletable: false, locked: false,
            effective: None, mcp_config: None,
        });
    }
    items
}

// ════════════════════════════════════════════════════════════════════════
// TESTS — ported from CCO `tests/unit/test-codex-adapter.mjs`.
// Each test asserts parity with the JS golden, exercising one or more
// of the 11 categories against a fixture home built via tempfile.
// ════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Build a fixture `home` containing the full set of Codex files
    /// that CCO's `createCodexHome` produces. Used by every global-scope
    /// assertion in this module.
    fn fixture_codex_home() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let codex = home.join(".codex");

        fs::create_dir_all(codex.join("memories")).unwrap();
        fs::create_dir_all(codex.join("skills").join("demo-skill")).unwrap();
        fs::create_dir_all(codex.join("skills").join(".system").join("system-skill")).unwrap();
        fs::create_dir_all(codex.join("rules")).unwrap();
        fs::create_dir_all(
            codex.join("plugins").join("cache").join("openai-curated")
                .join("github").join("abc123").join(".codex-plugin")
        ).unwrap();

        fs::write(codex.join("config.toml"), "\
model = \"gpt-5.5\"\n\
approval_policy = \"never\"\n\
sandbox_mode = \"danger-full-access\"\n\
\n\
[profiles.review]\n\
sandbox_mode = \"read-only\"\n\
approval_policy = \"never\"\n\
\n\
[mcp_servers.context7]\n\
command = \"npx\"\n\
args = [\"-y\", \"@upstash/context7-mcp\"]\n\
\n\
[mcp_servers.remote]\n\
url = \"https://example.com/mcp\"\n\
").unwrap();

        fs::write(codex.join("memories").join("project.md"), "\
---\n\
name: Project Memory\n\
description: Important project context\n\
type: project\n\
---\n\
# Project Memory\n\
").unwrap();

        fs::write(codex.join("skills").join("demo-skill").join("SKILL.md"),
            "# Demo Skill\n\nUse this for adapter smoke tests.\n").unwrap();
        fs::write(codex.join("skills").join(".system").join("system-skill").join("SKILL.md"),
            "# System Skill\n\nNested system skill layout.\n").unwrap();

        fs::write(codex.join("rules").join("default.rules"),
            "always respond with concise engineering notes\n").unwrap();
        fs::write(
            codex.join("plugins").join("cache").join("openai-curated")
                .join("github").join("abc123").join(".codex-plugin").join("plugin.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "name": "github",
                "description": "GitHub plugin",
            })).unwrap(),
        ).unwrap();

        (dir, home)
    }

    /// Build a fixture containing a project repo under `<home>/work/demo-repo`
    /// and a nested `packages/api` dir.
    fn fixture_codex_project_home() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let project_dir = home.join("work").join("demo-repo");
        let nested_dir = project_dir.join("packages").join("api");
        let codex = home.join(".codex");

        fs::create_dir_all(project_dir.join(".git")).unwrap();
        fs::create_dir_all(project_dir.join(".codex").join("skills").join("repo-skill")).unwrap();
        fs::create_dir_all(project_dir.join(".agents").join("skills").join("agents-skill")).unwrap();
        fs::create_dir_all(&nested_dir).unwrap();
        fs::create_dir_all(&codex).unwrap();

        let project_path_str = project_dir.to_string_lossy().to_string();
        let cfg = format!("\
[projects.\"{project_path_str}\"]\n\
trust_level = \"trusted\"\n\
");
        fs::write(codex.join("config.toml"), cfg).unwrap();

        fs::write(project_dir.join("AGENTS.md"),
            "# Repo Instructions\nUse npm test.\n").unwrap();
        fs::write(project_dir.join(".codex").join("config.toml"), "\
[profiles.repo]\n\
sandbox_mode = \"workspace-write\"\n\
\n\
[mcp_servers.repo_mcp]\n\
command = \"node\"\n\
args = [\"server.mjs\"]\n\
").unwrap();
        fs::write(project_dir.join(".codex").join("skills").join("repo-skill").join("SKILL.md"),
            "# Repo Skill\n\nUse inside this repo.\n").unwrap();
        fs::write(project_dir.join(".agents").join("skills").join("agents-skill").join("SKILL.md"),
            "# Agents Skill\n\nShared repo skill root.\n").unwrap();

        (dir, home, project_dir)
    }

    // ── Adapter metadata & capabilities ──

    #[test]
    fn adapter_metadata_matches_plan() {
        assert_eq!(CodexAdapter.id(), "codex");
        assert_eq!(CodexAdapter.display_name(), "Codex CLI");
        assert_eq!(CodexAdapter.short_name(), "Codex");
        assert_eq!(CodexAdapter.executable(), "codex");
    }

    #[test]
    fn capabilities_exactly_match_plan() {
        let c = CodexAdapter.capabilities();
        // Exactly these three are true; everything else false.
        assert!(c.mcp_security, "mcpSecurity must be true");
        assert!(c.sessions,     "sessions must be true");
        assert!(c.backup,       "backup must be true");
        assert!(!c.context_budget, "contextBudget must be false");
        assert!(!c.mcp_controls,   "mcpControls must be false");
        assert!(!c.mcp_policy,     "mcpPolicy must be false");
        assert!(!c.effective,      "effective must be false");
        assert!(!c.mcp_editable,   "mcpEditable must be false");
    }

    #[test]
    fn advertises_eleven_categories() {
        let cats = CodexAdapter.category_ids();
        assert_eq!(cats.len(), 11);
        for required in ["config", "memory", "skill", "mcp", "profile",
                         "rule", "plugin", "session", "history",
                         "shell", "runtime"] {
            assert!(cats.contains(&required), "missing category {required}");
        }
    }

    // ── Global scope scan (parity with CCO `createCodexHome`) ──

    #[test]
    fn discovers_global_scope() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let scopes = CodexAdapter.discover_scopes(&ctx).unwrap();
        assert!(scopes.iter().any(|s| s.id == "global"));
        assert_eq!(scopes[0].id, "global");
    }

    #[test]
    fn scans_all_global_categories_with_correct_counts() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let scopes = CodexAdapter.discover_scopes(&ctx).unwrap();
        let global = scopes.iter().find(|s| s.id == "global").unwrap();
        let mut counts: std::collections::HashMap<&str, usize> = Default::default();
        for cat in CodexAdapter.category_ids() {
            let items = CodexAdapter.scan_category(&ctx, cat, global).unwrap();
            counts.insert(cat, items.len());
        }
        // CCO golden counts (from test-codex-adapter.mjs).
        assert_eq!(counts["config"], 1);
        assert_eq!(counts["memory"], 1);
        assert_eq!(counts["skill"],  2);
        assert_eq!(counts["mcp"],    2);
        assert_eq!(counts["profile"], 1);
        assert_eq!(counts["rule"],   1);
        assert_eq!(counts["plugin"], 1);
    }

    #[test]
    fn config_emits_config_toml_item() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let global = CodexAdapter.discover_scopes(&ctx).unwrap()
            .into_iter().find(|s| s.id == "global").unwrap();
        let items = CodexAdapter.scan_category(&ctx, "config", &global).unwrap();
        assert!(items.iter().any(|i| i.name == "config.toml"));
    }

    #[test]
    fn memory_uses_frontmatter_name() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let global = CodexAdapter.discover_scopes(&ctx).unwrap()
            .into_iter().find(|s| s.id == "global").unwrap();
        let items = CodexAdapter.scan_category(&ctx, "memory", &global).unwrap();
        assert!(items.iter().any(|i| i.name == "Project Memory"));
    }

    #[test]
    fn skills_include_system_and_normal() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let global = CodexAdapter.discover_scopes(&ctx).unwrap()
            .into_iter().find(|s| s.id == "global").unwrap();
        let items = CodexAdapter.scan_category(&ctx, "skill", &global).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"demo-skill"),       "demo-skill missing in {names:?}");
        assert!(names.contains(&".system/system-skill"),
            ".system/system-skill missing in {names:?}");
    }

    #[test]
    fn mcp_servers_surface_command_and_url() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let global = CodexAdapter.discover_scopes(&ctx).unwrap()
            .into_iter().find(|s| s.id == "global").unwrap();
        let items = CodexAdapter.scan_category(&ctx, "mcp", &global).unwrap();
        let context7 = items.iter().find(|i| i.name == "context7").unwrap();
        assert_eq!(context7.mcp_config.as_ref().unwrap()["command"], "npx");
        let remote = items.iter().find(|i| i.name == "remote").unwrap();
        assert_eq!(remote.mcp_config.as_ref().unwrap()["url"], "https://example.com/mcp");
    }

    #[test]
    fn profiles_parsed_from_config_toml() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let global = CodexAdapter.discover_scopes(&ctx).unwrap()
            .into_iter().find(|s| s.id == "global").unwrap();
        let items = CodexAdapter.scan_category(&ctx, "profile", &global).unwrap();
        assert!(items.iter().any(|i| i.name == "review"));
    }

    #[test]
    fn rules_emit_md_files() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let global = CodexAdapter.discover_scopes(&ctx).unwrap()
            .into_iter().find(|s| s.id == "global").unwrap();
        let items = CodexAdapter.scan_category(&ctx, "rule", &global).unwrap();
        assert!(items.iter().any(|i| i.name == "default.rules"));
    }

    #[test]
    fn plugins_walk_to_manifest() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let global = CodexAdapter.discover_scopes(&ctx).unwrap()
            .into_iter().find(|s| s.id == "global").unwrap();
        let items = CodexAdapter.scan_category(&ctx, "plugin", &global).unwrap();
        assert!(items.iter().any(|i| i.name == "github"));
    }

    // ── Config description surfaces model/sandbox/approval/profiles ──

    #[test]
    fn config_description_surfaces_model_sandbox_approval() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let global = CodexAdapter.discover_scopes(&ctx).unwrap()
            .into_iter().find(|s| s.id == "global").unwrap();
        let items = CodexAdapter.scan_category(&ctx, "config", &global).unwrap();
        let cfg = items.iter().find(|i| i.name == "config.toml").unwrap();
        assert!(cfg.description.contains("model: gpt-5.5"), "got: {}", cfg.description);
        assert!(cfg.description.contains("sandbox: danger-full-access"));
        assert!(cfg.description.contains("approval: never"));
    }

    // ── Project scope (CCO `createCodexProjectHome` parity) ──

    #[test]
    fn project_scope_surfaces_trust_entry_and_files() {
        let (_dir, home, project_dir) = fixture_codex_project_home();
        let ctx = Ctx { home: &home, cwd: Some(&project_dir) };
        let scopes = CodexAdapter.discover_scopes(&ctx).unwrap();
        // The project scope id is base64url of the path; find it by root.
        let project = scopes.iter().find(|s| s.root == project_dir.to_string_lossy())
            .expect("project scope should be discovered");
        assert_eq!(project.kind, "project");
        assert_eq!(project.label, "demo-repo");

        let config_items = CodexAdapter.scan_category(&ctx, "config", project).unwrap();
        assert!(config_items.iter().any(|i| i.name == "AGENTS.md"));
        assert!(config_items.iter().any(|i| i.name == ".codex/config.toml"));

        let skill_items = CodexAdapter.scan_category(&ctx, "skill", project).unwrap();
        assert!(skill_items.iter().any(|i| i.name == "repo-skill"));
        assert!(skill_items.iter().any(|i| i.name == "agents-skill"));

        let mcp_items = CodexAdapter.scan_category(&ctx, "mcp", project).unwrap();
        assert!(mcp_items.iter().any(|i| i.name == "repo_mcp"));

        let profile_items = CodexAdapter.scan_category(&ctx, "profile", project).unwrap();
        assert!(profile_items.iter().any(|i| i.name == "repo"));
    }

    // ── Switching harness re-scans (parity with CCO capability tests) ──

    #[test]
    fn scan_result_capabilities_match_adapter() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let result = crate::harness::framework::run_scan(&CodexAdapter, &ctx).unwrap();
        assert_eq!(result.harness_id, "codex");
        assert_eq!(result.capabilities.mcp_security, true);
        assert_eq!(result.capabilities.sessions,     true);
        assert_eq!(result.capabilities.backup,       true);
        assert_eq!(result.capabilities.context_budget, false);
        assert_eq!(result.capabilities.mcp_controls, false);
        assert_eq!(result.capabilities.mcp_policy, false);
        assert_eq!(result.capabilities.effective,  false);
    }

    #[test]
    fn framework_categories_labeled_correctly() {
        let (_dir, home) = fixture_codex_home();
        let ctx = Ctx { home: &home, cwd: Some(&home) };
        let result = crate::harness::framework::run_scan(&CodexAdapter, &ctx).unwrap();
        let labels: std::collections::HashMap<String, String> = result.categories
            .iter().map(|c| (c.id.clone(), c.label.clone())).collect();
        assert_eq!(labels.get("config").unwrap(),  "Config");
        assert_eq!(labels.get("memory").unwrap(),  "Memories");
        assert_eq!(labels.get("skill").unwrap(),   "Skills");
        assert_eq!(labels.get("mcp").unwrap(),     "MCP");
        assert_eq!(labels.get("profile").unwrap(), "Profiles");
        assert_eq!(labels.get("rule").unwrap(),    "Rules");
        assert_eq!(labels.get("plugin").unwrap(),  "Plugins");
        assert_eq!(labels.get("session").unwrap(), "Sessions");
        assert_eq!(labels.get("history").unwrap(), "History");
        assert_eq!(labels.get("shell").unwrap(),   "Shell");
        assert_eq!(labels.get("runtime").unwrap(), "Runtime");
    }

    // ── extract_session_id utility ──

    #[test]
    fn extract_session_id_parses_rollout_uuid() {
        assert_eq!(
            extract_session_id("rollout-2026-07-04T12-30-00-abc12345-1234-5678-9abc-def012345678.jsonl"),
            "abc12345-1234-5678-9abc-def012345678"
        );
    }

    #[test]
    fn extract_session_id_falls_back_to_stem() {
        assert_eq!(extract_session_id("foo.jsonl"), "foo");
    }
}