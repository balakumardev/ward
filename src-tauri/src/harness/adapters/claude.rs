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
            mcp_editable: true,
            skill_creatable: true,
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
            "memory" => scan_memories(scope, ctx.home, &paths),
            "mcp" => scan_mcp(scope, ctx.home, &paths),
            "command" => scan_commands(scope, &paths),
            "agent" => scan_agents(scope, &paths),
            "plan" => scan_plans(scope, &paths),
            "rule" => scan_rules(scope, &paths),
            "config" => scan_configs(scope, &paths),
            "hook" => scan_hooks(scope, &paths),
            // Plugins are user-global (installed_plugins.json lives under
            // ~/.claude/plugins). The scan loop calls scan_category once per
            // scope, so only emit for the global scope — otherwise every
            // plugin is duplicated once per project scope.
            "plugin" => if scope.id == "global" { scan_plugins(ctx.home) } else { vec![] },
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
                // Read frontmatter for the display name (falls back to dir
                // name) and the description (used by the Organizer detail
                // pane AND by the context-budget skill listing).
                let content = std::fs::read_to_string(&manifest).unwrap_or_default();
                let fm = parse_frontmatter(&content);
                let display = fm.get("name").cloned().unwrap_or_else(|| name.clone());
                let description = fm.get("description").cloned().unwrap_or_default();
                items.push(HarnessItem {
                    category: "skill".into(),
                    scope_id: scope.id.clone(),
                    name: display,
                    description,
                    path: manifest.display().to_string(),
                    movable: true, deletable: true, locked: false,
                    effective: None,
            mcp_config: None,
                });
            }
        }
    }
    items
}

fn scan_memories(scope: &Scope, home: &Path, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    // The auto-memory directory can be relocated via the `autoMemoryDirectory`
    // setting (any settings scope, absolute or `~/`-prefixed). Default global
    // auto-memory lives per-project under ~/.claude/projects/<enc>/memory/, but
    // when the override points at a fixed dir we surface it on the global scope.
    let override_dir = home.join(".claude").is_dir()
        .then(|| memory_override_dir(home))
        .flatten();
    if scope.id == "global" {
        // Root CLAUDE.md (locked) + any relocated auto-memory dir.
        if let Some(root) = &paths.claude_root {
            let claude_md = root.join("CLAUDE.md");
            if claude_md.is_file() {
                items.push(HarnessItem {
                    category: "memory".into(), scope_id: scope.id.clone(),
                    name: "CLAUDE.md".into(), description: String::new(),
                    path: claude_md.display().to_string(),
                    movable: false, deletable: false, locked: true,
                    effective: None,
            mcp_config: None,
                });
            }
            // Legacy/explicit ~/.claude/memory (only when it actually exists).
            let mem_dir = root.join("memory");
            scan_md_dir(&mem_dir, scope, "memory", &mut items, false, false, true);
        }
        // A relocated auto-memory dir (autoMemoryDirectory) is global-ish — it
        // holds one shared MEMORY.md + topic files. Surface it under global so
        // the user sees their real notes regardless of which project wrote them.
        if let Some(dir) = &override_dir {
            scan_md_dir(dir, scope, "memory", &mut items, false, true, true);
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
            mcp_config: None,
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
            mcp_config: None,
                    });
                }
            }
        }
        // Per-project auto-memory lives at ~/.claude/projects/<enc>/memory/.
        // Scan it regardless of any autoMemoryDirectory override — the override
        // relocates only the global/home auto-memory (surfaced under Global);
        // each project keeps its own distinct notes here.
        if let Some(pdir) = &paths.project_dir {
            let memory_dir = pdir.join("memory");
            scan_md_dir(&memory_dir, scope, "memory", &mut items, true, true, true);
        }
    }
    items
}

/// Read the `autoMemoryDirectory` override from the user-scope settings files
/// (`~/.claude/settings.json`, then `settings.local.json` — local wins) and
/// resolve it to an absolute path. Returns `None` when unset or the dir does
/// not exist. Values may be absolute or `~/`-prefixed (per Claude docs).
fn memory_override_dir(home: &Path) -> Option<PathBuf> {
    let root = home.join(".claude");
    // settings.local.json takes precedence over settings.json.
    let mut chosen: Option<String> = None;
    for f in ["settings.json", "settings.local.json"] {
        let p = root.join(f);
        let Ok(content) = std::fs::read_to_string(&p) else { continue };
        let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) else { continue };
        if let Some(v) = cfg.get("autoMemoryDirectory").and_then(|v| v.as_str()) {
            let v = v.trim();
            if !v.is_empty() { chosen = Some(v.to_string()); }
        }
    }
    let raw = chosen?;
    let expanded = expand_home(&raw, home);
    expanded.is_dir().then_some(expanded)
}

