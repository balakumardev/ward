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

use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

use crate::error::WardError;
use crate::fs_utils::ensure_under_home;
use crate::harness::adapters::claude_ops::{delete_single_file, delete_skill_dir, restore_file};
use crate::harness::{Ctx, HarnessOps};
use crate::model::{Destination, HarnessItem, RestoreInfo, Scope};

/// The Codex harness ops. Unit struct — all state comes from `Ctx`.
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

/// Surgically remove `[mcp_servers.<name>]` from `target`, capturing the whole
/// prior file bytes for undo. Errors if the file or the entry is absent.
fn remove_mcp_entry_toml(target: &Path, name: &str) -> Result<RestoreInfo, WardError> {
    let backup_bytes = std::fs::read(target)
        .map_err(|_| WardError::NotFound(format!("config.toml not found: {}", target.display())))?;
    let text = String::from_utf8_lossy(&backup_bytes).into_owned();
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| WardError::NotFound(format!("parse toml {}: {e}", target.display())))?;
    let removed = doc
        .get_mut("mcp_servers")
        .and_then(|i| i.as_table_mut())
        .and_then(|t| t.remove(name));
    if removed.is_none() {
        return Err(WardError::NotFound(format!(
            "Server {name} not found in {}",
            target.display()
        )));
    }
    std::fs::write(target, doc.to_string())?;
    Ok(RestoreInfo {
        kind: "mcp-upsert".into(),
        original_path: target.display().to_string(),
        current_path: None,
        backup_bytes: Some(backup_bytes),
        mcp_entry: None,
        mcp_key: Some(name.to_string()),
        mcp_parent_key: Some("mcp_servers".into()),
        mcp_scope: None,
    })
}

// ── Target resolution ───────────────────────────────────────────────────

/// Resolve the `config.toml` write target for `(scope_id, scopes)`:
/// global → `~/.codex/config.toml`; project → `<repo>/.codex/config.toml`.
fn resolve_codex_config_toml(
    home: &Path,
    scope_id: &str,
    scopes: &[Scope],
) -> Result<PathBuf, WardError> {
    if scope_id == "global" {
        return Ok(home.join(".codex").join("config.toml"));
    }
    let scope = scopes
        .iter()
        .find(|s| s.id == scope_id)
        .ok_or_else(|| WardError::NotFound(format!("Unknown scope: {scope_id}")))?;
    Ok(PathBuf::from(&scope.root).join(".codex").join("config.toml"))
}

// ── HarnessOps impl ─────────────────────────────────────────────────────

impl HarnessOps for CodexOps {
    /// Codex config is single-file per scope; move stays a Claude capability.
    fn get_valid_destinations(&self, _ctx: &Ctx, _item: &HarnessItem, _scopes: &[Scope]) -> Vec<Destination> {
        Vec::new()
    }

    fn move_item(&self, _ctx: &Ctx, _item: &HarnessItem, _dest_scope_id: &str, _scopes: &[Scope])
        -> Result<RestoreInfo, WardError>
    {
        Err(WardError::NotFound("Codex does not support moving items".into()))
    }

    fn delete_item(&self, ctx: &Ctx, item: &HarnessItem, _scopes: &[Scope])
        -> Result<RestoreInfo, WardError>
    {
        if item.locked {
            return Err(WardError::NotFound(format!("{} is locked and cannot be deleted", item.name)));
        }
        match item.category.as_str() {
            // MCP delete = surgical TOML remove; whole-file backup for undo.
            "mcp" => {
                let target = ensure_under_home(Path::new(&item.path), ctx.home)?;
                remove_mcp_entry_toml(&target, &item.name)
            }
            // File-based categories reuse Claude's semantics verbatim.
            "memory" | "rule" => delete_single_file(ctx, item),
            "skill" => delete_skill_dir(ctx, item),
            other => Err(WardError::NotFound(format!("{other} items cannot be deleted"))),
        }
    }

