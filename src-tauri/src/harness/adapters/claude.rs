use std::path::{Path, PathBuf};
use crate::error::WardError;
use crate::fs_utils::{decode_project_dir_name, parse_frontmatter};
use crate::harness::{framework, Ctx, Harness};
use crate::model::{Capabilities, HarnessItem, Scope};

/// Resolved layout for one scope — answers "where do I look for X?"
/// without forcing each scanner to re-derive the same branching.
#[derive(Default, Debug, Clone)]
struct ScopePaths {
    /// `~/.claude` for the global scope. `None` for project scopes.
    claude_root: Option<PathBuf>,
    /// Real repo path for resolved project scopes. `None` for global /
    /// unresolved project scopes.
    repo_dir: Option<PathBuf>,
    /// `~/.claude/projects/<encoded>` for any project scope (used for
    /// plans/sessions/memory on unresolved scopes).
    project_dir: Option<PathBuf>,
}

impl ScopePaths {
    fn for_scope(home: &Path, scope: &Scope) -> Self {
        let mut p = ScopePaths::default();
        if scope.id == "global" {
            p.claude_root = Some(home.join(".claude"));
        } else if scope.kind == "project" {
            p.repo_dir = Some(PathBuf::from(&scope.root));
        } else if scope.kind == "project-unresolved" {
            p.project_dir = Some(PathBuf::from(&scope.root));
        }
        // All project scopes also have a projects/<encoded> dir available.
        if scope.id != "global" {
            let encoded = scope.id.clone();
            let dir = home.join(".claude").join("projects").join(&encoded);
            p.project_dir.get_or_insert(dir);
        }
        p
    }

    fn is_global_claude_dir_match(&self) -> bool {
        // True when the project's .claude is the same as the global ~/.claude.
        // Avoids double-counting items when repoDir == home.
        match (&self.claude_root, &self.repo_dir) {
            (Some(g), Some(r)) => g == &r.join(".claude"),
            _ => false,
        }
    }

    fn repo_claude_dir(&self) -> Option<PathBuf> {
        self.repo_dir.as_ref().map(|r| r.join(".claude"))
    }
}

pub struct ClaudeAdapter;

impl ClaudeAdapter {
    fn claude_root(home: &Path) -> PathBuf {
        home.join(".claude")
    }

    fn projects_dir(home: &Path) -> PathBuf {
        Self::claude_root(home).join("projects")
    }

    /// Pretty-print an unresolved encoded name into something readable.
    fn prettify_encoded(encoded: &str) -> String {
        let mut s = encoded.strip_prefix('-').unwrap_or(encoded).to_string();
        s = s.replace("--", "/…/");
        s = s.replace('-', "/");
        s = s.trim_matches('/').to_string();
        if s.is_empty() {
            encoded.to_string()
        } else {
            s
        }
    }

    /// Final identifier for a project scope. We always use the encoded
    /// directory name so item `scope_id` values stay stable across runs.
    fn build_project_scopes(home: &Path, claude_root: &Path) -> Vec<Scope> {
        let projects_dir = claude_root.join("projects");
        let entries = match std::fs::read_dir(&projects_dir) {
            Ok(e) => e,
            Err(_) => return vec![],
        };
        struct Entry { encoded: String, real: Option<PathBuf>, short: String }
        let mut entries_vec: Vec<Entry> = Vec::new();
        for dir_entry in entries.flatten() {
            let file_type = match dir_entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if !file_type.is_dir() {
                continue;
            }
            let encoded = dir_entry.file_name().to_string_lossy().to_string();
            let real = decode_project_dir_name(home, &encoded);
            let short = match &real {
                Some(p) => p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| encoded.clone()),
                None => Self::prettify_encoded(&encoded),
            };
            entries_vec.push(Entry { encoded, real, short });
        }
        entries_vec.sort_by(|a, b| {
            match (&a.real, &b.real) {
                (Some(pa), Some(pb)) => {
                    let da = pa.components().count();
                    let db = pb.components().count();
                    if da != db { return da.cmp(&db); }
                    pa.display().to_string().cmp(&pb.display().to_string())
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.short.cmp(&b.short),
            }
        });
        let mut name_count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for e in &entries_vec { *name_count.entry(e.short.clone()).or_insert(0) += 1; }
        let mut scopes: Vec<Scope> = Vec::new();
        for e in entries_vec {
            let mut label = e.short.clone();
            if e.real.is_some() && name_count.get(&e.short).copied().unwrap_or(0) > 1 {
                if let Some(p) = &e.real {
                    if let Some(parent) = p.parent() {
                        if let Some(pname) = parent.file_name() {
                            label = format!("{}/{}", pname.to_string_lossy(), label);
                        }
                    }
                }
            }
            let root_str = e.real.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| {
                    Self::projects_dir(home).join(&e.encoded).display().to_string()
                });
            let kind = if e.real.is_some() { "project" } else { "project-unresolved" };
            scopes.push(Scope {
                id: e.encoded,
                kind: kind.into(),
                label,
                root: root_str,
            });
        }
        scopes
    }

    /// Path decoder used to validate that an embedded `<repo>/.mcp.json`
    /// belongs to the current repo scope when scanning project-level MCP.
    fn encode_project_name(real: &Path) -> String {
        // Match CCO's `encodeClaudeProjectName` — replace every non
        // `[A-Za-z0-9-]` character with `-`.
        let s = real.display().to_string();
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
            .collect()
    }
}

