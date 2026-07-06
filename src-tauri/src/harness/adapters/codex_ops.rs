//! codex_ops.rs — Move / delete / restore / save-file / MCP upsert for the
//! Codex adapter (Plan 20). Codex's write path is greenfield (CCO never wrote
//! Codex config), so this module is authored from scratch — matching CCO only
//! on the READ side (config.toml paths, `mcp_servers` key spelling).
//!
//! The single hard rule: `~/.codex/config.toml` edits MUST be **surgical and
//! format-preserving**. A `toml::Value` serialize round-trip would destroy
//! comments, reorder keys, and mangle the quoted-key / nested-sub-table shapes
//! the real config uses. Everything here goes through `toml_edit::DocumentMut`
//! so every other table, comment, and key survives, and only the one
//! `[mcp_servers.<name>]` sub-table we own is rewritten. Undo captures the
//! whole prior file bytes (mirrors `claude_mcp::set_policy`).

use std::path::Path;

use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

use crate::error::WardError;
use crate::model::RestoreInfo;

/// The Codex harness ops. Unit struct — all state comes from `Ctx`. The
/// `HarnessOps` implementation is wired in Task 2.
pub struct CodexOps;

// ── JSON → TOML conversion (inverse of `codex::toml_to_json`) ───────────

/// Convert a JSON scalar/array/object into a `toml_edit::Value` (used for
/// array elements and inline tables — anything that lives on the value side
/// of a key/value pair). Nested objects become **inline tables** here because
/// standard tables cannot appear inside an array or as an inline value.
/// Returns `None` for JSON `null` (TOML has no null) and for non-finite
/// numbers, so those keys are simply dropped.
fn json_to_toml_value(v: &serde_json::Value) -> Option<Value> {
    use serde_json::Value as J;
    match v {
        J::Null => None,
        J::Bool(b) => Some(Value::from(*b)),
        J::Number(n) => n
            .as_i64()
            .map(Value::from)
            .or_else(|| n.as_f64().map(Value::from)),
        J::String(s) => Some(Value::from(s.as_str())),
        J::Array(arr) => {
            let mut a = Array::new();
            for el in arr {
                if let Some(val) = json_to_toml_value(el) {
                    a.push(val);
                }
            }
            Some(Value::Array(a))
        }
        J::Object(map) => {
            let mut it = InlineTable::new();
            for (k, val) in map {
                if let Some(v) = json_to_toml_value(val) {
                    it.insert(k, v);
                }
            }
            Some(Value::InlineTable(it))
        }
    }
}

/// Convert a JSON object into a `toml_edit::Table` — the inverse of
/// `codex::toml_to_json`. Scalars and arrays become `Item::Value`; **nested
/// objects become standard `Item::Table` sub-tables** (e.g. `env`/`headers`
/// render as `[mcp_servers.<name>.env]`). `null` keys are skipped. A
/// non-object input yields an empty table.
pub fn json_to_toml_table(config: &serde_json::Value) -> Table {
    let mut table = Table::new();
    if let Some(map) = config.as_object() {
        for (k, v) in map {
            match v {
                serde_json::Value::Object(_) => {
                    table.insert(k, Item::Table(json_to_toml_table(v)));
                }
                serde_json::Value::Null => { /* TOML has no null — drop the key */ }
                _ => {
                    if let Some(val) = json_to_toml_value(v) {
                        table.insert(k, Item::Value(val));
                    }
                }
            }
        }
    }
    table
}

// ── Surgical MCP upsert (config.toml) ───────────────────────────────────

/// Ensure `mcp_servers` exists as a table and return a mutable handle to it.
/// A freshly-created parent is marked implicit so no bare `[mcp_servers]`
/// header is emitted (children render as `[mcp_servers.<name>]`). If the key
/// exists but is not a table, error rather than clobber it.
fn ensure_mcp_servers_table(doc: &mut DocumentMut) -> Result<&mut Table, WardError> {
    // Compute existence WITHOUT holding an immutable borrow across the mutate.
    let state: Option<bool> = doc.get("mcp_servers").map(|item| item.is_table());
    match state {
        None => {
            let mut t = Table::new();
            t.set_implicit(true);
            doc.insert("mcp_servers", Item::Table(t));
        }
        Some(true) => {}
        Some(false) => {
            return Err(WardError::NotFound(
                "mcp_servers exists in config.toml but is not a table".into(),
            ))
        }
    }
    doc.get_mut("mcp_servers")
        .and_then(|i| i.as_table_mut())
        .ok_or_else(|| WardError::NotFound("mcp_servers table missing after ensure".into()))
}