    fn restore(&self, ctx: &Ctx, info: &RestoreInfo) -> Result<(), WardError> {
        match info.kind.as_str() {
            "file" => restore_file(ctx, info),
            "mcp-upsert" => crate::harness::adapters::claude_mcp::restore_mcp_file(ctx.home, info),
            "skill-create" => {
                let dir = ensure_under_home(Path::new(&info.original_path), ctx.home)?;
                if dir.exists() {
                    std::fs::remove_dir_all(&dir)?;
                }
                Ok(())
            }
            other => Err(WardError::NotFound(format!("Unknown restore kind: {other}"))),
        }
    }

    fn save_file(&self, ctx: &Ctx, path: &str, content: &str) -> Result<(), WardError> {
        let abs = ensure_under_home(Path::new(path), ctx.home)?;
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&abs, content)?;
        Ok(())
    }

    fn upsert_mcp_entry(&self, ctx: &Ctx, scope_id: &str, name: &str,
        config: &serde_json::Value, target_path: Option<&str>, scopes: &[Scope])
        -> Result<RestoreInfo, WardError>
    {
        let target = match target_path {
            // Edit existing: write that exact file (validated under home).
            Some(tp) => ensure_under_home(Path::new(tp), ctx.home)?,
            // Add new: resolve the scope's config.toml.
            None => {
                let p = resolve_codex_config_toml(ctx.home, scope_id, scopes)?;
                ensure_under_home(&p, ctx.home)?
            }
        };
        upsert_mcp_entry_toml(&target, name, config)
    }
}

