//! Effective resolution — per-category rules for cross-scope availability.
//!
//! Ports the semantics of CCO's `src/effective.mjs`. A category "participates
//! in Show Effective" when there is an official rule describing how its
//! items are resolved across scopes (skill, mcp, command, agent, config,
//! hook, memory). The remaining categories (plan, rule, session, plugin)
//! do not participate; their items are never included as global or
//! ancestor contributions for a project scope.

use std::collections::{HashMap, HashSet};
use crate::model::{HarnessItem, Scope};

/// Categories that participate in effective resolution.
pub static EFFECTIVE_RULES: &[(&str, &str)] = &[
    ("skill",   "Available from Personal (~/.claude/skills), Project (.claude/skills), and installed Plugins"),
    ("mcp",     "Resolved by local > project > user — same-name servers use the narrower scope"),
    ("command", "Available from User and Project — same-name conflicts are not supported"),
    ("agent",   "Project-level agents override same-name User agents"),
    ("config",  "Resolved by precedence: managed > CLI > project local > project shared > user"),
    ("hook",    "Configured in settings files — resolved by settings precedence"),
    ("memory",  "Global memories are available in all projects; project memories are specific to this project"),
];

/// Build a quick lookup of category → rule text.
pub fn effective_rules_map() -> HashMap<&'static str, &'static str> {
    EFFECTIVE_RULES.iter().copied().collect()
}

/// Returns true if `category` participates in Show Effective.
pub fn has_effective_rule(category: &str) -> bool {
    EFFECTIVE_RULES.iter().any(|(c, _)| *c == category)
}

/// Find scopes whose `root` is a path ancestor of the given scope's root.
/// e.g. `/work/company` is an ancestor of `/work/company/repo-a`. Global is
/// excluded. The deepest ancestor is returned first.
pub fn get_ancestor_scopes(scope_id: &str, scopes: &[Scope]) -> Vec<Scope> {
    let scope = match scopes.iter().find(|s| s.id == scope_id) {
        Some(s) => s,
        None => return vec![],
    };
    if scope_id == "global" {
        return vec![];
    }
    let mut ancestors: Vec<&Scope> = scopes.iter()
        .filter(|s| s.id != scope_id && s.id != "global" && s.kind == "project")
        .filter(|s| scope.root.starts_with(&format!("{}/", s.root)))
        .collect();
    // Deepest first — sort by root path length descending.
    ancestors.sort_by(|a, b| b.root.len().cmp(&a.root.len()));
    ancestors.into_iter().cloned().collect()
}

/// Key function — matches `itemKey` in CCO: `${category}::${name}::${scopeId}`.
pub fn item_key(item: &HarnessItem) -> String {
    format!("{}::{}::{}", item.category, item.name, item.scope_id)
}

/// Compute the shadowed / conflict / ancestor sets for a given scope.
///
/// - `shadowed_keys`: keys for global items of category `mcp` or `agent` that
///   have the same `name` as a project-scope item of the same category.
/// - `conflict_keys`: keys for any `command` item whose name appears in both
///   global and project scopes.
/// - `ancestor_keys`: keys for `config` and `memory` items from ancestor scopes.
///
/// Returns three empty sets for the `global` scope.
pub fn compute_effective_sets(
    scope_id: &str,
    all_items: &[HarnessItem],
    scopes: &[Scope],
) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
    let mut shadowed_keys = HashSet::new();
    let mut conflict_keys = HashSet::new();
    let mut ancestor_keys = HashSet::new();

    if scope_id.is_empty() || scope_id == "global" {
        return (shadowed_keys, conflict_keys, ancestor_keys);
    }

    let project_items: Vec<&HarnessItem> = all_items.iter().filter(|i| i.scope_id == scope_id).collect();
    let global_items: Vec<&HarnessItem> = all_items.iter().filter(|i| i.scope_id == "global").collect();

    // MCP & Agents: narrower (project) scope wins same-name.
    for cat in ["mcp", "agent"] {
        let project_names: HashSet<&str> = project_items.iter()
            .filter(|i| i.category == cat)
            .map(|i| i.name.as_str())
            .collect();
        for gi in global_items.iter().filter(|i| i.category == cat) {
            if project_names.contains(gi.name.as_str()) {
                shadowed_keys.insert(item_key(gi));
            }
        }
    }

    // Commands: same-name in both scopes → conflict.
    let proj_cmd_names: HashSet<&str> = project_items.iter()
        .filter(|i| i.category == "command")
        .map(|i| i.name.as_str())
        .collect();
    let global_cmd_names: HashSet<&str> = global_items.iter()
        .filter(|i| i.category == "command")
        .map(|i| i.name.as_str())
        .collect();
    for name in &proj_cmd_names {
        if !global_cmd_names.contains(name) { continue; }
        for i in project_items.iter().chain(global_items.iter())
            .filter(|i| i.category == "command" && i.name == *name)
        {
            conflict_keys.insert(item_key(i));
        }
    }

    // Ancestor scopes: config + memory items from path-parent scopes.
    let ancestors = get_ancestor_scopes(scope_id, scopes);
    for as_ in &ancestors {
        for i in all_items.iter()
            .filter(|i| i.scope_id == as_.id && (i.category == "config" || i.category == "memory"))
        {
            ancestor_keys.insert(item_key(i));
        }
    }

    (shadowed_keys, conflict_keys, ancestor_keys)
}