/// Surgically insert-or-overwrite `[mcp_servers.<name>]` in `target`
/// (config.toml), preserving all other tables, comments, and formatting.
/// `config` is the JSON server object (same shape as Claude): `{command, args,
/// env}` or `{url, headers, ...}`. Whole prior file bytes are captured for a
/// true undo (`None` when the file was absent/empty → undo removes the file).
pub fn upsert_mcp_entry_toml(
    target: &Path,
    name: &str,
    config: &serde_json::Value,
) -> Result<RestoreInfo, WardError> {
    let backup_bytes = std::fs::read(target).unwrap_or_default();
    let text = std::fs::read_to_string(target).unwrap_or_default();
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| WardError::NotFound(format!("parse toml {}: {e}", target.display())))?;
    let servers = ensure_mcp_servers_table(&mut doc)?;
    servers.insert(name, Item::Table(json_to_toml_table(config)));
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(target, doc.to_string())?;
    Ok(RestoreInfo {
        kind: "mcp-upsert".into(),
        original_path: target.display().to_string(),
        current_path: None,
        backup_bytes: if backup_bytes.is_empty() { None } else { Some(backup_bytes) },
        mcp_entry: None,
        mcp_key: Some(name.to_string()),
        mcp_parent_key: Some("mcp_servers".into()),
        mcp_scope: None,
    })
}

// ════════════════════════════════════════════════════════════════════════
// TESTS — Task 1: json_to_toml_table round-trip + surgical upsert goldens.
// ════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    /// Render a `toml_edit::Table` under `[mcp_servers.srv]`, re-parse it with
    /// the `toml` crate, and convert back to JSON via the SAME
    /// `codex::toml_to_json` that `json_to_toml_table` inverts. This proves the
    /// inverse property end-to-end (JSON → toml_edit → text → toml → JSON).
    fn round_trip(config: &serde_json::Value) -> serde_json::Value {
        let mut doc = DocumentMut::new();
        let mut servers = Table::new();
        servers.set_implicit(true);
        servers.insert("srv", Item::Table(json_to_toml_table(config)));
        doc.insert("mcp_servers", Item::Table(servers));
        let text = doc.to_string();
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        let srv = parsed
            .get("mcp_servers")
            .and_then(|v| v.get("srv"))
            .expect("srv table present after round-trip");
        crate::harness::adapters::codex::toml_to_json(srv)
    }

    #[test]
    fn json_to_toml_table_round_trips_stdio_shape() {
        let config = json!({
            "command": "npx",
            "args": ["-y", "@upstash/context7-mcp"],
            "env": { "API_KEY": "placeholder", "REGION": "us" }
        });
        assert_eq!(round_trip(&config), config);
    }

    #[test]
    fn json_to_toml_table_round_trips_remote_shape() {
        let config = json!({
            "url": "https://example.com/mcp",
            "headers": { "Authorization": "Bearer x", "X-Env": "prod" }
        });
        assert_eq!(round_trip(&config), config);
    }

    #[test]
    fn json_to_toml_table_round_trips_bool_and_nested() {
        let config = json!({
            "command": "node",
            "args": ["server.mjs"],
            "enabled": true,
            "env": { "DEBUG": "1" }
        });
        assert_eq!(round_trip(&config), config);
    }

    // ── Surgical upsert goldens ──

    const COMMENTED_CONFIG: &str = "\
# Codex global config — hand-written, comments must survive.
model = \"gpt-5.5\"
approval_policy = \"never\"

