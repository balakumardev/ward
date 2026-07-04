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

pub fn read_file_impl(path: &Path, home: &Path) -> Result<String, WardError> {
    let abs = crate::fs_utils::ensure_under_home(path, home)?;
    Ok(std::fs::read_to_string(abs)?)
}

#[tauri::command]
pub fn read_file_content(path: String) -> Result<String, WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    read_file_impl(Path::new(&path), &home)
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

    #[test]
    fn reads_allowed_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join(".claude/x.md");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, "hello").unwrap();
        assert_eq!(read_file_impl(&f, dir.path()).unwrap(), "hello");
    }

    #[test]
    fn rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("../etc/passwd");
        assert!(matches!(read_file_impl(&bad, dir.path()), Err(WardError::PathEscaped(_))));
    }
}