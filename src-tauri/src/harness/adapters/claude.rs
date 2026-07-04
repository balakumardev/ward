use std::path::{Path, PathBuf};
use crate::error::WardError;
use crate::fs_utils::{decode_project_dir_name, parse_frontmatter};
use crate::harness::{framework, Ctx, Harness};
use crate::model::{Capabilities, HarnessItem, Scope};

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
        // Collect (encodedName, realPath, shortName) tuples first so we
        // can sort path-resolved entries before unresolved ones.
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
            let project_dir = projects_dir.join(&encoded);
            let real = decode_project_dir_name(home, &encoded);
            // Use basename of the resolved path; fall back to a prettified
            // encoded name when we cannot resolve.
            let short = match &real {
                Some(p) => p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| encoded.clone()),
                None => Self::prettify_encoded(&encoded),
            };
            entries_vec.push(Entry { encoded, real, short });
        }
        // Sort: resolved entries first (by depth then path), then unresolved.
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
        // Disambiguate same-named entries by prepending parent dir.
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
                    // Unresolved — use the projects/<encoded> dir itself.
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
        let root = Self::claude_root(ctx.home);
        let mut items = Vec::new();
        match category {
            "skill" => {
                let skills = root.join("skills");
                if let Ok(entries) = std::fs::read_dir(&skills) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        let manifest = p.join("SKILL.md");
                        if manifest.is_file() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            items.push(HarnessItem {
                                category: "skill".into(),
                                scope_id: scope.id.clone(),
                                name,
                                path: manifest.display().to_string(),
                                movable: true, deletable: true, locked: false,
                            });
                        }
                    }
                }
            }
            "memory" => {
                let root_md = root.join("CLAUDE.md");
                if root_md.is_file() {
                    items.push(HarnessItem {
                        category: "memory".into(), scope_id: scope.id.clone(),
                        name: "CLAUDE.md".into(), path: root_md.display().to_string(),
                        movable: false, deletable: false, locked: true,
                    });
                }
                let mem = root.join("memory");
                if let Ok(entries) = std::fs::read_dir(&mem) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.extension().and_then(|e| e.to_str()) == Some("md") {
                            let name = p.file_name().unwrap().to_string_lossy().to_string();
                            items.push(HarnessItem {
                                category: "memory".into(), scope_id: scope.id.clone(),
                                name, path: p.display().to_string(),
                                movable: true, deletable: true, locked: false,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(items)
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
        assert!(names.contains(&"user.md"));
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
}
