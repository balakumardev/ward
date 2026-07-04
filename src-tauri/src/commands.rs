use std::path::Path;
use crate::error::WardError;
use crate::harness::adapters::claude::ClaudeAdapter;
use crate::harness::{framework, Ctx, Registry};
use crate::model::ScanResult;

pub fn build_registry() -> Registry {
    let mut r = Registry::new();
    r.register(Box::new(ClaudeAdapter));
    r
}

pub fn scan_impl(registry: &Registry, home: &Path, harness_id: &str) -> Result<ScanResult, WardError> {
    let adapter = registry
        .get(harness_id)
        .ok_or_else(|| WardError::HarnessUnavailable(harness_id.to_string()))?;
    let ctx = Ctx { home, cwd: None };
    framework::run_scan(adapter, &ctx)
}

#[tauri::command]
pub fn scan(harness: String) -> Result<ScanResult, WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    let registry = build_registry();
    scan_impl(&registry, &home, &harness)
}

// Placeholder so the handler list compiles before Task 10 lands.
// Task 10 will replace this body with the real implementation.
#[tauri::command]
pub fn read_file_content() -> Result<String, WardError> {
    Err(WardError::NotFound("not implemented yet".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scan_impl_returns_claude_result() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".claude/skills/a")).unwrap();
        fs::write(dir.path().join(".claude/skills/a/SKILL.md"), "x").unwrap();

        let registry = build_registry();
        let result = scan_impl(&registry, dir.path(), "claude").unwrap();
        assert_eq!(result.harness_id, "claude");
        assert_eq!(result.items.iter().filter(|i| i.category == "skill").count(), 1);
    }

    #[test]
    fn scan_impl_unknown_harness_errors() {
        let registry = build_registry();
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            scan_impl(&registry, dir.path(), "nope"),
            Err(WardError::HarnessUnavailable(_))
        ));
    }
}