use std::collections::HashMap;
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

/// Parse simple YAML frontmatter (`---\nkey: value\n---\n`) returning
/// flat string pairs. Non-string values are stringified. Returns empty
/// map when no frontmatter is present. Mirrors `parseFrontmatter` in CCO.
pub fn parse_frontmatter(content: &str) -> HashMap<String, String> {
    let mut fm: HashMap<String, String> = HashMap::new();
    // Match a leading `^---\n...\n---` block at the very start of the file.
    let trimmed_start = content.trim_start_matches('\u{feff}');
    let stripped = if let Some(after) = trimmed_start.strip_prefix("---\n") {
        after
    } else if let Some(after) = trimmed_start.strip_prefix("---\r\n") {
        after
    } else {
        return fm;
    };
    let end = stripped.find("\n---").or_else(|| stripped.find("\r\n---"));
    let body = match end {
        Some(i) => &stripped[..i],
        None => return fm,
    };
    for line in body.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        // Only handle top-level `key: value` lines — skip indented YAML.
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let mut parts = line.splitn(2, ':');
        let key = match parts.next() {
            Some(k) => k.trim(),
            None => continue,
        };
        let val = match parts.next() {
            Some(v) => v.trim(),
            None => continue,
        };
        // Strip surrounding quotes for the common `"foo"` and `'foo'` cases.
        let val = val.trim();
        let val = if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
            || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
        {
            &val[1..val.len() - 1]
        } else {
            val
        };
        if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            fm.insert(key.to_string(), val.to_string());
        }
    }
    fm
}

/// Decode a Claude Code encoded project directory name back to the real
/// project path. Strategy: read the first few `*.jsonl` session files
/// inside `~/.claude/projects/<encoded>/` and look for a `"cwd"` field.
/// When the session-cwd path does not exist on disk, fall back to a
/// greedy character-level DFS over `/` using the lossy encoding rules
/// (alphanumerics preserved, everything else replaced with `-`).
///
/// Returns `None` when no path can be resolved.
pub fn decode_project_dir_name(home: &Path, encoded_name: &str) -> Option<PathBuf> {
    let claude_dir = home.join(".claude");
    let project_dir = claude_dir.join("projects").join(encoded_name);

    // Strategy 1: Try to read cwd from a session.jsonl inside this dir.
    if let Some(resolved) = read_session_cwd(&project_dir) {
        if resolved.exists() {
            return Some(resolved);
        }
    }

    // Strategy 2: Greedy DFS using normalized segments (handles _/- collisions).
    if let Some(resolved) = decode_by_normalized_segments(encoded_name) {
        if resolved.exists() {
            return Some(resolved);
        }
    }

    // Strategy 3: Character-level fallback for unicode-encoded paths.
    if let Some(resolved) = decode_by_char_unicode(encoded_name) {
        if resolved.exists() {
            return Some(resolved);
        }
    }

    None
}

fn read_session_cwd(project_dir: &Path) -> Option<PathBuf> {
    let entries: Vec<_> = std::fs::read_dir(project_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().map(|t| t.is_file()).unwrap_or(false)
                && e.file_name().to_string_lossy().ends_with(".jsonl")
        })
        .map(|e| e.path())
        .collect();
    let mut jsonls = entries;
    jsonls.sort();
    jsonls.truncate(3);
    for path in jsonls {
        let content = std::fs::read_to_string(&path).ok()?;
        for line in content.lines().take(40) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(cwd) = value.get("cwd").and_then(|v| v.as_str()) {
                    if !cwd.is_empty() {
                        return Some(PathBuf::from(cwd));
                    }
                }
            }
        }
    }
    None
}

fn norm(s: &str) -> String {
    s.to_lowercase().replace('_', "-")
}

/// Greedy DFS over `/` matching normalized segment sequences against
/// real directory entries. `_` and `-` are treated equivalently.
/// Resolution walks from `/`, takes the longest matching slice of the
/// remaining normalized segments at every step, and falls back to
/// shorter slices (backtracking).
fn decode_by_normalized_segments(encoded: &str) -> Option<PathBuf> {
    let raw = encoded.strip_prefix('-').unwrap_or(encoded);
    if raw.is_empty() {
        return None;
    }
    let segs: Vec<&str> = raw.split('-').collect();
    let mut current = PathBuf::from("/");
    resolve_segments(&current, &segs, 0)
}

fn resolve_segments(current: &Path, segs: &[&str], i: usize) -> Option<PathBuf> {
    if i >= segs.len() {
        return if current.exists() { Some(current.to_path_buf()) } else { None };
    }
    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return None,
    };
    // Build map: normalized name → first actual name
    let mut entry_map: HashMap<String, String> = HashMap::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        let key = norm(&name);
        entry_map.entry(key).or_insert(name);
    }
    // Try longest match first, then fall back to shorter.
    for end in (i + 1..=segs.len()).rev() {
        let candidate = norm(&segs[i..end].join("-"));
        if let Some(name) = entry_map.get(&candidate) {
            let next = current.join(name);
            if let Some(found) = resolve_segments(&next, segs, end) {
                return Some(found);
            }
        }
    }
    None
}

