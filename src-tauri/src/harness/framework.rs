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
        other => other,
    }
    .to_string()
}

pub fn run_scan(adapter: &dyn Harness, ctx: &Ctx) -> Result<ScanResult, WardError> {
    let scopes = adapter.discover_scopes(ctx)?;
    let mut items: Vec<HarnessItem> = Vec::new();
    for scope in &scopes {
        for cat in adapter.category_ids() {
            items.extend(adapter.scan_category(ctx, cat, scope)?);
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