[mcp_servers.context7]
command = \"npx\"
args = [\"-y\", \"@upstash/context7-mcp\"]

# A server whose name needs a quoted key.
[mcp_servers.\"auggie-mcp\"]
command = \"auggie\"
args = [\"mcp\"]

[profiles.review]
sandbox_mode = \"read-only\"
";

    fn write_tmp(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn upsert_inserts_new_server_preserving_comments_tables_and_quoted_keys() {
        let (_d, path) = write_tmp(COMMENTED_CONFIG);
        let cfg = json!({ "command": "uvx", "args": ["my-mcp==1.2.3"] });
        let info = upsert_mcp_entry_toml(&path, "my-mcp", &cfg).unwrap();
        assert_eq!(info.kind, "mcp-upsert");
        assert!(info.backup_bytes.is_some(), "prior bytes captured for edit undo");

        let out = std::fs::read_to_string(&path).unwrap();
        // Comments preserved.
        assert!(out.contains("# Codex global config"), "top comment lost:\n{out}");
        assert!(out.contains("# A server whose name needs a quoted key."), "inline comment lost:\n{out}");
        // Untouched tables + quoted key preserved verbatim.
        assert!(out.contains("[mcp_servers.context7]"), "context7 lost:\n{out}");
        assert!(out.contains("[mcp_servers.\"auggie-mcp\"]"), "quoted key lost:\n{out}");
        assert!(out.contains("[profiles.review]"), "profiles table lost:\n{out}");
        assert!(out.contains("model = \"gpt-5.5\""), "top-level scalar lost:\n{out}");

        // New server is present and re-parses with the right values.
        let parsed: toml::Value = toml::from_str(&out).unwrap();
        let srv = parsed.get("mcp_servers").and_then(|v| v.get("my-mcp")).unwrap();
        assert_eq!(srv.get("command").unwrap().as_str(), Some("uvx"));
        // Every prior server still parses.
        assert!(parsed.get("mcp_servers").and_then(|v| v.get("context7")).is_some());
        assert!(parsed.get("mcp_servers").and_then(|v| v.get("auggie-mcp")).is_some());
    }

    #[test]
    fn upsert_overwrites_existing_server_and_keeps_others() {
        let (_d, path) = write_tmp(COMMENTED_CONFIG);
        let cfg = json!({ "command": "npx", "args": ["-y", "@upstash/context7-mcp@2.0.0"] });
        upsert_mcp_entry_toml(&path, "context7", &cfg).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        let parsed: toml::Value = toml::from_str(&out).unwrap();
        let srv = parsed.get("mcp_servers").and_then(|v| v.get("context7")).unwrap();
        let args: Vec<&str> = srv.get("args").unwrap().as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(args, vec!["-y", "@upstash/context7-mcp@2.0.0"]);
        // Sibling quoted-key server + comments untouched.
        assert!(out.contains("[mcp_servers.\"auggie-mcp\"]"));
        assert!(out.contains("# Codex global config"));
    }

    #[test]
    fn upsert_creates_file_when_absent_backup_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        assert!(!path.exists());
        let cfg = json!({ "url": "https://example.com/mcp" });
        let info = upsert_mcp_entry_toml(&path, "remote", &cfg).unwrap();
        assert!(info.backup_bytes.is_none(), "absent file → no backup (undo removes)");
        assert!(path.exists(), "parent dir created + file written");
        let out = std::fs::read_to_string(&path).unwrap();
        // Implicit parent → no bare [mcp_servers] header, only the child.
        assert!(!out.contains("[mcp_servers]\n"), "should not emit an empty parent header:\n{out}");
        assert!(out.contains("[mcp_servers.remote]"), "child header missing:\n{out}");
        let parsed: toml::Value = toml::from_str(&out).unwrap();
        assert_eq!(
            parsed.get("mcp_servers").and_then(|v| v.get("remote")).and_then(|v| v.get("url")).and_then(|v| v.as_str()),
            Some("https://example.com/mcp")
        );
    }

    #[test]
    fn upsert_edit_undo_is_byte_identical_via_restore_mcp_file() {
        // Editing an existing file: undo (restore_mcp_file with Some(bytes))
        // must reproduce the ORIGINAL file byte-for-byte.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let path = home.join(".codex").join("config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, COMMENTED_CONFIG).unwrap();
        let original = std::fs::read(&path).unwrap();

        let cfg = json!({ "command": "changed" });
        let info = upsert_mcp_entry_toml(&path, "context7", &cfg).unwrap();
        assert_ne!(std::fs::read(&path).unwrap(), original, "file changed by edit");

        crate::harness::adapters::claude_mcp::restore_mcp_file(home, &info).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original, "undo restores byte-identical original");
    }

    #[test]
    fn upsert_create_undo_removes_file_via_restore_mcp_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let path = home.join(".codex").join("config.toml");
        let cfg = json!({ "command": "node", "args": ["s.mjs"] });
        let info = upsert_mcp_entry_toml(&path, "srv", &cfg).unwrap();
        assert!(path.exists());
        crate::harness::adapters::claude_mcp::restore_mcp_file(home, &info).unwrap();
        assert!(!path.exists(), "undo of a create removes the file");
    }
}