/// Character-level decoding for Unicode-heavy encoded paths.
/// Each non-alphanumeric in the source folder name collapses to `-`.
fn decode_by_char_unicode(encoded: &str) -> Option<PathBuf> {
    let pattern = encoded.strip_prefix('-').unwrap_or(encoded).to_string();
    if pattern.is_empty() {
        return None;
    }
    let current = PathBuf::from("/");
    walk_char_unicode(&current, &pattern)
}

fn walk_char_unicode(current: &Path, pattern: &str) -> Option<PathBuf> {
    if pattern.is_empty() {
        return if current.exists() { Some(current.to_path_buf()) } else { None };
    }
    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return None,
    };
    let alnum = |c: char| c.is_ascii_alphanumeric();
    let mut candidates: Vec<(String, usize)> = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        // Try to match name against the leading chars of pattern.
        let pat_bytes = pattern.as_bytes();
        let name_bytes = name.as_bytes();
        if name_bytes.len() > pat_bytes.len() {
            continue;
        }
        let mut ok = true;
        for (i, nb) in name_bytes.iter().enumerate() {
            let pc = pat_bytes[i] as char;
            let nc = *nb as char;
            if pc == '-' {
                if alnum(nc) {
                    ok = false;
                    break;
                }
            } else if pc.to_ascii_lowercase() != nc.to_ascii_lowercase() {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        let next_pos = name_bytes.len();
        let advance = if next_pos == pat_bytes.len() {
            0
        } else if pat_bytes[next_pos] == b'-' {
            1
        } else {
            // Must continue with the next non-encoded separator.
            continue;
        };
        candidates.push((name, next_pos + advance));
    }
    // Prefer longest name first.
    candidates.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (name, next_pos) in candidates {
        let next_path = current.join(&name);
        let next_pattern = &pattern[next_pos..];
        if let Some(resolved) = walk_char_unicode(&next_path, next_pattern) {
            return Some(resolved);
        }
    }
    None
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

    // ── parse_frontmatter ──

    #[test]
    fn parse_frontmatter_basic_keys() {
        let md = "---\nname: brainstorming\ndescription: brainstorm help\n---\n# Body\n";
        let fm = parse_frontmatter(md);
        assert_eq!(fm.get("name").unwrap(), "brainstorming");
        assert_eq!(fm.get("description").unwrap(), "brainstorm help");
    }

    #[test]
    fn parse_frontmatter_missing_returns_empty() {
        let fm = parse_frontmatter("# Heading\nBody\n");
        assert!(fm.is_empty());
    }

    #[test]
    fn parse_frontmatter_strips_quotes() {
        let md = "---\nname: \"quoted-name\"\ntype: 'single'\n---\n";
        let fm = parse_frontmatter(md);
        assert_eq!(fm.get("name").unwrap(), "quoted-name");
        assert_eq!(fm.get("type").unwrap(), "single");
    }

    #[test]
    fn parse_frontmatter_handles_crlf() {
        let md = "---\r\nname: crlf\r\n---\r\nbody\r\n";
        let fm = parse_frontmatter(md);
        assert_eq!(fm.get("name").unwrap(), "crlf");
    }

    // ── decode_project_dir_name ──

    fn write_session_with_cwd(dir: &Path, cwd: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let line = format!("{{\"cwd\":\"{}\",\"type\":\"user\"}}\n", cwd);
        std::fs::write(dir.join("session.jsonl"), line).unwrap();
    }

    #[test]
    fn decode_resolves_via_session_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        // Real path lives outside of any encoded layout.
        let real_repo = home.join("work").join("ward-demo");
        std::fs::create_dir_all(&real_repo).unwrap();
        let encoded = "-work-ward-demo";
        let project_dir = home.join(".claude").join("projects").join(encoded);
        write_session_with_cwd(&project_dir, real_repo.to_str().unwrap());

        let resolved = decode_project_dir_name(home, encoded).expect("should resolve via session cwd");
        assert_eq!(resolved, real_repo);
    }

    #[test]
    fn decode_falls_back_to_normalized_segments() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        // Build a real path under `/tmp` so the segments-based decoder,
        // which walks from `/`, can reach it. Each segment must be a
        // clean alnum token (no underscores / hyphens inside the token).
        let pid_tag = format!("wdtest{}", std::process::id());
        let real_repo = PathBuf::from(format!("/tmp/warddecode-{}-Projects/RepoOne", pid_tag));
        let _ = std::fs::remove_dir_all(&real_repo);
        std::fs::create_dir_all(&real_repo).unwrap();
        let encoded = format!("-tmp-warddecode-{}-Projects-RepoOne", pid_tag);
        let project_dir = home.join(".claude").join("projects").join(&encoded);
        // No session.jsonl — force the segment-based fallback path.
        std::fs::create_dir_all(&project_dir).unwrap();

        let resolved = decode_project_dir_name(home, &encoded)
            .expect("should resolve via normalized segments");
        assert_eq!(resolved, real_repo);
        let _ = std::fs::remove_dir_all(&real_repo);
    }

    #[test]
    fn decode_returns_none_for_missing_target() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        // An encoded name with no matching dir on disk AND no session.jsonl cwd → None.
        let project_dir = home.join(".claude").join("projects").join("-missing-project");
        std::fs::create_dir_all(&project_dir).unwrap();
        assert!(decode_project_dir_name(home, "-missing-project").is_none());
    }
}