// ════════════════════════════════════════════════════════════════════════
// TESTS — Task 1: json_to_toml_table round-trip + surgical upsert goldens.
//         Task 2: CodexOps HarnessOps (delete/restore/upsert/move/dest).
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

    // ── Task 2: CodexOps HarnessOps ──

    use crate::harness::adapters::codex::CodexAdapter;
    use crate::harness::Harness;
    use crate::model::Scope;

    fn ctx_home() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        (dir, home)
    }

    fn global_scope(home: &Path) -> Scope {
        Scope {
            id: "global".into(),
            kind: "global".into(),
            label: "Global (~/.codex)".into(),
            root: home.join(".codex").display().to_string(),
        }
    }

    fn mcp_item(path: &Path, name: &str) -> HarnessItem {
        HarnessItem {
            category: "mcp".into(),
            scope_id: "global".into(),
            name: name.into(),
            description: String::new(),
            path: path.display().to_string(),
            movable: false,
            deletable: true,
            locked: false,
            effective: None,
            mcp_config: None,
        }
    }

    #[test]
    fn resolve_config_toml_global_and_project() {
        let home = Path::new("/Users/testhome");
        let scopes = vec![
            Scope { id: "global".into(), kind: "global".into(),
                label: "Global".into(), root: "/Users/testhome/.codex".into() },
            Scope { id: "-proj".into(), kind: "project".into(),
                label: "proj".into(), root: "/work/proj".into() },
        ];
        let g = resolve_codex_config_toml(home, "global", &scopes).unwrap();
        assert!(g.ends_with(".codex/config.toml"));
        assert!(g.starts_with("/Users/testhome"));
        let p = resolve_codex_config_toml(home, "-proj", &scopes).unwrap();
        assert_eq!(p, PathBuf::from("/work/proj/.codex/config.toml"));
        assert!(resolve_codex_config_toml(home, "-nope", &scopes).is_err());
    }

    #[test]
    fn ops_upsert_add_is_visible_on_rescan() {
        // Write target must be a scanned file: upsert → re-scan → present.
        let (_d, home) = ctx_home();
        let cfg_path = home.join(".codex").join("config.toml");
        std::fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
        std::fs::write(&cfg_path, COMMENTED_CONFIG).unwrap();
        let ctx = Ctx { home: &home, cwd: None };
        let scopes = vec![global_scope(&home)];

        let cfg = json!({ "command": "node", "args": ["srv.mjs"], "env": { "K": "v" } });
        CodexOps
            .upsert_mcp_entry(&ctx, "global", "brand-new", &cfg, None, &scopes)
            .unwrap();

        let items = CodexAdapter.scan_category(&ctx, "mcp", &scopes[0]).unwrap();
        let found = items.iter().find(|i| i.name == "brand-new").expect("new server visible after re-scan");
        assert_eq!(found.mcp_config.as_ref().unwrap()["command"], "node");
        // Prior servers still present.
        assert!(items.iter().any(|i| i.name == "context7"));
        assert!(items.iter().any(|i| i.name == "auggie-mcp"));
    }

    #[test]
    fn ops_upsert_edit_via_target_path_overwrites_in_place() {
        let (_d, home) = ctx_home();
        let cfg_path = home.join(".codex").join("config.toml");
        std::fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
        std::fs::write(&cfg_path, COMMENTED_CONFIG).unwrap();
        let ctx = Ctx { home: &home, cwd: None };
        let scopes = vec![global_scope(&home)];

        let cfg = json!({ "command": "npx", "args": ["-y", "@upstash/context7-mcp@9.9.9"] });
        let info = CodexOps
            .upsert_mcp_entry(&ctx, "global", "context7", &cfg,
                Some(&cfg_path.display().to_string()), &scopes)
            .unwrap();
        assert_eq!(info.kind, "mcp-upsert");
        let items = CodexAdapter.scan_category(&ctx, "mcp", &scopes[0]).unwrap();
        let c7 = items.iter().find(|i| i.name == "context7").unwrap();
        let args = c7.mcp_config.as_ref().unwrap()["args"].as_array().unwrap();
        assert_eq!(args.last().unwrap(), "@upstash/context7-mcp@9.9.9");
    }

    #[test]
    fn ops_upsert_project_scope_writes_repo_config() {
        let (_d, home) = ctx_home();
        let repo = home.join("work").join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let ctx = Ctx { home: &home, cwd: None };
        let scopes = vec![
            global_scope(&home),
            Scope { id: "-repo".into(), kind: "project".into(),
                label: "repo".into(), root: repo.display().to_string() },
        ];
        let cfg = json!({ "command": "node", "args": ["s.mjs"] });
        CodexOps
            .upsert_mcp_entry(&ctx, "-repo", "repo_mcp", &cfg, None, &scopes)
            .unwrap();
        let written = repo.join(".codex").join("config.toml");
        assert!(written.exists(), "project config.toml created under repo");
        let out = std::fs::read_to_string(&written).unwrap();
        assert!(out.contains("[mcp_servers.repo_mcp]"));
    }

    #[test]
    fn ops_delete_and_restore_mcp_preserves_other_tables() {
        let (_d, home) = ctx_home();
        let cfg_path = home.join(".codex").join("config.toml");
        std::fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
        std::fs::write(&cfg_path, COMMENTED_CONFIG).unwrap();
        let original = std::fs::read(&cfg_path).unwrap();
        let ctx = Ctx { home: &home, cwd: None };

        let item = mcp_item(&cfg_path, "context7");
        let info = CodexOps.delete_item(&ctx, &item, &[]).unwrap();
        assert_eq!(info.kind, "mcp-upsert");
        let after = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(!after.contains("[mcp_servers.context7]"), "deleted server gone:\n{after}");
        assert!(after.contains("[mcp_servers.\"auggie-mcp\"]"), "sibling preserved");
        assert!(after.contains("[profiles.review]"), "unrelated table preserved");

        // Undo brings the whole file back byte-identically.
        CodexOps.restore(&ctx, &info).unwrap();
        assert_eq!(std::fs::read(&cfg_path).unwrap(), original, "restore is byte-identical");
    }

    #[test]
    fn ops_delete_and_restore_skill_dir() {
        let (_d, home) = ctx_home();
        let skill_dir = home.join(".codex").join("skills").join("demo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Demo\n\nbody\n").unwrap();
        std::fs::write(skill_dir.join("extra.txt"), "aux").unwrap();
        let ctx = Ctx { home: &home, cwd: None };

        let item = HarnessItem {
            category: "skill".into(), scope_id: "global".into(),
            name: "demo".into(), description: String::new(),
            path: skill_dir.display().to_string(),
            movable: false, deletable: true, locked: false,
            effective: None, mcp_config: None,
        };
        let info = CodexOps.delete_item(&ctx, &item, &[]).unwrap();
        assert_eq!(info.kind, "file");
        assert!(!skill_dir.exists(), "skill dir removed");

        CodexOps.restore(&ctx, &info).unwrap();
        assert!(skill_dir.join("SKILL.md").exists(), "SKILL.md restored");
        assert_eq!(std::fs::read_to_string(skill_dir.join("extra.txt")).unwrap(), "aux");
    }

    #[test]
    fn ops_delete_and_restore_memory_file() {
        let (_d, home) = ctx_home();
        let mem = home.join(".codex").join("memories").join("note.md");
        std::fs::create_dir_all(mem.parent().unwrap()).unwrap();
        std::fs::write(&mem, "remember this").unwrap();
        let ctx = Ctx { home: &home, cwd: None };
        let item = HarnessItem {
            category: "memory".into(), scope_id: "global".into(),
            name: "note".into(), description: String::new(),
            path: mem.display().to_string(),
            movable: false, deletable: true, locked: false,
            effective: None, mcp_config: None,
        };
        let info = CodexOps.delete_item(&ctx, &item, &[]).unwrap();
        assert!(!mem.exists());
        CodexOps.restore(&ctx, &info).unwrap();
        assert_eq!(std::fs::read_to_string(&mem).unwrap(), "remember this");
    }

    #[test]
    fn ops_delete_rejects_locked_and_unsupported_category() {
        let (_d, home) = ctx_home();
        let ctx = Ctx { home: &home, cwd: None };
        let mut locked = mcp_item(&home.join(".codex").join("config.toml"), "x");
        locked.locked = true;
        assert!(CodexOps.delete_item(&ctx, &locked, &[]).is_err(), "locked rejected");

        let mut cfg = mcp_item(&home.join(".codex").join("config.toml"), "x");
        cfg.category = "config".into();
        cfg.locked = false;
        assert!(CodexOps.delete_item(&ctx, &cfg, &[]).is_err(), "non-deletable category rejected");
    }

    #[test]
    fn ops_move_is_unsupported_and_destinations_empty() {
        let (_d, home) = ctx_home();
        let ctx = Ctx { home: &home, cwd: None };
        let item = mcp_item(&home.join(".codex").join("config.toml"), "x");
        assert!(CodexOps.move_item(&ctx, &item, "global", &[]).is_err());
        assert!(CodexOps.get_valid_destinations(&ctx, &item, &[]).is_empty());
    }

    #[test]
    fn ops_restore_unknown_kind_errors() {
        let (_d, home) = ctx_home();
        let ctx = Ctx { home: &home, cwd: None };
        let info = RestoreInfo {
            kind: "mystery".into(), original_path: home.display().to_string(),
            current_path: None, backup_bytes: None, mcp_entry: None,
            mcp_key: None, mcp_parent_key: None, mcp_scope: None,
        };
        assert!(CodexOps.restore(&ctx, &info).is_err());
    }

    #[test]
    fn ops_restore_skill_create_removes_dir() {
        let (_d, home) = ctx_home();
        let created = home.join(".codex").join("skills").join("fresh");
        std::fs::create_dir_all(&created).unwrap();
        std::fs::write(created.join("SKILL.md"), "x").unwrap();
        let ctx = Ctx { home: &home, cwd: None };
        let info = RestoreInfo {
            kind: "skill-create".into(),
            original_path: created.display().to_string(),
            current_path: None, backup_bytes: None, mcp_entry: None,
            mcp_key: None, mcp_parent_key: None, mcp_scope: None,
        };
        CodexOps.restore(&ctx, &info).unwrap();
        assert!(!created.exists(), "skill-create undo removes the created dir");
    }

    #[test]
    fn ops_save_file_writes_under_home_and_rejects_traversal() {
        let (_d, home) = ctx_home();
        let ctx = Ctx { home: &home, cwd: None };
        let target = home.join(".codex").join("AGENTS.md");
        CodexOps.save_file(&ctx, &target.display().to_string(), "# Agents\n").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "# Agents\n");
        // Traversal outside home is rejected.
        let bad = home.join("..").join("evil.md");
        assert!(CodexOps.save_file(&ctx, &bad.display().to_string(), "x").is_err());
    }
}