/// Expand a leading `~/` (or bare `~`) against `home`; otherwise return the
/// path as-is. Non-`~` relative paths are left relative (Claude only honors
/// absolute or `~/` values, so a relative one simply won't resolve to a dir).
fn expand_home(raw: &str, home: &Path) -> PathBuf {
    if raw == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home.join(rest);
    }
    PathBuf::from(raw)
}

fn scan_mcp(scope: &Scope, home: &Path, paths: &ScopePaths) -> Vec<HarnessItem> {
    let mut items = Vec::new();
    if scope.id == "global" {
        // Claude Code registers user-scope MCP servers ONLY in ~/.claude.json's
        // top-level `mcpServers` (this is what `claude mcp add -s user` writes and
        // `claude mcp list` reads). We previously also scanned ~/.claude/settings.json,
        // ~/.mcp.json and ~/.claude/.mcp.json for a `mcpServers` block — but Claude
        // Code does not register any of those: a `mcpServers` map in settings.json is
        // inert, and `.mcp.json` is a project-root file, not a global source. Merging
        // them (with no dedup) surfaced phantom servers (e.g. a `perplexity` that
        // `claude mcp list` never shows) and duplicate rows (the same server present in
        // two files). Read ONLY the authoritative source so Ward matches `claude mcp list`.
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
        mcp_config: Some(server_config.clone()),
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
        scan_md_dir(&dir, scope, "command", &mut items, true, true, false);
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
        scan_md_dir(&dir, scope, "agent", &mut items, true, true, false);
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
        scan_md_dir(&dir, scope, "plan", &mut items, true, true, false);
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
        scan_md_dir(&dir, scope, "rule", &mut items, true, true, false);
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
            mcp_config: None,
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
        // Emit one row per individual hook command (not per matcher-group), so
        // each row is a distinct, inspectable action instead of N identical
        // rows named after the event. The detail is carried structurally in
        // `mcp_config` so the pane can render a clean card rather than dumping
        // the whole settings.json.
        for (event, hook_array) in hooks {
            let arr = match hook_array.as_array() { Some(a) => a, None => continue };
            for group in arr {
                let matcher = group.get("matcher").and_then(|v| v.as_str()).unwrap_or("");
                let cmds = match group.get("hooks").and_then(|v| v.as_array()) {
                    Some(c) => c,
                    None => continue,
                };
                for cmd in cmds {
                    let htype = cmd.get("type").and_then(|v| v.as_str()).unwrap_or("command");
                    // A command hook carries `command`; an http hook carries `url`.
                    let action = cmd.get("command").and_then(|v| v.as_str())
                        .or_else(|| cmd.get("url").and_then(|v| v.as_str()))
                        .unwrap_or("");
                    let timeout = cmd.get("timeout").and_then(|v| v.as_u64());
                    // Row name = event; description = a compact matcher + action
                    // summary so the list is scannable without opening each row.
                    let short_action = summarize_command(action);
                    let mut desc = String::new();
                    if !matcher.is_empty() && matcher != "*" {
                        desc.push_str(&format!("[{}] ", truncate_str(matcher, 28)));
                    }
                    desc.push_str(&short_action);
                    let detail = serde_json::json!({
                        "kind": "hook",
                        "event": event,
                        "matcher": matcher,
                        "type": htype,
                        "action": action,
                        "timeout": timeout,
                        "source": label,
                    });
                    items.push(HarnessItem {
                        category: "hook".into(),
                        scope_id: scope.id.clone(),
                        name: event.clone(),
                        description: desc,
                        path: p.display().to_string(),
                        movable: false, deletable: false, locked: true,
                        effective: None,
                        mcp_config: Some(detail),
                    });
                }
            }
        }
    }
    items
}

