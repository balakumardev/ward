use std::path::Path;
use crate::error::WardError;
use crate::harness::{Ctx, Harness};
use crate::model::{Capabilities, HarnessItem, Scope};

pub struct ClaudeAdapter;

impl ClaudeAdapter {
    fn claude_root(home: &Path) -> std::path::PathBuf {
        home.join(".claude")
    }
}

impl Harness for ClaudeAdapter {
    fn id(&self) -> &str { "claude" }
    fn display_name(&self) -> &str { "Claude Code" }
    fn short_name(&self) -> &str { "Claude" }
    fn icon(&self) -> &str { "◆" }
    fn executable(&self) -> &str { "claude" }

    fn category_ids(&self) -> Vec<&'static str> { vec!["skill", "memory"] }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            context_budget: true, mcp_controls: true, mcp_policy: true,
            mcp_security: true, sessions: true, effective: true, backup: true,
        }
    }

    fn discover_scopes(&self, ctx: &Ctx) -> Result<Vec<Scope>, WardError> {
        let root = Self::claude_root(ctx.home);
        Ok(vec![Scope {
            id: "global".into(),
            kind: "global".into(),
            label: "Global (~/.claude)".into(),
            root: root.display().to_string(),
        }])
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
}
