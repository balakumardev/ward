//! settings/write.rs — Plan 29 Task 5: the surgical settings WRITER.
//!
//! Edits exactly ONE setting key in `~/.claude/settings.json` (or
//! `~/.claude.json`) while preserving every other key, with a byte-exact undo
//! captured in `RestoreInfo`. This is a direct port of the proven
//! `claude_mcp::set_policy` round-trip pattern (read prior bytes → parse →
//! ensure root object → mutate one key → write pretty → stash prior bytes).
//!
//! Scope: **user scope only.** Ward's Settings mode has no single "current
//! project" context, so project/local writes (which need a project picker) are
//! a documented follow-up. `managed` settings are read-only by design.

use std::path::Path;

use serde_json::{json, Value};

use crate::error::WardError;
use crate::harness::adapters::claude_mcp::write_json_pretty;
use crate::model::RestoreInfo;

/// Resolve the destination file for a user-scope write. `"claudeJson"` routes
/// to `~/.claude.json` (the global-config class); anything else — i.e.
/// `"settings.json"` — routes to `~/.claude/settings.json`.
fn user_target_path(home: &Path, target_file: &str) -> std::path::PathBuf {
    if target_file == "claudeJson" {
        home.join(".claude.json")
    } else {
        home.join(".claude").join("settings.json")
    }
}

/// Guard the write scope. Only `"user"` is writable today:
///   - `"managed"` → read-only (managed-settings.json is admin-owned).
///   - `"project"` / `"local"` → not yet supported (need a project picker).
///   - anything else → an outright error.
fn guard_scope(scope: &str) -> Result<(), WardError> {
    match scope {
        "user" => Ok(()),
        "managed" => Err(WardError::Settings(
            "managed settings are read-only".into(),
        )),
        "project" | "local" => Err(WardError::Settings(
            "project/local settings scope is not yet supported — user scope only".into(),
        )),
        other => Err(WardError::Settings(format!(
            "unknown settings scope '{other}'"
        ))),
    }
}

/// Read the target file's prior bytes and parse them into a root JSON object.
/// Missing/empty file → `{}`; a parse error surfaces as `WardError::Settings`.
/// Returns `(root_object_value, prior_bytes)` where `prior_bytes` is empty
/// when the file did not exist.
fn read_root(p: &Path) -> Result<(Value, Vec<u8>), WardError> {
    let backup_bytes = std::fs::read(p).unwrap_or_default();
    let mut root: Value = if backup_bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&backup_bytes)
            .map_err(|e| WardError::Settings(format!("parse {}: {e}", p.display())))?
    };
    if !root.is_object() {
        root = json!({});
    }
    Ok((root, backup_bytes))
}

/// Write `root` back to `p` (creating the parent dir) and build the
/// `"setting-write"` `RestoreInfo` capturing the prior bytes for undo.
fn write_root(
    p: &Path,
    root: &Value,
    key: &str,
    backup_bytes: Vec<u8>,
) -> Result<RestoreInfo, WardError> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_json_pretty(p, root)?;
    Ok(RestoreInfo {
        kind: "setting-write".into(),
        original_path: p.display().to_string(),
        current_path: None,
        backup_bytes: if backup_bytes.is_empty() {
            None
        } else {
            Some(backup_bytes)
        },
        mcp_entry: None,
        mcp_key: Some(key.to_string()),
        mcp_parent_key: None,
        mcp_scope: None,
    })
}

/// Set `root[key] = value` in the (scope, target_file) file, preserving every
/// other key. `value` is the WHOLE value for that key — for an object-typed
/// setting like `permissions`, pass the entire object. Returns a `RestoreInfo`
/// (kind `"setting-write"`) whose `backup_bytes` holds the prior file bytes
/// verbatim, so the Organizer/Settings undo is byte-exact.
pub fn set_setting(
    home: &Path,
    scope: &str,
    key: &str,
    target_file: &str,
    value: Value,
) -> Result<RestoreInfo, WardError> {
    guard_scope(scope)?;
    let p = user_target_path(home, target_file);
    let (mut root, backup_bytes) = read_root(&p)?;
    root.as_object_mut()
        .expect("read_root guarantees an object")
        .insert(key.to_string(), value);
    write_root(&p, &root, key, backup_bytes)
}