/// Compact a hook command for the list description: collapse whitespace,
/// take the last path segment of an absolute script path when the whole
/// thing is just `run <script>`, and truncate. Keeps rows scannable.
fn summarize_command(cmd: &str) -> String {
    let collapsed = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_str(&collapsed, 72)
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max { return s.to_string(); }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// One-line preview of a settings value for the list row. Scalars render
/// literally; objects/arrays render a compact shape hint (e.g. `{3 keys}`,
/// `[5 items]`) so a huge nested block doesn't blow out the row.
fn summarize_json_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => truncate_str(s, 72),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Array(a) => format!("[{} item{}]", a.len(), if a.len() == 1 { "" } else { "s" }),
        serde_json::Value::Object(o) => format!("{{{} key{}}}", o.len(), if o.len() == 1 { "" } else { "s" }),
    }
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
            for (key, value) in obj {
                // Skip hooks (handled by scan_hooks) and mcpServers (handled by scan_mcp).
                if key == "hooks" || key == "mcpServers" { continue; }
                // Surface the value inline so the row reads "key = value" and
                // the detail pane can render just this setting (not the whole
                // file). Carry the raw JSON value structurally in mcp_config.
                let source = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                let preview = summarize_json_value(value);
                items.push(HarnessItem {
                    category: "setting".into(),
                    scope_id: scope.id.clone(),
                    name: key.clone(),
                    description: preview,
                    path: p.display().to_string(),
                    movable: false, deletable: false, locked: true,
                    effective: None,
                    mcp_config: Some(serde_json::json!({
                        "kind": "setting",
                        "key": key,
                        "value": value,
                        "source": source,
                    })),
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
    // enabledPlugins from settings.json maps "<name>@<marketplace>" → bool.
    let enabled_map = plugin_enabled_map(home);
    for (plugin_key, installs) in plugins {
        let arr = match installs.as_array() { Some(a) => a, None => continue };
        // A plugin key can carry multiple install records (e.g. different
        // scopes). Collapse to a single row — pick the newest by lastUpdated
        // (falling back to installedAt) so we don't show N duplicate rows.
        let install = arr.iter().max_by_key(|i| {
            i.get("lastUpdated").and_then(|v| v.as_str())
                .or_else(|| i.get("installedAt").and_then(|v| v.as_str()))
                .unwrap_or("").to_string()
        });
        let Some(install) = install else { continue };
        let install_path = install.get("installPath").and_then(|v| v.as_str()).unwrap_or("");
        if install_path.is_empty() { continue; }
        let version = install.get("version").and_then(|v| v.as_str()).unwrap_or("");
        // Split "name@marketplace" for a cleaner display + marketplace chip.
        let (short_name, marketplace) = match plugin_key.rsplit_once('@') {
            Some((n, m)) => (n.to_string(), m.to_string()),
            None => (plugin_key.clone(), String::new()),
        };
        let enabled = enabled_map.get(plugin_key).copied().unwrap_or(true);
        // Description carries the human-facing summary: version, marketplace,
        // and enabled state. The detail pane reads this instead of dumping the
        // install directory.
        let mut desc_parts: Vec<String> = Vec::new();
        if !version.is_empty() && version != "unknown" { desc_parts.push(format!("v{version}")); }
        if !marketplace.is_empty() { desc_parts.push(marketplace.clone()); }
        desc_parts.push(if enabled { "enabled".into() } else { "disabled".into() });
        items.push(HarnessItem {
            category: "plugin".into(),
            scope_id: "global".into(),
            name: short_name,
            description: desc_parts.join(" · "),
            path: install_path.to_string(),
            movable: false, deletable: false, locked: true,
            effective: None,
            mcp_config: None,
        });
    }
    // Stable order: enabled first, then alphabetical, so the list doesn't
    // reshuffle between scans (HashMap iteration order is nondeterministic).
    items.sort_by(|a, b| a.name.cmp(&b.name));
    items
}

/// Read `enabledPlugins` from `~/.claude/settings.json` — a map of
/// `"<name>@<marketplace>"` → bool. Absent keys default to enabled.
fn plugin_enabled_map(home: &Path) -> std::collections::HashMap<String, bool> {
    let mut map = std::collections::HashMap::new();
    let p = home.join(".claude").join("settings.json");
    let Ok(content) = std::fs::read_to_string(&p) else { return map };
    let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) else { return map };
    if let Some(obj) = cfg.get("enabledPlugins").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if let Some(b) = v.as_bool() { map.insert(k.clone(), b); }
        }
    }
    map
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
        let title = crate::sessions::parse::session_head_title(
            &p,
            crate::sessions::parse::SESSION_HEAD_CAP,
        )
        .unwrap_or_else(|| stem.clone());
        items.push(HarnessItem {
            category: "session".into(),
            scope_id: scope.id.clone(),
            name: title,
            description: String::new(),
            path: p.display().to_string(),
            movable: false, deletable: false, locked: true,
            effective: None,
            mcp_config: None,
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
    include_memory_md: bool,
) {
    let entries = match std::fs::read_dir(dir) { Ok(e) => e, Err(_) => return };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() { continue; }
        if p.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
        // MEMORY.md is the auto-memory index — skip it for skill/command/etc.
        // listings, but for the memory category it's the most important file
        // to surface, so callers opt in via `include_memory_md`.
        let is_memory_index = p.file_name().and_then(|n| n.to_str()) == Some("MEMORY.md");
        if is_memory_index && !include_memory_md { continue; }
        let content = std::fs::read_to_string(&p).unwrap_or_default();
        let fm = parse_frontmatter(&content);
        let file_name = p.file_name().unwrap().to_string_lossy().to_string();
        // MEMORY.md keeps its full name (it's an index, not a titled note);
        // other files fall back to their frontmatter `name` or stem.
        let display = if is_memory_index {
            file_name.clone()
        } else {
            fm.get("name").cloned()
                .unwrap_or_else(|| file_name.trim_end_matches(".md").to_string())
        };
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
            mcp_config: None,
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
        assert!(c.mcp_editable, "Claude advertises an MCP upsert backend");
        assert!(c.skill_creatable, "Claude advertises a skill-create backend");
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
    fn scan_skills_populates_description_from_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join(".claude/skills/deploy");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: deploy\ndescription: Ship the app to prod\n---\nbody",
        )
        .unwrap();
        let ctx = Ctx { home: dir.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let items = ClaudeAdapter.scan_category(&ctx, "skill", &scope).unwrap();
        let deploy = items.iter().find(|i| i.name == "deploy").unwrap();
        // Description now flows through so the budget listing + Organizer
        // detail can use it.
        assert_eq!(deploy.description, "Ship the app to prod");
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

    #[test]
    fn memory_includes_memory_md_index() {
        // MEMORY.md (the auto-memory index) must show up in the memory listing
        // even though scan_md_dir skips it for skill/command/etc.
        let dir = tempfile::tempdir().unwrap();
        let mem = dir.path().join(".claude/memory");
        fs::create_dir_all(&mem).unwrap();
        fs::write(mem.join("MEMORY.md"), "# index\n").unwrap();
        fs::write(mem.join("debugging.md"), "notes").unwrap();
        let ctx = Ctx { home: dir.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let items = ClaudeAdapter.scan_category(&ctx, "memory", &scope).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"MEMORY.md"), "MEMORY.md must be surfaced: {names:?}");
        assert!(names.contains(&"debugging"));
    }

    #[test]
    fn memory_honors_auto_memory_directory_override() {
        // With autoMemoryDirectory set to a real dir, its MEMORY.md is surfaced
        // under the global scope.
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        let custom = dir.path().join("my-memory");
        fs::create_dir_all(&custom).unwrap();
        fs::write(custom.join("MEMORY.md"), "# custom index\n").unwrap();
        fs::write(custom.join("api.md"), "notes").unwrap();
        fs::write(
            claude.join("settings.json"),
            format!(r#"{{"autoMemoryDirectory":"{}"}}"#, custom.display()),
        ).unwrap();
        let ctx = Ctx { home: dir.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let items = ClaudeAdapter.scan_category(&ctx, "memory", &scope).unwrap();
        let paths: Vec<&str> = items.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.iter().any(|p| p.contains("my-memory") && p.ends_with("MEMORY.md")),
            "override MEMORY.md must be surfaced: {paths:?}");
        assert!(paths.iter().any(|p| p.contains("my-memory") && p.ends_with("api.md")));
    }

    #[test]
    fn memory_scans_per_project_dir_even_with_override() {
        // Regression: `autoMemoryDirectory` relocates only the GLOBAL/home
        // auto-memory (surfaced under the global scope). Each project keeps its
        // own distinct notes at ~/.claude/projects/<enc>/memory/ — a location
        // that never equals the override dir. A guard used to skip the
        // per-project dir entirely whenever the override was set, hiding every
        // project's memory notes (and dropping memory-only projects from the
        // list). The per-project dir must be scanned unconditionally.
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        // autoMemoryDirectory override -> a real, existing global override dir.
        let custom = dir.path().join("my-memory");
        fs::create_dir_all(&custom).unwrap();
        fs::write(custom.join("MEMORY.md"), "# custom index\n").unwrap();
        fs::write(
            claude.join("settings.json"),
            format!(r#"{{"autoMemoryDirectory":"{}"}}"#, custom.display()),
        ).unwrap();
        // A project whose per-project auto-memory dir holds a distinct note.
        let encoded = "-work-proj-with-memory";
        make_project_dir(dir.path(), encoded, None);
        let pmem = dir.path().join(".claude").join("projects").join(encoded).join("memory");
        fs::create_dir_all(&pmem).unwrap();
        fs::write(pmem.join("proj-note.md"), "project note").unwrap();

        // Scan the memory category for the PROJECT (non-global) scope.
        let scope = Scope {
            id: encoded.into(),
            kind: "project-unresolved".into(),
            label: "proj".into(),
            root: dir.path().join(".claude").join("projects").join(encoded).display().to_string(),
        };
        let paths = ScopePaths::for_scope(dir.path(), &scope);
        let items = scan_memories(&scope, dir.path(), &paths);
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"proj-note"),
            "per-project memory note must be scanned even with autoMemoryDirectory set: {names:?}");
    }

    #[test]
    fn expand_home_handles_tilde() {
        let home = Path::new("/Users/x");
        assert_eq!(expand_home("~/mem", home), PathBuf::from("/Users/x/mem"));
        assert_eq!(expand_home("~", home), PathBuf::from("/Users/x"));
        assert_eq!(expand_home("/abs/mem", home), PathBuf::from("/abs/mem"));
    }

    #[test]
    fn scan_mcp_global_reads_only_claude_json_toplevel() {
        // Claude Code registers user-scope MCP servers ONLY in ~/.claude.json's
        // top-level `mcpServers` (what `claude mcp add -s user` writes and
        // `claude mcp list` reads). A `mcpServers` block inside
        // ~/.claude/settings.json (or ~/.mcp.json / ~/.claude/.mcp.json) is inert
        // — Claude Code never registers it — so surfacing it produced phantom
        // servers (a `perplexity` that `claude mcp list` omits) and duplicate rows
        // (the same server present in two files). Global MCP must match reality.
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        // Authoritative user-scope registrations.
        fs::write(
            dir.path().join(".claude.json"),
            r#"{"mcpServers":{"Context7":{"command":"npx","args":["-y","ctx"]},"reddit":{"command":"npx","args":["reddit"]}}}"#,
        ).unwrap();
        // Inert settings.json block — must NOT surface as registered MCP items.
        fs::write(
            claude.join("settings.json"),
            r#"{"mcpServers":{"Context7":{"command":"npx"},"perplexity":{"command":"npx"}}}"#,
        ).unwrap();
        // A stray ~/.mcp.json (a project-root file, not a global registration).
        fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers":{"stray":{"command":"npx"}}}"#,
        ).unwrap();

        let ctx = Ctx { home: dir.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let items = ClaudeAdapter.scan_category(&ctx, "mcp", &scope).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();

        assert_eq!(items.len(), 2, "global MCP must equal ~/.claude.json top-level only: {names:?}");
        assert!(names.contains(&"Context7"));
        assert!(names.contains(&"reddit"));
        assert!(!names.contains(&"perplexity"), "settings.json mcpServers must not surface");
        assert!(!names.contains(&"stray"), "~/.mcp.json must not surface as global");
        assert_eq!(names.iter().filter(|n| **n == "Context7").count(), 1, "Context7 must not duplicate");
        // Every global MCP item must point at ~/.claude.json (the writable registration file).
        assert!(items.iter().all(|i| i.path.ends_with(".claude.json")),
            "global MCP path must be ~/.claude.json: {:?}", items.iter().map(|i| &i.path).collect::<Vec<_>>());
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
        // The command is surfaced in the description + structured in mcp_config.
        assert!(items[0].description.contains("echo hi"), "desc: {}", items[0].description);
        let detail = items[0].mcp_config.as_ref().expect("hook detail");
        assert_eq!(detail.get("action").and_then(|v| v.as_str()), Some("echo hi"));
        assert_eq!(detail.get("event").and_then(|v| v.as_str()), Some("PreToolUse"));
    }

    #[test]
    fn scan_hooks_emits_one_row_per_command() {
        // A single event with two matcher-groups, each with one command, must
        // produce two distinct rows (not one row per event, not collapsed).
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        fs::write(claude.join("settings.json"), r#"{"hooks":{"PostToolUse":[
            {"matcher":"Edit","hooks":[{"type":"command","command":"fmt.sh"}]},
            {"matcher":"*","hooks":[{"type":"command","command":"log.sh"},{"type":"command","command":"notify.sh"}]}
        ]}}"#).unwrap();
        let ctx = Ctx { home: dir.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let items = ClaudeAdapter.scan_category(&ctx, "hook", &scope).unwrap();
        assert_eq!(items.len(), 3, "one row per individual hook command");
        let actions: Vec<String> = items.iter()
            .filter_map(|i| i.mcp_config.as_ref())
            .filter_map(|d| d.get("action").and_then(|v| v.as_str()).map(String::from))
            .collect();
        assert!(actions.contains(&"fmt.sh".to_string()));
        assert!(actions.contains(&"log.sh".to_string()));
        assert!(actions.contains(&"notify.sh".to_string()));
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
        // outputStyle=dark should surface its value inline + structurally.
        let os = items.iter().find(|i| i.name == "outputStyle").unwrap();
        assert!(os.description.contains("dark"), "desc: {}", os.description);
        let detail = os.mcp_config.as_ref().expect("setting detail");
        assert_eq!(detail.get("value").and_then(|v| v.as_str()), Some("dark"));
    }

    #[test]
    fn scan_plugins_reads_installed_plugins_json() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join(".claude").join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        fs::write(plugins.join("installed_plugins.json"),
            r#"{"plugins":{"demo-plugin@some-mp":[{"scope":"user","installPath":"/Users/x/.claude/plugins/demo","version":"1.2.3"}]}}"#
        ).unwrap();
        let items = scan_plugins(dir.path());
        assert_eq!(items.len(), 1);
        // Name is the short form (marketplace stripped); marketplace + version
        // move into the description.
        assert_eq!(items[0].name, "demo-plugin");
        assert!(items[0].description.contains("v1.2.3"));
        assert!(items[0].description.contains("some-mp"));
        assert_eq!(items[0].scope_id, "global");
        assert!(items[0].locked && !items[0].movable);
    }

    #[test]
    fn scan_plugins_dedupes_multiple_installs() {
        // A plugin key with two install records collapses to one row (newest).
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join(".claude").join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        fs::write(plugins.join("installed_plugins.json"),
            r#"{"plugins":{"p@mp":[
                {"installPath":"/a","version":"1.0.0","lastUpdated":"2026-01-01T00:00:00Z"},
                {"installPath":"/b","version":"2.0.0","lastUpdated":"2026-06-01T00:00:00Z"}
            ]}}"#
        ).unwrap();
        let items = scan_plugins(dir.path());
        assert_eq!(items.len(), 1, "multiple installs collapse to one row");
        assert!(items[0].description.contains("v2.0.0"));
    }

    #[test]
    fn scan_plugins_reflects_enabled_state() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        let plugins = claude.join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        fs::write(plugins.join("installed_plugins.json"),
            r#"{"plugins":{"off@mp":[{"installPath":"/x","version":"1.0.0"}]}}"#
        ).unwrap();
        fs::write(claude.join("settings.json"),
            r#"{"enabledPlugins":{"off@mp":false}}"#
        ).unwrap();
        let items = scan_plugins(dir.path());
        assert_eq!(items.len(), 1);
        assert!(items[0].description.contains("disabled"), "desc: {}", items[0].description);
    }

    #[test]
    fn scan_plugins_only_emitted_once_across_scopes() {
        // Regression: scan_category is called per-scope; plugins must only be
        // emitted for the global scope, never duplicated per project scope.
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        let plugins = claude.join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        fs::write(plugins.join("installed_plugins.json"),
            r#"{"plugins":{"p@mp":[{"installPath":"/x","version":"1.0.0"}]}}"#
        ).unwrap();
        // Create a project scope so the run has >1 scope.
        make_project_dir(dir.path(), "-work-demo", Some(&dir.path().join("work/demo")));
        fs::create_dir_all(dir.path().join("work/demo")).unwrap();
        let ctx = Ctx { home: dir.path(), cwd: None };
        let result = framework::run_scan(&ClaudeAdapter, &ctx).unwrap();
        let plugin_items: Vec<_> = result.items.iter().filter(|i| i.category == "plugin").collect();
        assert_eq!(plugin_items.len(), 1, "plugin must appear exactly once, not once per scope");
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
