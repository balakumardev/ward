//! plugins/enable.rs — enable/disable an installed Claude Code plugin as a
//! surgical single-key flip of `~/.claude/settings.json`.
//!
//! Claude Code records a plugin's on/off state under
//! `settings.json → enabledPlugins["<name>@<marketplace>"]: bool`. Toggling
//! it in Ward must NOT rewrite the whole file — the user's `settings.json`
//! carries unrelated keys (theme, permissions, hooks, MCP policy, …) that a
//! naïve whole-file save would clobber. So this is a direct port of the
//! `claude_mcp::set_policy` write engine: read the prior bytes, parse to a
//! `serde_json::Value`, mutate ONLY the target entry in `enabledPlugins`
//! (creating the object when absent), and write the rest back untouched.
//! The returned `RestoreInfo` stashes the prior file bytes verbatim so the
//! Organizer's Undo restores byte-for-byte via `claude_mcp::restore_mcp_file`.
//!
//! One deliberate difference from `set_policy`: there is **no "empty ⇒ remove
//! the key" case** here. Disabling writes an explicit `false`; it does not
//! delete the entry. `enabledPlugins["x"] = false` and the key being absent
//! are distinct states in Claude Code (absent = "never installed/toggled"),
//! so Ward always writes the explicit boolean the user asked for.

use std::path::Path;

use serde_json::{json, Value};

use crate::error::WardError;
use crate::harness::adapters::claude_mcp;
use crate::model::RestoreInfo;

/// Set `enabledPlugins[plugin_key] = enabled` in `~/.claude/settings.json`,
/// preserving every other key and every other plugin entry. Returns a
/// `RestoreInfo` whose `backup_bytes` holds the prior file bytes (or `None`
/// when the file didn't exist) so the change can be undone byte-for-byte.
pub fn set_plugin_enabled(home: &Path, plugin_key: &str, enabled: bool)
    -> Result<RestoreInfo, WardError>
{
    let p = claude_mcp::policy_settings_path(home);
    let backup_bytes = std::fs::read(&p).unwrap_or_default();
    let mut root: Value = if backup_bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&backup_bytes)
            .map_err(|e| WardError::NotFound(format!("parse {}: {e}", p.display())))?
    };
    if !root.is_object() { root = json!({}); }
    let obj = root.as_object_mut().unwrap();
    let plugins = obj
        .entry("enabledPlugins".to_string())
        .or_insert_with(|| json!({}));
    if !plugins.is_object() { *plugins = json!({}); }
    // Always write the explicit boolean — disabling records `false`, it does
    // NOT delete the key (unlike set_policy's empty-list ⇒ key-absence case).
    plugins.as_object_mut().unwrap()
        .insert(plugin_key.to_string(), json!(enabled));

    if let Some(parent) = p.parent() { std::fs::create_dir_all(parent)?; }
    claude_mcp::write_json_pretty(&p, &root)?;
    Ok(RestoreInfo {
        kind: "plugin-enable".into(),
        original_path: p.display().to_string(),
        current_path: None,
        backup_bytes: if backup_bytes.is_empty() { None } else { Some(backup_bytes) },
        mcp_entry: None,
        mcp_key: Some(plugin_key.to_string()),
        mcp_parent_key: Some("enabledPlugins".into()),
        mcp_scope: None,
    })
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

    fn seed(home: &Path, content: &str) -> PathBuf {
        let settings = home.join(".claude").join("settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(&settings, content).unwrap();
        settings
    }

    #[test]
    fn set_enabled_flips_and_preserves_siblings() {
        let (_d, home) = temp_home();
        let settings = seed(&home, r#"{"theme":"dark","enabledPlugins":{"x@m":true}}"#);
        let info = set_plugin_enabled(&home, "x@m", false).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        // Sibling key untouched.
        assert_eq!(after["theme"], "dark");
        // Target flipped to explicit false (not removed).
        assert_eq!(after["enabledPlugins"]["x@m"], json!(false));
        assert!(after["enabledPlugins"].as_object().unwrap().contains_key("x@m"),
            "disabling must keep the key with value false, not delete it");
        assert_eq!(info.kind, "plugin-enable");
        assert_eq!(info.mcp_key.as_deref(), Some("x@m"));
        assert_eq!(info.mcp_parent_key.as_deref(), Some("enabledPlugins"));
        assert!(info.backup_bytes.is_some());
    }

    #[test]
    fn set_enabled_creates_object_when_absent() {
        let (_d, home) = temp_home();
        let settings = seed(&home, r#"{"theme":"dark"}"#);
        set_plugin_enabled(&home, "a@b", true).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["enabledPlugins"]["a@b"], json!(true));
        assert_eq!(after["theme"], "dark");
    }

    #[test]
    fn restore_reverts_to_prior_bytes() {
        let (_d, home) = temp_home();
        // Realistic pretty-printed file with trailing newline.
        let original = "{\n  \"theme\": \"dark\",\n  \"enabledPlugins\": {\n    \"x@m\": true\n  }\n}\n";
        let settings = seed(&home, original);
        let info = set_plugin_enabled(&home, "x@m", false).unwrap();
        // File was actually mutated.
        let mutated = fs::read_to_string(&settings).unwrap();
        assert_ne!(mutated, original, "set must have changed the file");
        // Undo restores byte-for-byte.
        claude_mcp::restore_mcp_file(&home, &info).unwrap();
        let after = fs::read_to_string(&settings).unwrap();
        assert_eq!(after, original, "restore must yield byte-identical content");
    }

    #[test]
    fn set_enabled_preserves_other_plugin_entries() {
        let (_d, home) = temp_home();
        let settings = seed(&home,
            r#"{"enabledPlugins":{"keep@m":true,"flip@m":true}}"#);
        set_plugin_enabled(&home, "flip@m", false).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["enabledPlugins"]["keep@m"], json!(true));
        assert_eq!(after["enabledPlugins"]["flip@m"], json!(false));
    }

    #[test]
    fn set_enabled_creates_file_when_missing() {
        let (_d, home) = temp_home();
        let settings = home.join(".claude").join("settings.json");
        assert!(!settings.exists());
        let info = set_plugin_enabled(&home, "a@b", true).unwrap();
        assert!(settings.exists());
        assert!(info.backup_bytes.is_none(), "no prior file → no backup bytes");
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["enabledPlugins"]["a@b"], json!(true));
        // Undo removes the file Ward created.
        claude_mcp::restore_mcp_file(&home, &info).unwrap();
        assert!(!settings.exists());
    }
}
