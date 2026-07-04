use crate::effective;
use crate::error::WardError;
use crate::harness::{Ctx, Harness};
use crate::model::{Category, HarnessItem, ScanResult};

pub fn category_label(id: &str) -> String {
    match id {
        "skill" => "Skills",
        "memory" => "Memories",
        "mcp" => "MCP",
        "command" => "Commands",
        "agent" => "Agents",
        "hook" => "Hooks",
        "plan" => "Plans",
        "rule" => "Rules",
        "config" => "Config",
        "plugin" => "Plugins",
        "session" => "Sessions",
        "setting" => "Settings",
        "profile" => "Profiles",
        "history" => "History",
        "shell" => "Shell",
        "runtime" => "Runtime",
        other => other,
    }
    .to_string()
}

/// Pick the most relevant project scope for effective resolution.
/// Priority: first non-global project scope, else global.
fn pick_effective_scope(scopes: &[crate::model::Scope]) -> &str {
    scopes.iter()
        .find(|s| s.kind == "project")
        .map(|s| s.id.as_str())
        .unwrap_or("global")
}

pub fn run_scan(adapter: &dyn Harness, ctx: &Ctx) -> Result<ScanResult, WardError> {
    let scopes = adapter.discover_scopes(ctx)?;
    let mut items: Vec<HarnessItem> = Vec::new();
    for scope in &scopes {
        for cat in adapter.category_ids() {
            items.extend(adapter.scan_category(ctx, cat, scope)?);
        }
    }

    // Apply effective tags relative to the most-relevant project scope.
    let effective_scope_id = pick_effective_scope(&scopes);
    let (shadowed, conflict, ancestor) =
        effective::compute_effective_sets(effective_scope_id, &items, &scopes);
    for item in items.iter_mut() {
        let key = effective::item_key(item);
        if shadowed.contains(&key) {
            item.effective = Some("shadowed".to_string());
        } else if conflict.contains(&key) {
            item.effective = Some("conflict".to_string());
        } else if ancestor.contains(&key) {
            item.effective = Some("ancestor".to_string());
        }
    }

    let categories = adapter
        .category_ids()
        .iter()
        .map(|id| Category {
            id: (*id).to_string(),
            label: category_label(id),
            count: items.iter().filter(|i| i.category == *id).count(),
        })
        .collect();
    Ok(ScanResult {
        harness_id: adapter.id().to_string(),
        categories,
        scopes,
        items,
        capabilities: adapter.capabilities(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::adapters::claude::ClaudeAdapter;
    use std::fs;

    #[test]
    fn run_scan_counts_items_per_category() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        fs::create_dir_all(claude.join("skills/a")).unwrap();
        fs::write(claude.join("skills/a/SKILL.md"), "x").unwrap();
        fs::write(claude.join("CLAUDE.md"), "m").unwrap();

        let ctx = Ctx { home: dir.path(), cwd: None };
        let result = run_scan(&ClaudeAdapter, &ctx).unwrap();

        assert_eq!(result.harness_id, "claude");
        let skill_cat = result.categories.iter().find(|c| c.id == "skill").unwrap();
        assert_eq!(skill_cat.count, 1);
        assert_eq!(skill_cat.label, "Skills");
        assert_eq!(result.items.iter().filter(|i| i.category == "skill").count(), 1);
    }
}