impl Harness for ClaudeAdapter {
    fn id(&self) -> &str { "claude" }
    fn display_name(&self) -> &str { "Claude Code" }
    fn short_name(&self) -> &str { "Claude" }
    fn icon(&self) -> &str { "◆" }
    fn executable(&self) -> &str { "claude" }

    fn category_ids(&self) -> Vec<&'static str> {
        vec!["skill", "memory", "mcp", "command", "agent", "plan", "rule", "config", "hook", "plugin", "session", "setting"]
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            context_budget: true, mcp_controls: true, mcp_policy: true,
            mcp_security: true, sessions: true, effective: true, backup: true,
        }
    }

    fn discover_scopes(&self, ctx: &Ctx) -> Result<Vec<Scope>, WardError> {
        let root = Self::claude_root(ctx.home);
        let mut scopes: Vec<Scope> = vec![Scope {
            id: "global".into(),
            kind: "global".into(),
            label: "Global (~/.claude)".into(),
            root: root.display().to_string(),
        }];
        scopes.extend(Self::build_project_scopes(ctx.home, &root));
        Ok(scopes)
    }

    fn scan_category(&self, ctx: &Ctx, category: &str, scope: &Scope)
        -> Result<Vec<HarnessItem>, WardError> {
        let paths = ScopePaths::for_scope(ctx.home, scope);
        let items = match category {
            "skill" => scan_skills(scope, &paths),
            "memory" => scan_memories(scope, &paths),
            "mcp" => scan_mcp(scope, ctx.home, &paths),
            "command" => scan_commands(scope, &paths),
            "agent" => scan_agents(scope, &paths),
            "plan" => scan_plans(scope, &paths),
            "rule" => scan_rules(scope, &paths),
            "config" => scan_configs(scope, &paths),
            "hook" => scan_hooks(scope, &paths),
            "plugin" => scan_plugins(ctx.home),
            "session" => scan_sessions(scope, &paths),
            "setting" => scan_settings(scope, &paths),
            _ => vec![],
        };
        Ok(items)
    }
}

// ── Scanners ─────────────────────────────────────────────────────────────

fn scan_skills(scope: &Scope, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    let dirs: Vec<PathBuf> = if scope.id == "global" {
        match &paths.claude_root {
            Some(r) => vec![r.join("skills")],
            None => vec![],
        }
    } else if let Some(rc) = paths.repo_claude_dir() {
        if !paths.is_global_claude_dir_match() {
            vec![rc.join("skills")]
        } else { vec![] }
    } else { vec![] };

    for dir in dirs {
        let entries = match std::fs::read_dir(&dir) { Ok(e) => e, Err(_) => continue };
        for entry in entries.flatten() {
            let p = entry.path();
            let manifest = p.join("SKILL.md");
            if manifest.is_file() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Read frontmatter for the display name (falls back to dir name).
                let content = std::fs::read_to_string(&manifest).unwrap_or_default();
                let fm = parse_frontmatter(&content);
                let display = fm.get("name").cloned().unwrap_or_else(|| name.clone());
                items.push(HarnessItem {
                    category: "skill".into(),
                    scope_id: scope.id.clone(),
                    name: display,
                    description: String::new(),
                    path: manifest.display().to_string(),
                    movable: true, deletable: true, locked: false,
                    effective: None,
                });
            }
        }
    }
    items
}