/// Remove `key` from the (scope, target_file) file's root object, preserving
/// every other key. This is the empty-remove path (mirrors `set_policy`
/// dropping an empty allow/deny list) — it NEVER writes `null` or `[]`. When
/// the file or key is absent it is still a success and still returns a
/// `RestoreInfo` capturing the prior bytes, so undo stays consistent (a
/// `None` backup means undo removes the file the write may have created).
pub fn unset_setting(
    home: &Path,
    scope: &str,
    key: &str,
    target_file: &str,
) -> Result<RestoreInfo, WardError> {
    guard_scope(scope)?;
    let p = user_target_path(home, target_file);
    let (mut root, backup_bytes) = read_root(&p)?;
    root.as_object_mut()
        .expect("read_root guarantees an object")
        .remove(key);
    write_root(&p, &root, key, backup_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_home() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().to_path_buf();
        (dir, p)
    }

    /// Convenience: restore via the same entry point the UI undo uses.
    fn restore(home: &Path, info: &RestoreInfo) {
        crate::harness::adapters::claude_mcp::restore_mcp_file(home, info).unwrap();
    }

    #[test]
    fn set_preserves_siblings_and_undo_byte_exact() {
        let (_d, home) = temp_home();
        let settings = home.join(".claude/settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let original = "{\n  \"theme\": \"dark\",\n  \"verbose\": true\n}\n";
        fs::write(&settings, original).unwrap();

        let info = set_setting(&home, "user", "theme", "settings.json", json!("light")).unwrap();

        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["verbose"], true, "sibling key preserved");
        assert_eq!(after["theme"], "light", "target key updated");
        assert_eq!(info.kind, "setting-write");
        assert_eq!(info.mcp_key.as_deref(), Some("theme"));
        assert!(info.backup_bytes.is_some(), "prior bytes captured for undo");

        // Undo must yield byte-identical content.
        restore(&home, &info);
        let restored = fs::read_to_string(&settings).unwrap();
        assert_eq!(restored, original, "restore must be byte-exact");
    }

    #[test]
    fn unset_removes_key_not_null() {
        let (_d, home) = temp_home();
        let settings = home.join(".claude/settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(&settings, r#"{"theme":"dark","verbose":true}"#).unwrap();

        let info = unset_setting(&home, "user", "verbose", "settings.json").unwrap();

        let raw = fs::read_to_string(&settings).unwrap();
        let after: Value = serde_json::from_str(&raw).unwrap();
        assert!(
            after.get("verbose").is_none(),
            "the key must be ABSENT, not nulled: {raw}"
        );
        assert!(!raw.contains("null"), "must never write a null value: {raw}");
        assert_eq!(after["theme"], "dark", "sibling key preserved");
        assert_eq!(info.kind, "setting-write");
        assert_eq!(info.mcp_key.as_deref(), Some("verbose"));
    }

    #[test]
    fn set_writes_object_value_whole() {
        let (_d, home) = temp_home();
        let obj = json!({ "defaultMode": "plan", "deny": ["Bash(rm *)"] });
        set_setting(&home, "user", "permissions", "settings.json", obj.clone()).unwrap();

        let settings = home.join(".claude/settings.json");
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["permissions"], obj, "the whole object landed intact");
        assert_eq!(after["permissions"]["defaultMode"], "plan");
        assert_eq!(after["permissions"]["deny"][0], "Bash(rm *)");
    }

    #[test]
    fn managed_scope_refused() {
        let (_d, home) = temp_home();
        let err =
            set_setting(&home, "managed", "theme", "settings.json", json!("light")).unwrap_err();
        assert!(
            matches!(err, WardError::Settings(_)),
            "managed scope must be refused with WardError::Settings"
        );
        if let WardError::Settings(m) = err {
            assert!(m.contains("read-only"), "message explains read-only: {m}");
        }
        // Nothing was written.
        assert!(!home.join(".claude/settings.json").exists());
    }

    #[test]
    fn claudejson_target_routes_to_claude_json() {
        let (_d, home) = temp_home();
        set_setting(&home, "user", "autoConnectIde", "claudeJson", json!(true)).unwrap();

        let claude_json = home.join(".claude.json");
        let settings = home.join(".claude/settings.json");
        let after: Value = serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
        assert_eq!(after["autoConnectIde"], true, "value written to ~/.claude.json");
        assert!(
            !settings.exists(),
            "settings.json must be untouched when target is claudeJson"
        );
    }

    #[test]
    fn project_scope_refused() {
        let (_d, home) = temp_home();
        let err =
            set_setting(&home, "project", "theme", "settings.json", json!("light")).unwrap_err();
        assert!(
            matches!(err, WardError::Settings(_)),
            "project scope must be refused (defensive)"
        );
        let err2 =
            set_setting(&home, "local", "theme", "settings.json", json!("light")).unwrap_err();
        assert!(
            matches!(err2, WardError::Settings(_)),
            "local scope must be refused (defensive)"
        );
    }

    // ── Extra coverage (Ward: comprehensive over fast) ──

    #[test]
    fn set_creates_file_when_missing_and_undo_removes_it() {
        let (_d, home) = temp_home();
        let settings = home.join(".claude/settings.json");
        assert!(!settings.exists());

        let info = set_setting(&home, "user", "theme", "settings.json", json!("light")).unwrap();
        assert!(settings.exists(), "writer creates the file + parent dir");
        assert!(info.backup_bytes.is_none(), "no prior file → no backup");
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["theme"], "light");

        // Undo of a creation removes the file.
        restore(&home, &info);
        assert!(!settings.exists(), "undo of a creation removes the file");
    }

    #[test]
    fn unset_missing_file_is_noop_success() {
        let (_d, home) = temp_home();
        let info = unset_setting(&home, "user", "verbose", "settings.json").unwrap();
        assert_eq!(info.kind, "setting-write");
        assert!(info.backup_bytes.is_none(), "no prior file → no backup");
    }

    #[test]
    fn unset_undo_is_byte_exact() {
        let (_d, home) = temp_home();
        let settings = home.join(".claude/settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let original = "{\n  \"theme\": \"dark\",\n  \"verbose\": true\n}\n";
        fs::write(&settings, original).unwrap();

        let info = unset_setting(&home, "user", "verbose", "settings.json").unwrap();
        restore(&home, &info);
        assert_eq!(
            fs::read_to_string(&settings).unwrap(),
            original,
            "unset undo must be byte-exact"
        );
    }

    #[test]
    fn set_preserves_unrelated_keys_in_claude_json() {
        let (_d, home) = temp_home();
        let claude_json = home.join(".claude.json");
        fs::write(
            &claude_json,
            r#"{"numStartups":9,"projects":{"/a":{"x":1}}}"#,
        )
        .unwrap();

        set_setting(&home, "user", "autoConnectIde", "claudeJson", json!(false)).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
        assert_eq!(after["numStartups"], 9, "unrelated top-level key preserved");
        assert_eq!(after["projects"]["/a"]["x"], 1, "nested state preserved");
        assert_eq!(after["autoConnectIde"], false, "new key written");
    }

    #[test]
    fn unknown_scope_refused() {
        let (_d, home) = temp_home();
        let err = set_setting(&home, "banana", "theme", "settings.json", json!("x")).unwrap_err();
        assert!(matches!(err, WardError::Settings(_)));
    }
}