/// Get the effective items for a given scope.
///
/// - For `global`: just the global items.
/// - For project scopes: project items + global items of participating
///   categories + config/memory items from ancestor scopes.
pub fn get_effective_items(
    scope_id: &str,
    all_items: &[HarnessItem],
    scopes: &[Scope],
) -> Vec<HarnessItem> {
    let project_items: Vec<HarnessItem> = all_items.iter()
        .filter(|i| i.scope_id == scope_id)
        .cloned()
        .collect();

    if scope_id == "global" {
        return project_items;
    }

    // Global items only for participating categories.
    let effective_global: Vec<HarnessItem> = all_items.iter()
        .filter(|i| i.scope_id == "global" && has_effective_rule(i.category.as_str()))
        .cloned()
        .collect();

    // Ancestor items: config + memory only.
    let mut ancestor_items: Vec<HarnessItem> = Vec::new();
    for as_ in get_ancestor_scopes(scope_id, scopes) {
        for i in all_items.iter()
            .filter(|i| i.scope_id == as_.id && (i.category == "config" || i.category == "memory"))
        {
            ancestor_items.push(i.clone());
        }
    }

    let mut out = project_items;
    out.extend(effective_global);
    out.extend(ancestor_items);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(cat: &str, name: &str, scope: &str) -> HarnessItem {
        HarnessItem {
            category: cat.into(),
            scope_id: scope.into(),
            name: name.into(),
            description: String::new(),
            path: format!("/{cat}/{name}"),
            movable: true,
            deletable: true,
            locked: false,
            effective: None,
        }
    }

    fn scopes() -> Vec<Scope> {
        vec![
            Scope { id: "global".into(), kind: "global".into(), label: "Global".into(), root: "/Users/x/.claude".into() },
            Scope { id: "company".into(), kind: "project".into(), label: "company".into(), root: "/work/company".into() },
            Scope { id: "repo-a".into(), kind: "project".into(), label: "repo-a".into(), root: "/work/company/repo-a".into() },
        ]
    }

    fn items() -> Vec<HarnessItem> {
        vec![
            // Global
            make_item("skill", "deploy", "global"),
            make_item("skill", "lint", "global"),
            make_item("mcp", "github", "global"),
            make_item("mcp", "slack", "global"),
            make_item("command", "test", "global"),
            make_item("command", "deploy", "global"),
            make_item("agent", "reviewer", "global"),
            make_item("agent", "planner", "global"),
            make_item("config", "CLAUDE.md", "global"),
            make_item("memory", "user_prefs", "global"),
            make_item("plan", "roadmap", "global"),
            make_item("rule", "no-eval", "global"),
            make_item("session", "session-1", "global"),
            make_item("hook", "pre-tool", "global"),
            // Company (ancestor of repo-a)
            make_item("config", "CLAUDE.md", "company"),
            make_item("memory", "company_standards", "company"),
            // repo-a
            make_item("skill", "local-build", "repo-a"),
            make_item("mcp", "github", "repo-a"),
            make_item("command", "deploy", "repo-a"),
            make_item("agent", "planner", "repo-a"),
            make_item("config", "settings.json", "repo-a"),
            make_item("memory", "project_notes", "repo-a"),
            make_item("plan", "sprint", "repo-a"),
            make_item("rule", "no-console", "repo-a"),
        ]
    }

    // ── EFFECTIVE_RULES participation ──

    #[test]
    fn participating_categories_have_rules() {
        for cat in ["skill", "mcp", "command", "agent", "config", "hook", "memory"] {
            assert!(has_effective_rule(cat), "{cat} should have a rule");
            assert!(EFFECTIVE_RULES.iter().any(|(c, _)| *c == cat));
        }
    }

    #[test]
    fn non_participating_categories_have_no_rules() {
        for cat in ["plan", "rule", "session", "plugin"] {
            assert!(!has_effective_rule(cat), "{cat} should NOT have a rule");
        }
    }

    // ── get_effective_items per-category ──

    #[test]
    fn skills_shows_project_and_global() {
        let s = scopes();
        let it = items();
        let effective = get_effective_items("repo-a", &it, &s);
        let mut names: Vec<&str> = effective.iter()
            .filter(|i| i.category == "skill")
            .map(|i| i.name.as_str())
            .collect();
        names.sort();
        assert_eq!(names, vec!["deploy", "lint", "local-build"]);
    }

    #[test]
    fn mcp_shows_both_github_entries() {
        let s = scopes();
        let it = items();
        let effective = get_effective_items("repo-a", &it, &s);
        let mcps: Vec<&HarnessItem> = effective.iter().filter(|i| i.category == "mcp").collect();
        assert!(mcps.iter().any(|i| i.name == "github" && i.scope_id == "repo-a"));
        assert!(mcps.iter().any(|i| i.name == "github" && i.scope_id == "global"));
        assert!(mcps.iter().any(|i| i.name == "slack"));
    }

    #[test]
    fn commands_deploy_appears_twice() {
        let s = scopes();
        let it = items();
        let effective = get_effective_items("repo-a", &it, &s);
        let mut names: Vec<&str> = effective.iter()
            .filter(|i| i.category == "command")
            .map(|i| i.name.as_str())
            .collect();
        names.sort();
        assert_eq!(names, vec!["deploy", "deploy", "test"]);
    }

    #[test]
    fn agents_planner_appears_twice() {
        let s = scopes();
        let it = items();
        let effective = get_effective_items("repo-a", &it, &s);
        let mut names: Vec<&str> = effective.iter()
            .filter(|i| i.category == "agent")
            .map(|i| i.name.as_str())
            .collect();
        names.sort();
        assert_eq!(names, vec!["planner", "planner", "reviewer"]);
    }

    #[test]
    fn config_shows_project_global_ancestor() {
        let s = scopes();
        let it = items();
        let effective = get_effective_items("repo-a", &it, &s);
        let configs: Vec<String> = effective.iter()
            .filter(|i| i.category == "config")
            .map(|i| format!("{}@{}", i.name, i.scope_id))
            .collect();
        assert!(configs.contains(&"CLAUDE.md@global".to_string()));
        assert!(configs.contains(&"CLAUDE.md@company".to_string()));
        assert!(configs.contains(&"settings.json@repo-a".to_string()));
    }

    #[test]
    fn memory_shows_project_global_ancestor() {
        let s = scopes();
        let it = items();
        let effective = get_effective_items("repo-a", &it, &s);
        let mems: Vec<String> = effective.iter()
            .filter(|i| i.category == "memory")
            .map(|i| format!("{}@{}", i.name, i.scope_id))
            .collect();
        assert!(mems.contains(&"user_prefs@global".to_string()));
        assert!(mems.contains(&"project_notes@repo-a".to_string()));
        assert!(mems.contains(&"company_standards@company".to_string()));
    }

    #[test]
    fn hooks_shows_project_and_global() {
        let s = scopes();
        let it = items();
        let effective = get_effective_items("repo-a", &it, &s);
        let hooks: Vec<&HarnessItem> = effective.iter().filter(|i| i.category == "hook").collect();
        assert!(hooks.iter().any(|i| i.scope_id == "global"));
    }

    // ── get_effective_items — non-participating categories excluded from global ──

    #[test]
    fn plans_from_global_excluded() {
        let s = scopes();
        let it = items();
        let effective = get_effective_items("repo-a", &it, &s);
        let plans: Vec<&HarnessItem> = effective.iter().filter(|i| i.category == "plan").collect();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].scope_id, "repo-a");
    }

    #[test]
    fn rules_from_global_excluded() {
        let s = scopes();
        let it = items();
        let effective = get_effective_items("repo-a", &it, &s);
        let rules: Vec<&HarnessItem> = effective.iter().filter(|i| i.category == "rule").collect();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].scope_id, "repo-a");
    }

    #[test]
    fn sessions_from_global_excluded() {
        let s = scopes();
        let it = items();
        let effective = get_effective_items("repo-a", &it, &s);
        let sessions: Vec<&HarnessItem> = effective.iter().filter(|i| i.category == "session").collect();
        assert_eq!(sessions.len(), 0);
    }

    // ── compute_effective_sets shadow / conflict ──

    #[test]
    fn mcp_github_global_is_shadowed() {
        let s = scopes();
        let it = items();
        let (shadowed, _, _) = compute_effective_sets("repo-a", &it, &s);
        assert!(shadowed.contains("mcp::github::global"));
    }

    #[test]
    fn mcp_unique_names_not_shadowed() {
        let s = scopes();
        let it = items();
        let (shadowed, _, _) = compute_effective_sets("repo-a", &it, &s);
        assert!(!shadowed.iter().any(|k| k.contains("slack")));
    }

    #[test]
    fn agent_planner_global_is_shadowed() {
        let s = scopes();
        let it = items();
        let (shadowed, _, _) = compute_effective_sets("repo-a", &it, &s);
        assert!(shadowed.contains("agent::planner::global"));
    }

    #[test]
    fn agent_unique_names_not_shadowed() {
        let s = scopes();
        let it = items();
        let (shadowed, _, _) = compute_effective_sets("repo-a", &it, &s);
        assert!(!shadowed.iter().any(|k| k.contains("reviewer")));
    }

    #[test]
    fn command_deploy_in_both_scopes_flagged_as_conflict() {
        let s = scopes();
        let it = items();
        let (_, conflict, _) = compute_effective_sets("repo-a", &it, &s);
        assert!(conflict.contains("command::deploy::global"));
        assert!(conflict.contains("command::deploy::repo-a"));
    }

    #[test]
    fn command_unique_names_not_conflicts() {
        let s = scopes();
        let it = items();
        let (_, conflict, _) = compute_effective_sets("repo-a", &it, &s);
        assert!(!conflict.iter().any(|k| k.contains("test")));
    }

    #[test]
    fn global_scope_returns_empty_sets() {
        let s = scopes();
        let it = items();
        let (shadowed, conflict, ancestor) = compute_effective_sets("global", &it, &s);
        assert!(shadowed.is_empty());
        assert!(conflict.is_empty());
        assert!(ancestor.is_empty());
    }

    // ── get_ancestor_scopes ──

    #[test]
    fn repo_a_sees_company_as_ancestor() {
        let s = scopes();
        let ancestors = get_ancestor_scopes("repo-a", &s);
        assert!(ancestors.iter().any(|s| s.id == "company"));
    }

    #[test]
    fn company_does_not_see_repo_a_as_ancestor() {
        let s = scopes();
        let ancestors = get_ancestor_scopes("company", &s);
        assert!(!ancestors.iter().any(|s| s.id == "repo-a"));
    }

    #[test]
    fn global_has_no_ancestors() {
        let s = scopes();
        let ancestors = get_ancestor_scopes("global", &s);
        assert!(ancestors.is_empty());
    }

    #[test]
    fn ancestor_config_and_memory_items_are_in_ancestor_keys() {
        let s = scopes();
        let it = items();
        let (_, _, ancestor) = compute_effective_sets("repo-a", &it, &s);
        assert!(ancestor.contains("config::CLAUDE.md::company"));
        assert!(ancestor.contains("memory::company_standards::company"));
    }
}