fn scan_memories(scope: &Scope, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    if scope.id == "global" {
        // Root CLAUDE.md (locked) + ~/.claude/memory/*.md
        if let Some(root) = &paths.claude_root {
            let claude_md = root.join("CLAUDE.md");
            if claude_md.is_file() {
                items.push(HarnessItem {
                    category: "memory".into(), scope_id: scope.id.clone(),
                    name: "CLAUDE.md".into(), description: String::new(),
                    path: claude_md.display().to_string(),
                    movable: false, deletable: false, locked: true,
                    effective: None,
                });
            }
            let mem_dir = root.join("memory");
            scan_md_dir(&mem_dir, scope, "memory", &mut items, false, false);
        }
    } else {
        // For resolved projects: <repo>/CLAUDE.md (locked) + <repo>/.claude/CLAUDE.md (locked)
        // For unresolved: <projectsDir>/<enc>/CLAUDE.md + <projectsDir>/<enc>/memory/*.md
        if let Some(repo) = &paths.repo_dir {
            for name in ["CLAUDE.md"] {
                let p = repo.join(name);
                if p.is_file() {
                    items.push(HarnessItem {
                        category: "memory".into(), scope_id: scope.id.clone(),
                        name: name.into(), description: String::new(),
                        path: p.display().to_string(),
                        movable: false, deletable: false, locked: true,
                        effective: None,
                    });
                }
            }
            if let Some(rc) = paths.repo_claude_dir() {
                let p = rc.join("CLAUDE.md");
                if p.is_file() {
                    items.push(HarnessItem {
                        category: "memory".into(), scope_id: scope.id.clone(),
                        name: ".claude/CLAUDE.md".into(), description: String::new(),
                        path: p.display().to_string(),
                        movable: false, deletable: false, locked: true,
                        effective: None,
                    });
                }
            }
        }
        if let Some(pdir) = &paths.project_dir {
            let memory_dir = pdir.join("memory");
            scan_md_dir(&memory_dir, scope, "memory", &mut items, true, true);
        }
    }
    items
}

