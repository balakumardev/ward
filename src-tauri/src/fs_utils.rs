use std::path::{Component, Path, PathBuf};
use crate::error::WardError;

/// Confine `path` under `home`. Relative paths are joined onto `home`.
/// Any `..` component, or an absolute path not under `home`, is rejected.
pub fn ensure_under_home(path: &Path, home: &Path) -> Result<PathBuf, WardError> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(WardError::PathEscaped(path.display().to_string()));
    }
    let abs = if path.is_absolute() { path.to_path_buf() } else { home.join(path) };
    if !abs.starts_with(home) {
        return Err(WardError::PathEscaped(abs.display().to_string()));
    }
    Ok(abs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_path_under_home() {
        let home = Path::new("/Users/x");
        let p = Path::new("/Users/x/.claude/skills/a/SKILL.md");
        assert_eq!(ensure_under_home(p, home).unwrap(), p.to_path_buf());
    }

    #[test]
    fn rejects_parent_traversal() {
        let home = Path::new("/Users/x");
        let p = Path::new("/Users/x/../etc/passwd");
        assert!(matches!(ensure_under_home(p, home), Err(WardError::PathEscaped(_))));
    }

    #[test]
    fn rejects_outside_home() {
        let home = Path::new("/Users/x");
        let p = Path::new("/etc/passwd");
        assert!(matches!(ensure_under_home(p, home), Err(WardError::PathEscaped(_))));
    }
}