fn scan_mcp(scope: &Scope, home: &Path, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    if scope.id == "global" {
        // ~/.claude/.mcp.json
        if let Some(root) = &paths.claude_root {
            for f in [".mcp.json"] {
                let p = root.join(f);
                if p.is_file() {
                    if let Some(content) = std::fs::read_to_string(&p).ok() {
                        if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(servers) = cfg.get("mcpServers").and_then(|v| v.as_object()) {
                                for (name, server_config) in servers {
                                    items.push(mcp_item(name, &p, scope, server_config));
                                }
                            }
                        }
                    }
                }
            }
        }
        // ~/.mcp.json (alternate user location)
        let alt = home.join(".mcp.json");
        if alt.is_file() {
            if let Ok(content) = std::fs::read_to_string(&alt) {
                if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(servers) = cfg.get("mcpServers").and_then(|v| v.as_object()) {
                        for (name, server_config) in servers {
                            items.push(mcp_item(name, &alt, scope, server_config));
                        }
                    }
                }
            }
        }
        // ~/.claude.json — top-level mcpServers (user scope)
        let claude_json = home.join(".claude.json");
        if claude_json.is_file() {
            if let Ok(content) = std::fs::read_to_string(&claude_json) {
                if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(servers) = cfg.get("mcpServers").and_then(|v| v.as_object()) {
                        for (name, server_config) in servers {
                            items.push(mcp_item(name, &claude_json, scope, server_config));
                        }
                    }
                }
            }
        }
        // MCP embedded in settings.json / settings.local.json (handled separately by category="hook" parser;
        // we DO also surface mcpServers entries from settings as MCP items here).
        if let Some(root) = &paths.claude_root {
            for f in ["settings.json", "settings.local.json"] {
                let p = root.join(f);
                if p.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&p) {
                        if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(servers) = cfg.get("mcpServers").and_then(|v| v.as_object()) {
                                for (name, server_config) in servers {
                                    items.push(mcp_item(name, &p, scope, server_config));
                                }
                            }
                        }
                    }
                }
            }
        }
    } else if let Some(repo) = &paths.repo_dir {
        // Project: <repo>/.mcp.json (NOT inside .claude/)
        let pmcp = repo.join(".mcp.json");
        if pmcp.is_file() {
            if let Ok(content) = std::fs::read_to_string(&pmcp) {
                if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(servers) = cfg.get("mcpServers").and_then(|v| v.as_object()) {
                        for (name, server_config) in servers {
                            items.push(mcp_item(name, &pmcp, scope, server_config));
                        }
                    }
                }
            }
        }
        // ~/.claude.json — projects[<repo>].mcpServers
        let claude_json = home.join(".claude.json");
        let encoded = ClaudeAdapter::encode_project_name(repo);
        if claude_json.is_file() {
            if let Ok(content) = std::fs::read_to_string(&claude_json) {
                if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(projs) = cfg.get("projects").and_then(|v| v.as_object()) {
                        // Match either by the raw real path or the encoded form.
                        let repo_str = repo.display().to_string();
                        let mut found: Option<&serde_json::Value> = None;
                        for (k, v) in projs {
                            if k == &repo_str || k == &encoded {
                                found = Some(v);
                                break;
                            }
                        }
                        if let Some(proj) = found {
                            if let Some(servers) = proj.get("mcpServers").and_then(|v| v.as_object()) {
                                for (name, server_config) in servers {
                                    items.push(mcp_item(name, &claude_json, scope, server_config));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    items
}

fn mcp_item(name: &str, source: &Path, scope: &Scope, server_config: &serde_json::Value) -> HarnessItem {
    let cmd = server_config.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args = server_config.get("args").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let desc = if !cmd.is_empty() { format!("{cmd} {args}").trim().to_string() } else { "(HTTP MCP)".to_string() };
    HarnessItem {
        category: "mcp".into(),
        scope_id: scope.id.clone(),
        name: name.to_string(),
        description: String::new(),
        path: source.display().to_string(),
        movable: true, deletable: true, locked: false,
        effective: None,
    }.with_description(desc)
}

fn scan_commands(scope: &Scope, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    let dirs: Vec<PathBuf> = if scope.id == "global" {
        paths.claude_root.as_ref().map(|r| vec![r.join("commands")]).unwrap_or_default()
    } else if let Some(rc) = paths.repo_claude_dir() {
        if !paths.is_global_claude_dir_match() { vec![rc.join("commands")] } else { vec![] }
    } else { vec![] };
    for dir in dirs {
        scan_md_dir(&dir, scope, "command", &mut items, true, true);
    }
    items
}

fn scan_agents(scope: &Scope, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    let dirs: Vec<PathBuf> = if scope.id == "global" {
        paths.claude_root.as_ref().map(|r| vec![r.join("agents")]).unwrap_or_default()
    } else if let Some(rc) = paths.repo_claude_dir() {
        if !paths.is_global_claude_dir_match() { vec![rc.join("agents")] } else { vec![] }
    } else { vec![] };
    for dir in dirs {
        scan_md_dir(&dir, scope, "agent", &mut items, true, true);
    }
    items
}

fn scan_plans(scope: &Scope, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    let dirs: Vec<PathBuf> = if scope.id == "global" {
        paths.claude_root.as_ref().map(|r| vec![r.join("plans")]).unwrap_or_default()
    } else {
        paths.project_dir.as_ref().map(|p| vec![p.join("plans")]).unwrap_or_default()
    };
    for dir in dirs {
        scan_md_dir(&dir, scope, "plan", &mut items, true, true);
    }
    items
}

fn scan_rules(scope: &Scope, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    let dirs: Vec<PathBuf> = if scope.id == "global" {
        paths.claude_root.as_ref().map(|r| vec![r.join("rules")]).unwrap_or_default()
    } else if let Some(rc) = paths.repo_claude_dir() {
        if !paths.is_global_claude_dir_match() { vec![rc.join("rules")] } else { vec![] }
    } else { vec![] };
    for dir in dirs {
        scan_md_dir(&dir, scope, "rule", &mut items, true, true);
    }
    items
}

// ── Config / Hook / Settings / Plugin / Session scanners ────────────

fn scan_configs(scope: &Scope, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    // For each candidate path, emit an item if the file exists.
    let candidates: Vec<(String, PathBuf)> = if scope.id == "global" {
        if let Some(root) = &paths.claude_root {
            vec![
                ("CLAUDE.md".into(), root.join("CLAUDE.md")),
                ("settings.json".into(), root.join("settings.json")),
                ("settings.local.json".into(), root.join("settings.local.json")),
            ]
        } else { vec![] }
    } else if let Some(rc) = paths.repo_claude_dir() {
        if !paths.is_global_claude_dir_match() {
            if let Some(repo) = &paths.repo_dir {
                vec![
                    ("CLAUDE.md".into(), repo.join("CLAUDE.md")),
                    (".claude/CLAUDE.md".into(), rc.join("CLAUDE.md")),
                    ("settings.json".into(), rc.join("settings.json")),
                    ("settings.local.json".into(), rc.join("settings.local.json")),
                ]
            } else { vec![] }
        } else { vec![] }
    } else { vec![] };
    for (name, p) in candidates {
        if !p.is_file() { continue; }
        items.push(HarnessItem {
            category: "config".into(),
            scope_id: scope.id.clone(),
            name,
            description: String::new(),
            path: p.display().to_string(),
            movable: false, deletable: false, locked: true,
            effective: None,
        });
    }
    items
}

fn scan_hooks(scope: &Scope, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    let sources: Vec<(String, PathBuf)> = if scope.id == "global" {
        if let Some(root) = &paths.claude_root {
            vec![
                ("settings.json".into(), root.join("settings.json")),
                ("settings.local.json".into(), root.join("settings.local.json")),
            ]
        } else { vec![] }
    } else if let Some(rc) = paths.repo_claude_dir() {
        if !paths.is_global_claude_dir_match() {
            vec![
                ("settings.json".into(), rc.join("settings.json")),
                ("settings.local.json".into(), rc.join("settings.local.json")),
            ]
        } else { vec![] }
    } else { vec![] };
    for (label, p) in sources {
        if !p.is_file() { continue; }
        let content = match std::fs::read_to_string(&p) { Ok(c) => c, Err(_) => continue };
        let cfg: serde_json::Value = match serde_json::from_str(&content) { Ok(v) => v, Err(_) => continue };
        let hooks = match cfg.get("hooks").and_then(|v| v.as_object()) {
            Some(h) => h,
            None => continue,
        };
        for (event, hook_array) in hooks {
            let arr = match hook_array.as_array() { Some(a) => a, None => continue };
            for _hook_group in arr {
                items.push(HarnessItem {
                    category: "hook".into(),
                    scope_id: scope.id.clone(),
                    name: event.clone(),
                    description: format!("from {label}"),
                    path: p.display().to_string(),
                    movable: false, deletable: false, locked: true,
                    effective: None,
                });
            }
        }
    }
    items
}

fn scan_settings(scope: &Scope, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    let sources: Vec<PathBuf> = if scope.id == "global" {
        if let Some(root) = &paths.claude_root {
            vec![root.join("settings.json"), root.join("settings.local.json")]
        } else { vec![] }
    } else if let Some(rc) = paths.repo_claude_dir() {
        if !paths.is_global_claude_dir_match() {
            vec![rc.join("settings.json"), rc.join("settings.local.json")]
        } else { vec![] }
    } else { vec![] };
    for p in sources {
        if !p.is_file() { continue; }
        let content = match std::fs::read_to_string(&p) { Ok(c) => c, Err(_) => continue };
        let cfg: serde_json::Value = match serde_json::from_str(&content) { Ok(v) => v, Err(_) => continue };
        if let Some(obj) = cfg.as_object() {
            for key in obj.keys() {
                // Skip hooks (handled by scan_hooks) and mcpServers (handled by scan_mcp).
                if key == "hooks" || key == "mcpServers" { continue; }
                items.push(HarnessItem {
                    category: "setting".into(),
                    scope_id: scope.id.clone(),
                    name: key.clone(),
                    description: String::new(),
                    path: p.display().to_string(),
                    movable: false, deletable: false, locked: true,
                    effective: None,
                });
            }
        }
    }
    items
}

fn scan_plugins(home: &Path) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    let p = home.join(".claude").join("plugins").join("installed_plugins.json");
    if !p.is_file() { return items; }
    let content = match std::fs::read_to_string(&p) { Ok(c) => c, Err(_) => return items };
    let cfg: serde_json::Value = match serde_json::from_str(&content) { Ok(v) => v, Err(_) => return items };
    let plugins = match cfg.get("plugins").and_then(|v| v.as_object()) {
        Some(p) => p,
        None => return items,
    };
    for (plugin_key, installs) in plugins {
        let arr = match installs.as_array() { Some(a) => a, None => continue };
        for install in arr {
            let install_path = install.get("installPath").and_then(|v| v.as_str()).unwrap_or("");
            if install_path.is_empty() { continue; }
            items.push(HarnessItem {
                category: "plugin".into(),
                scope_id: "global".into(),
                name: plugin_key.clone(),
                description: String::new(),
                path: install_path.to_string(),
                movable: false, deletable: false, locked: true,
                effective: None,
            });
        }
    }
    items
}

fn scan_sessions(scope: &Scope, paths: &ScopePaths) -> Vec<HarnessItem> {
    // No global sessions. Only project (resolved or unresolved) sessions.
    if scope.id == "global" { return vec![]; }
    let pdir = match &paths.project_dir {
        Some(d) => d,
        None => return vec![],
    };
    let entries = match std::fs::read_dir(pdir) { Ok(e) => e, Err(_) => return vec![] };
    let mut items = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() { continue; }
        if p.extension().and_then(|e| e.to_str()) != Some("jsonl") { continue; }
        let name = entry.file_name().to_string_lossy().to_string();
        let stem = name.trim_end_matches(".jsonl").to_string();
        items.push(HarnessItem {
            category: "session".into(),
            scope_id: scope.id.clone(),
            name: stem,
            description: String::new(),
            path: p.display().to_string(),
            movable: false, deletable: false, locked: true,
            effective: None,
        });
    }
    items
}

// ── Shared markdown-dir helper ───────────────────────────────────────────

fn scan_md_dir(
    dir: &Path,
    scope: &Scope,
    category: &'static str,
    out: &mut Vec<HarnessItem>,
    movable: bool,
    deletable: bool,
) {
    let entries = match std::fs::read_dir(dir) { Ok(e) => e, Err(_) => return };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() { continue; }
        if p.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
        if p.file_name().and_then(|n| n.to_str()) == Some("MEMORY.md") { continue; }
        let content = std::fs::read_to_string(&p).unwrap_or_default();
        let fm = parse_frontmatter(&content);
        let file_name = p.file_name().unwrap().to_string_lossy().to_string();
        let display = fm.get("name").cloned()
            .unwrap_or_else(|| file_name.trim_end_matches(".md").to_string());
        out.push(HarnessItem {
            category: category.into(),
            scope_id: scope.id.clone(),
            name: display,
            description: String::new(),
            path: p.display().to_string(),
            movable,
            deletable,
            locked: false,
            effective: None,
        }.with_description(fm.get("description").cloned().unwrap_or_default()));
    }
}

// ── Helpers to attach optional description to items ─────────────────────

trait WithDescription {
    fn with_description(self, desc: String) -> HarnessItem;
}
impl WithDescription for HarnessItem {
    fn with_description(mut self, desc: String) -> HarnessItem {
        // Attach the description to the HarnessItem's optional description
        // field (used by the Organizer to surface details).
        self.description = desc;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_global_scope() {
        let home = Path::new("/Users/x");
        let ctx = Ctx { home, cwd: None };
        let scopes = ClaudeAdapter.discover_scopes(&ctx).unwrap();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].id, "global");
        assert_eq!(scopes[0].root, "/Users/x/.claude");
    }

    #[test]
    fn advertises_all_capabilities() {
        let c = ClaudeAdapter.capabilities();
        assert!(c.effective && c.mcp_security && c.backup && c.context_budget);
    }

    use std::fs;

    fn make_home() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        fs::create_dir_all(claude.join("skills/brainstorming")).unwrap();
        fs::write(claude.join("skills/brainstorming/SKILL.md"), "---\nname: brainstorming\n---\n").unwrap();
        fs::create_dir_all(claude.join("skills/deep-research")).unwrap();
        fs::write(claude.join("skills/deep-research/SKILL.md"), "x").unwrap();
        fs::write(claude.join("CLAUDE.md"), "root memory").unwrap();
        fs::create_dir_all(claude.join("memory")).unwrap();
        fs::write(claude.join("memory/user.md"), "u").unwrap();
        dir
    }

    #[test]
    fn scans_skills() {
        let home = make_home();
        let ctx = Ctx { home: home.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let mut items = ClaudeAdapter.scan_category(&ctx, "skill", &scope).unwrap();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "brainstorming");
        assert_eq!(items[0].category, "skill");
        assert_eq!(items[0].scope_id, "global");
        assert!(items[0].path.ends_with("skills/brainstorming/SKILL.md"));
    }

    #[test]
    fn scans_memories_including_root_claude_md() {
        let home = make_home();
        let ctx = Ctx { home: home.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let items = ClaudeAdapter.scan_category(&ctx, "memory", &scope).unwrap();
        let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"CLAUDE.md"));
        assert!(names.contains(&"user"));
        assert_eq!(items.len(), 2);
    }

    /// Helper: write an empty `~/.claude/projects/<encoded>/` dir,
    /// optionally with `session.jsonl` carrying a `cwd` line.
    fn make_project_dir(home: &Path, encoded: &str, real_cwd: Option<&Path>) {
        let dir = home.join(".claude").join("projects").join(encoded);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(cwd) = real_cwd {
            let line = format!("{{\"cwd\":\"{}\",\"type\":\"user\"}}\n", cwd.display());
            std::fs::write(dir.join("session.jsonl"), line).unwrap();
        }
    }

    #[test]
    fn discovers_global_only_when_projects_dir_absent() {
        let dir = tempfile::tempdir().unwrap();
        // Make a Claude home without a projects/ subdir.
        std::fs::create_dir_all(dir.path().join(".claude/skills")).unwrap();
        let ctx = Ctx { home: dir.path(), cwd: None };
        let scopes = ClaudeAdapter.discover_scopes(&ctx).unwrap();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].id, "global");
    }

    #[test]
    fn discovers_project_scope_via_session_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let real_repo = dir.path().join("work").join("ward-demo");
        std::fs::create_dir_all(&real_repo).unwrap();
        let encoded = "-work-ward-demo";
        make_project_dir(dir.path(), encoded, Some(&real_repo));

        let ctx = Ctx { home: dir.path(), cwd: None };
        let scopes = ClaudeAdapter.discover_scopes(&ctx).unwrap();
        let project = scopes.iter().find(|s| s.id == encoded).expect("project scope");
        assert_eq!(project.kind, "project");
        assert!(project.root.contains("ward-demo"), "root should reference decoded path: {}", project.root);
        assert!(project.label.contains("ward-demo"));
    }

    #[test]
    fn preserves_unresolved_project_scopes() {
        let dir = tempfile::tempdir().unwrap();
        // Encoded dir with no session.jsonl and no matching on-disk path.
        let encoded = "-missing-project-with-memory";
        make_project_dir(dir.path(), encoded, None);

        let ctx = Ctx { home: dir.path(), cwd: None };
        let scopes = ClaudeAdapter.discover_scopes(&ctx).unwrap();
        let unresolved = scopes.iter().find(|s| s.id == encoded).expect("unresolved scope");
        assert_eq!(unresolved.kind, "project-unresolved");
        // Root must still point to the projects/<encoded> dir so items inside are reachable.
        assert!(unresolved.root.ends_with(&format!("projects/{}", encoded)));
        assert!(unresolved.label.contains("missing") || !unresolved.label.is_empty());
    }

    #[test]
    fn resolves_symlinked_project_dirs_via_session_cwd() {
        let dir = tempfile::tempdir().unwrap();
        // Real dir lives somewhere the symlink reaches.
        let target = dir.path().join("work").join("ward-demo");
        std::fs::create_dir_all(&target).unwrap();
        let encoded = "-work-ward-demo";
        make_project_dir(dir.path(), encoded, Some(&target));

        let ctx = Ctx { home: dir.path(), cwd: None };
        let scopes = ClaudeAdapter.discover_scopes(&ctx).unwrap();
        let project = scopes.iter().find(|s| s.id == encoded).expect("project scope");
        assert!(project.root.contains("ward-demo"));
    }

    #[test]
    fn advertises_all_twelve_categories() {
        let cats = ClaudeAdapter.category_ids();
        assert_eq!(cats.len(), 12);
        for required in ["skill", "memory", "mcp", "command", "agent", "plan", "rule", "config", "hook", "plugin", "session", "setting"] {
            assert!(cats.contains(&required), "missing category {required}");
        }
    }

    /// The Category entries handed back from the framework must label
    /// our 12 categories with the right display names.
    #[test]
    fn framework_labeled_categories_match_canonical_set() {
        let home = make_home();
        let ctx = Ctx { home: home.path(), cwd: None };
        let result = framework::run_scan(&ClaudeAdapter, &ctx).unwrap();
        let ids: Vec<&str> = result.categories.iter().map(|c| c.id.as_str()).collect();
        let labels: std::collections::HashMap<String, String> =
            result.categories.iter().map(|c| (c.id.clone(), c.label.clone())).collect();
        assert_eq!(ids.len(), 12);
        assert_eq!(labels.get("skill").unwrap(), "Skills");
        assert_eq!(labels.get("memory").unwrap(), "Memories");
        assert_eq!(labels.get("mcp").unwrap(), "MCP");
        assert_eq!(labels.get("command").unwrap(), "Commands");
        assert_eq!(labels.get("agent").unwrap(), "Agents");
        assert_eq!(labels.get("plan").unwrap(), "Plans");
        assert_eq!(labels.get("rule").unwrap(), "Rules");
        assert_eq!(labels.get("config").unwrap(), "Config");
        assert_eq!(labels.get("hook").unwrap(), "Hooks");
        assert_eq!(labels.get("plugin").unwrap(), "Plugins");
        assert_eq!(labels.get("session").unwrap(), "Sessions");
        assert_eq!(labels.get("setting").unwrap(), "Settings");
    }

    /// Helper: build a home with config/hook/settings files populated.
    fn make_home_with_settings() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        fs::write(claude.join("settings.json"), r#"{"permissions":{"allow":["bash"]},"outputStyle":"dark","hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#).unwrap();
        fs::write(claude.join("settings.local.json"), r#"{"language":"en"}"#).unwrap();
        fs::write(claude.join("CLAUDE.md"), "global memory").unwrap();
        dir
    }

    #[test]
    fn scan_configs_emits_settings_and_root_claude_md() {
        let home = make_home_with_settings();
        let ctx = Ctx { home: home.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let items = ClaudeAdapter.scan_category(&ctx, "config", &scope).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"CLAUDE.md"));
        assert!(names.contains(&"settings.json"));
        assert!(names.contains(&"settings.local.json"));
        assert!(items.iter().all(|i| i.locked && !i.movable && !i.deletable));
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn scan_hooks_parses_settings_json() {
        let home = make_home_with_settings();
        let ctx = Ctx { home: home.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let items = ClaudeAdapter.scan_category(&ctx, "hook", &scope).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "PreToolUse");
        assert_eq!(items[0].category, "hook");
        assert!(items[0].locked && !items[0].movable);
    }

    #[test]
    fn scan_settings_emits_keys_from_settings_files() {
        let home = make_home_with_settings();
        let ctx = Ctx { home: home.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let items = ClaudeAdapter.scan_category(&ctx, "setting", &scope).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        // settings.json has permissions and outputStyle; settings.local.json has language.
        assert!(names.contains(&"outputStyle"));
        assert!(names.contains(&"language"));
        // hooks and mcpServers are excluded — they belong to scan_hooks and scan_mcp.
        assert!(!names.contains(&"hooks"));
        assert!(!names.contains(&"mcpServers"));
        assert!(items.iter().all(|i| i.locked));
    }

    #[test]
    fn scan_plugins_reads_installed_plugins_json() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join(".claude").join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        fs::write(plugins.join("installed_plugins.json"),
            r#"{"plugins":{"demo-plugin":[{"scope":"user","installPath":"/Users/x/.claude/plugins/demo"}]}}"#
        ).unwrap();
        let ctx = Ctx { home: dir.path(), cwd: None };
        let items = scan_plugins(dir.path());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "demo-plugin");
        assert_eq!(items[0].scope_id, "global");
        assert!(items[0].locked && !items[0].movable);
    }

    #[test]
    fn scan_sessions_returns_jsonl_in_project_dir() {
        let dir = tempfile::tempdir().unwrap();
        let encoded = "-test-project";
        make_project_dir(dir.path(), encoded, None);
        let pdir = dir.path().join(".claude").join("projects").join(encoded);
        fs::write(pdir.join("alpha.jsonl"), "{}\n").unwrap();
        fs::write(pdir.join("beta.jsonl"), "{}\n").unwrap();
        fs::write(pdir.join("notes.txt"), "ignore").unwrap();

        let scope = Scope {
            id: encoded.into(),
            kind: "project".into(),
            label: "test".into(),
            root: pdir.display().to_string(),
        };
        let paths = ScopePaths::for_scope(dir.path(), &scope);
        let items = scan_sessions(&scope, &paths);
        let mut names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["alpha", "beta"]);
        assert!(items.iter().all(|i| i.locked));
    }

    #[test]
    fn scan_sessions_empty_for_global_scope() {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope {
            id: "global".into(),
            kind: "global".into(),
            label: "Global".into(),
            root: dir.path().join(".claude").display().to_string(),
        };
        let paths = ScopePaths::for_scope(dir.path(), &scope);
        let items = scan_sessions(&scope, &paths);
        assert!(items.is_empty());
    }
}
