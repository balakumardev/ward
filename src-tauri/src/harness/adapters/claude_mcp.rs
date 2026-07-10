//! claude_mcp.rs — MCP policy + enable/disable controls for the Claude
//! adapter. Port of CCO's `getDisabledMcpServers` / `setDisabledMcpServers`
//! and `scanMcpPolicy` / `checkMcpPolicy` to Rust.
//!
//! Storage layout (CCO parity, verified by reading CCO):
//!   - Disabled list (per project):
//!     `~/.claude.json → projects[<projectPath>].disabledMcpServers: [String]`
//!   - Policy (user scope):
//!     `~/.claude/settings.json → allowedMcpServers / deniedMcpServers: [entry]`
//!
//! Disabled is per-project (mirrors `/mcp disable <name>` in Claude Code).
//! Policy is user scope (mirrors `isMcpServerAllowedByPolicy` in cc-src).
//!
//! Round-trip preservation: every setter reads the existing file,
//! mutates only the relevant field(s), and writes the rest of the
//! JSON back untouched. `set_disabled_servers` and `set_policy` both
//! return a `RestoreInfo` whose `backup_bytes` holds the prior file
//! bytes verbatim, so the Organizer can offer a true undo.

use std::path::Path;

use serde_json::{json, Value};

use crate::error::WardError;
use crate::model::{McpPolicy, PolicyEntry, PolicyVerdict, RestoreInfo};

// ── File path resolvers ────────────────────────────────────────────────

/// `~/.claude.json` — the file that holds per-project state including
/// `disabledMcpServers`.
fn claude_json_path(home: &Path) -> std::path::PathBuf { home.join(".claude.json") }

/// `~/.claude/settings.json` — the user-scope settings file where CCO
/// writes `allowedMcpServers` / `deniedMcpServers`. CCO's `scanMcpPolicy`
/// reads three files (settings.json, settings.local.json,
/// managed-settings.json) but its POST endpoint writes only
/// `settings.json`. We follow the write path: policy goes here.
pub(crate) fn policy_settings_path(home: &Path) -> std::path::PathBuf { home.join(".claude").join("settings.json") }

// ── Disabled list (per project) ────────────────────────────────────────

/// Read `projects[<projectPath>].disabledMcpServers` from
/// `~/.claude.json`. Missing file / missing key / wrong type → empty Vec.
pub fn get_disabled_servers(home: &Path, project_path: &Path)
    -> Result<Vec<String>, WardError>
{
    let p = claude_json_path(home);
    let content = match std::fs::read_to_string(&p) {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()),
    };
    let root: Value = serde_json::from_str(&content)
        .map_err(|e| WardError::NotFound(format!("parse {}: {e}", p.display())))?;
    let key = project_path.to_string_lossy().to_string();
    let list = root
        .get("projects")
        .and_then(|v| v.get(&key))
        .and_then(|v| v.get("disabledMcpServers"))
        .and_then(|v| v.as_array());
    Ok(list.map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default())
}

/// Write `projects[<projectPath>].disabledMcpServers = list` to
/// `~/.claude.json`. Preserves all other keys. Returns a `RestoreInfo`
/// whose `backup_bytes` is the original file bytes verbatim (or an
/// empty Vec when the file didn't exist) so the UI can offer undo.
pub fn set_disabled_servers(home: &Path, project_path: &Path, list: &[String])
    -> Result<RestoreInfo, WardError>
{
    let p = claude_json_path(home);
    let backup_bytes = std::fs::read(&p).unwrap_or_default();
    let mut root: Value = if backup_bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&backup_bytes)
            .map_err(|e| WardError::NotFound(format!("parse {}: {e}", p.display())))?
    };
    if !root.is_object() { root = json!({}); }
    let root_obj = root.as_object_mut().unwrap();
    let projects = root_obj
        .entry("projects".to_string())
        .or_insert_with(|| json!({}));
    if !projects.is_object() { *projects = json!({}); }
    let proj_obj = projects.as_object_mut().unwrap();
    let key = project_path.to_string_lossy().to_string();
    let proj_entry = proj_obj
        .entry(key.clone())
        .or_insert_with(|| json!({}));
    if !proj_entry.is_object() { *proj_entry = json!({}); }
    proj_entry.as_object_mut().unwrap()
        .insert("disabledMcpServers".to_string(), json!(list));

    write_json_pretty(&p, &root)?;
    Ok(RestoreInfo {
        kind: "mcp-disabled".into(),
        original_path: p.display().to_string(),
        current_path: None,
        backup_bytes: if backup_bytes.is_empty() { None } else { Some(backup_bytes) },
        mcp_entry: None,
        mcp_key: Some(key),
        mcp_parent_key: Some("projects".into()),
        mcp_scope: None,
    })
}

// ── Policy (user scope) ────────────────────────────────────────────────

/// Read the user-scope MCP policy from `~/.claude/settings.json`.
/// Returns empty allowlist + denylist when the file or keys are
/// absent. The function only reads `settings.json` (the file CCO's
/// POST endpoint writes to).
pub fn get_policy(home: &Path) -> Result<McpPolicy, WardError> {
    let p = policy_settings_path(home);
    let content = match std::fs::read_to_string(&p) {
        Ok(c) => c,
        Err(_) => return Ok(McpPolicy::default()),
    };
    let root: Value = serde_json::from_str(&content)
        .map_err(|e| WardError::NotFound(format!("parse {}: {e}", p.display())))?;
    Ok(McpPolicy {
        allowlist: parse_policy_entries(root.get("allowedMcpServers")),
        denylist: parse_policy_entries(root.get("deniedMcpServers")),
    })
}

/// Write `allowedMcpServers` and `deniedMcpServers` into
/// `~/.claude/settings.json`. Preserves all other keys. Returns a
/// `RestoreInfo` capturing the prior file bytes for undo.
pub fn set_policy(home: &Path, policy: &McpPolicy) -> Result<RestoreInfo, WardError> {
    let p = policy_settings_path(home);
    let backup_bytes = std::fs::read(&p).unwrap_or_default();
    let mut root: Value = if backup_bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&backup_bytes)
            .map_err(|e| WardError::NotFound(format!("parse {}: {e}", p.display())))?
    };
    if !root.is_object() { root = json!({}); }
    let obj = root.as_object_mut().unwrap();
    // Empty allowlist/denylist mean "no policy". Claude Code reads
    // `allowedMcpServers: []` as an allowlist that permits NOTHING — it blocks
    // every MCP server (global and project). So an empty policy MUST be written
    // as absence of the key, never as `[]`. Remove when empty; write when set.
    if policy.allowlist.is_empty() {
        obj.remove("allowedMcpServers");
    } else {
        obj.insert("allowedMcpServers".to_string(), json!(policy.allowlist));
    }
    if policy.denylist.is_empty() {
        obj.remove("deniedMcpServers");
    } else {
        obj.insert("deniedMcpServers".to_string(), json!(policy.denylist));
    }

    if let Some(parent) = p.parent() { std::fs::create_dir_all(parent)?; }
    write_json_pretty(&p, &root)?;
    Ok(RestoreInfo {
        kind: "mcp-policy".into(),
        original_path: p.display().to_string(),
        current_path: None,
        backup_bytes: if backup_bytes.is_empty() { None } else { Some(backup_bytes) },
        mcp_entry: None,
        mcp_key: None,
        mcp_parent_key: None,
        mcp_scope: None,
    })
}

/// Restore `~/.claude/settings.json` (or `~/.claude.json`) from the
/// captured `backup_bytes`. Used by the Organizer's Undo button.
pub fn restore_mcp_file(ctx_home: &Path, info: &RestoreInfo) -> Result<(), WardError> {
    let p = std::path::PathBuf::from(&info.original_path);
    // Defensive: ensure the path is inside home.
    let abs = crate::fs_utils::ensure_under_home(&p, ctx_home)?;
    match &info.backup_bytes {
        Some(bytes) => {
            if let Some(parent) = abs.parent() { std::fs::create_dir_all(parent)?; }
            std::fs::write(&abs, bytes)?;
        }
        None => {
            // No prior content captured — remove the file if it exists.
            if abs.exists() { std::fs::remove_file(&abs)?; }
        }
    }
    Ok(())
}

// ── Policy check ───────────────────────────────────────────────────────

/// Evaluate `server_name` + `server_config` against `policy`. Mirrors
/// CCO's `checkMcpPolicy` exactly:
///
///   1. Denylist has absolute precedence over allowlist.
///   2. Within each list, match by `serverName` (exact), `serverCommand`
///      (`[command, ...args]` array equality), or `serverUrl` (glob
///      where `*` matches any run of chars).
///   3. If both lists are empty → `NoPolicy`.
///   4. Denylist match → `Denied`.
///   5. Allowlist empty (and not denied) → `NoPolicy`.
///   6. Allowlist match → `Allowed`.
///   7. Allowlist non-empty + no match → `Denied`.
pub fn check_policy(server_name: &str, server_config: &Value, policy: &McpPolicy)
    -> PolicyVerdict
{
    // Denylist first.
    for entry in &policy.denylist {
        if entry_matches(entry, server_name, server_config) {
            return PolicyVerdict::Denied;
        }
    }
    // No allowlist → no policy.
    if policy.allowlist.is_empty() { return PolicyVerdict::NoPolicy; }
    for entry in &policy.allowlist {
        if entry_matches(entry, server_name, server_config) {
            return PolicyVerdict::Allowed;
        }
    }
    PolicyVerdict::Denied
}

fn entry_matches(entry: &PolicyEntry, server_name: &str, server_config: &Value) -> bool {
    if let Some(name) = &entry.server_name {
        if name == server_name { return true; }
    }
    if let Some(cmd) = &entry.server_command {
        if let Some(actual_cmd) = command_matches(cmd, server_config) {
            return actual_cmd;
        }
    }
    if let Some(url_pat) = &entry.server_url {
        if let Some(actual) = server_config.get("url").and_then(|v| v.as_str()) {
            if glob_match(url_pat, actual) { return true; }
        }
    }
    false
}

/// `cmd == [server_config.command, ...server_config.args]` (exact array
/// equality, mirroring `JSON.stringify(cmd) === JSON.stringify([c, ...args])`).
/// Returns `Some(true)` when matched, `Some(false)` when the policy
/// entry is a serverCommand but the server has no command/args (so we
/// keep scanning), `None` when the entry isn't a serverCommand at all
/// (so the caller can move on to the URL check).
fn command_matches(policy_cmd: &[String], server_config: &Value) -> Option<bool> {
    let command = server_config.get("command").and_then(|v| v.as_str())?;
    let args: Vec<&str> = server_config.get("args")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    if policy_cmd.len() != args.len() + 1 { return Some(false); }
    if policy_cmd[0] != command { return Some(false); }
    for (i, arg) in args.iter().enumerate() {
        if policy_cmd[i + 1] != *arg { return Some(false); }
    }
    Some(true)
}

/// Glob match where `*` is the only special character. CCO escapes
/// regex metachars and converts `*` to `.*`, then anchors with `^…$`.
/// We hand-roll the same algorithm to avoid pulling in the regex
/// crate: walk both strings left-to-right, treating `*` as a wildcard
/// that matches any run of characters (including empty).
fn glob_match(pattern: &str, value: &str) -> bool {
    glob_match_rec(pattern.as_bytes(), value.as_bytes())
}

fn glob_match_rec(pat: &[u8], val: &[u8]) -> bool {
    let mut pi = 0usize;
    let mut vi = 0usize;
    let mut star_pat: Option<usize> = None;
    let mut star_val: usize = 0;
    while vi < val.len() {
        if pi < pat.len() && pat[pi] == b'*' {
            star_pat = Some(pi);
            star_val = vi;
            pi += 1;
        } else if pi < pat.len() && pat[pi] == val[vi] {
            pi += 1;
            vi += 1;
        } else if let Some(sp) = star_pat {
            pi = sp + 1;
            star_val += 1;
            vi = star_val;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' { pi += 1; }
    pi == pat.len()
}

fn parse_policy_entries(v: Option<&Value>) -> Vec<PolicyEntry> {
    let Some(arr) = v.and_then(|v| v.as_array()) else { return Vec::new() };
    arr.iter().filter_map(|item| serde_json::from_value::<PolicyEntry>(item.clone()).ok()).collect()
}

pub(crate) fn write_json_pretty(path: &Path, value: &Value) -> Result<(), WardError> {
    let s = serde_json::to_string_pretty(value)
        .map_err(|e| WardError::NotFound(format!("serialize: {e}")))?;
    std::fs::write(path, format!("{s}\n"))?;
    Ok(())
}

// ════════════════════════════════════════════════════════════════════════
// TESTS — ported from CCO `tests/unit/test-security-features.mjs` lines
// 109-203 (the `checkMcpPolicy` block). Plus our own get/set tests.
// ════════════════════════════════════════════════════════════════════════

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

    // ── checkMcpPolicy — ported from CCO test-security-features.mjs ──

    #[test]
    fn check_policy_no_policy_when_both_empty() {
        let p = McpPolicy::default();
        let v = check_policy("my-server", &json!({}), &p);
        assert_eq!(v, PolicyVerdict::NoPolicy);
    }

    #[test]
    fn check_policy_denylist_precedes_allowlist() {
        let p = McpPolicy {
            allowlist: vec![PolicyEntry { server_name: Some("my-server".into()), ..Default::default() }],
            denylist: vec![PolicyEntry { server_name: Some("my-server".into()), ..Default::default() }],
        };
        let v = check_policy("my-server", &json!({}), &p);
        assert_eq!(v, PolicyVerdict::Denied);
    }

    #[test]
    fn check_policy_allows_by_name_in_allowlist() {
        let p = McpPolicy {
            allowlist: vec![PolicyEntry { server_name: Some("my-server".into()), ..Default::default() }],
            denylist: vec![],
        };
        let v = check_policy("my-server", &json!({}), &p);
        assert_eq!(v, PolicyVerdict::Allowed);
    }

    #[test]
    fn check_policy_denies_not_in_allowlist_when_allowlist_set() {
        let p = McpPolicy {
            allowlist: vec![PolicyEntry { server_name: Some("other-server".into()), ..Default::default() }],
            denylist: vec![],
        };
        let v = check_policy("my-server", &json!({}), &p);
        assert_eq!(v, PolicyVerdict::Denied);
    }

    #[test]
    fn check_policy_denies_by_server_name() {
        let p = McpPolicy {
            allowlist: vec![],
            denylist: vec![PolicyEntry { server_name: Some("bad-server".into()), ..Default::default() }],
        };
        let v = check_policy("bad-server", &json!({}), &p);
        assert_eq!(v, PolicyVerdict::Denied);
    }

    #[test]
    fn check_policy_denies_by_command_match() {
        let p = McpPolicy {
            allowlist: vec![],
            denylist: vec![PolicyEntry {
                server_command: Some(vec!["python".into(), "evil.py".into()]),
                ..Default::default()
            }],
        };
        let cfg = json!({ "command": "python", "args": ["evil.py"] });
        let v = check_policy("any", &cfg, &p);
        assert_eq!(v, PolicyVerdict::Denied);
    }

    #[test]
    fn check_policy_denies_by_url_wildcard() {
        let p = McpPolicy {
            allowlist: vec![],
            denylist: vec![PolicyEntry {
                server_url: Some("https://*.evil.com/*".into()),
                ..Default::default()
            }],
        };
        let cfg = json!({ "url": "https://api.evil.com/mcp" });
        let v = check_policy("any", &cfg, &p);
        assert_eq!(v, PolicyVerdict::Denied);
    }

    #[test]
    fn check_policy_allows_by_url_in_allowlist() {
        let p = McpPolicy {
            allowlist: vec![PolicyEntry {
                server_url: Some("https://*.company.com/*".into()),
                ..Default::default()
            }],
            denylist: vec![],
        };
        let cfg = json!({ "url": "https://api.company.com/mcp" });
        let v = check_policy("any", &cfg, &p);
        assert_eq!(v, PolicyVerdict::Allowed);
    }

    #[test]
    fn check_policy_allows_by_command_in_allowlist() {
        let p = McpPolicy {
            allowlist: vec![PolicyEntry {
                server_command: Some(vec!["node".into(), "approved.js".into()]),
                ..Default::default()
            }],
            denylist: vec![],
        };
        let cfg = json!({ "command": "node", "args": ["approved.js"] });
        let v = check_policy("any", &cfg, &p);
        assert_eq!(v, PolicyVerdict::Allowed);
    }

    #[test]
    fn check_policy_url_pattern_does_not_match_different_url() {
        let p = McpPolicy {
            allowlist: vec![],
            denylist: vec![PolicyEntry {
                server_url: Some("https://evil.com/mcp".into()),
                ..Default::default()
            }],
        };
        let cfg = json!({ "url": "https://good.com/mcp" });
        let v = check_policy("any", &cfg, &p);
        assert_eq!(v, PolicyVerdict::NoPolicy);
    }

    #[test]
    fn check_policy_command_match_requires_exact_array_match() {
        let p = McpPolicy {
            allowlist: vec![],
            denylist: vec![PolicyEntry {
                server_command: Some(vec!["node".into(), "evil.js".into()]),
                ..Default::default()
            }],
        };
        let cfg = json!({ "command": "node", "args": ["good.js"] });
        let v = check_policy("any", &cfg, &p);
        assert_eq!(v, PolicyVerdict::NoPolicy);
    }

    /// Extra port — CCO covered by the previous 11 tests. We add a few
    /// more to exercise edge cases that come up in real use: command
    /// match with extra args, missing args (treated as empty), and
    /// trailing `*` matching arbitrary suffix.
    #[test]
    fn check_policy_command_match_with_extra_args_does_not_match() {
        let p = McpPolicy {
            allowlist: vec![],
            denylist: vec![PolicyEntry {
                server_command: Some(vec!["node".into(), "evil.js".into()]),
                ..Default::default()
            }],
        };
        // Server has command + args (extra trailing arg) — does NOT match.
        let cfg = json!({ "command": "node", "args": ["evil.js", "extra"] });
        let v = check_policy("any", &cfg, &p);
        assert_eq!(v, PolicyVerdict::NoPolicy);
    }

    #[test]
    fn check_policy_url_with_trailing_star_matches_suffix() {
        let p = McpPolicy {
            allowlist: vec![],
            denylist: vec![PolicyEntry {
                server_url: Some("https://evil.com/*".into()),
                ..Default::default()
            }],
        };
        let cfg = json!({ "url": "https://evil.com/anything/here" });
        let v = check_policy("any", &cfg, &p);
        assert_eq!(v, PolicyVerdict::Denied);
    }

    #[test]
    fn check_policy_url_exact_match() {
        let p = McpPolicy {
            allowlist: vec![],
            denylist: vec![PolicyEntry {
                server_url: Some("https://evil.com/mcp".into()),
                ..Default::default()
            }],
        };
        let cfg = json!({ "url": "https://evil.com/mcp" });
        let v = check_policy("any", &cfg, &p);
        assert_eq!(v, PolicyVerdict::Denied);
    }

    #[test]
    fn check_policy_server_name_match_takes_priority_over_command_match() {
        // If an entry's server_name matches, the command/url fields are
        // ignored — mirrors CCO (the entry-level short-circuit).
        let p = McpPolicy {
            allowlist: vec![PolicyEntry {
                server_name: Some("github".into()),
                server_command: Some(vec!["python".into(), "evil.py".into()]),
                ..Default::default()
            }],
            denylist: vec![],
        };
        // server name matches → Allowed even though command doesn't fit.
        let v = check_policy("github", &json!({ "command": "node" }), &p);
        assert_eq!(v, PolicyVerdict::Allowed);
    }

    #[test]
    fn check_policy_url_glob_escapes_regex_metachars() {
        let p = McpPolicy {
            allowlist: vec![],
            denylist: vec![PolicyEntry {
                server_url: Some("https://x.com/api?y=1".into()),
                ..Default::default()
            }],
        };
        // '?' should match literally, not "any char".
        let v1 = check_policy("any", &json!({ "url": "https://x.com/api?y=1" }), &p);
        assert_eq!(v1, PolicyVerdict::Denied);
        let v2 = check_policy("any", &json!({ "url": "https://x.com/apiay=1" }), &p);
        assert_eq!(v2, PolicyVerdict::NoPolicy);
    }

    // ── get/set_disabled_servers ──

    #[test]
    fn get_disabled_empty_when_file_missing() {
        let (_d, home) = temp_home();
        let list = get_disabled_servers(&home, Path::new("/work/repo")).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn get_disabled_returns_listed_names() {
        let (_d, home) = temp_home();
        let json = home.join(".claude.json");
        fs::write(&json, r#"{"projects":{"/work/repo":{"disabledMcpServers":["a","b"]}}}"#).unwrap();
        let list = get_disabled_servers(&home, Path::new("/work/repo")).unwrap();
        assert_eq!(list, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn get_disabled_returns_empty_when_key_absent() {
        let (_d, home) = temp_home();
        let json = home.join(".claude.json");
        fs::write(&json, r#"{"projects":{"/work/repo":{"mcpServers":{"foo":{}}}}}"#).unwrap();
        let list = get_disabled_servers(&home, Path::new("/work/repo")).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn set_disabled_writes_correct_key_and_preserves_other_state() {
        let (_d, home) = temp_home();
        let json = home.join(".claude.json");
        fs::write(&json,
            r#"{"numStartups":7,"projects":{"/work/repo":{"mcpServers":{"github":{}}}}}"#
        ).unwrap();
        let info = set_disabled_servers(&home, Path::new("/work/repo"), &["github".into()]).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(after["numStartups"], 7);
        assert_eq!(after["projects"]["/work/repo"]["disabledMcpServers"][0], "github");
        assert_eq!(after["projects"]["/work/repo"]["mcpServers"]["github"], json!({}));
        assert_eq!(info.kind, "mcp-disabled");
        assert!(info.backup_bytes.is_some());
        // Unrelated state preserved.
        let backup = info.backup_bytes.as_ref().unwrap();
        let backup_v: Value = serde_json::from_slice(backup).unwrap();
        assert_eq!(backup_v["numStartups"], 7);
    }

    #[test]
    fn set_disabled_creates_file_when_missing() {
        let (_d, home) = temp_home();
        let json = home.join(".claude.json");
        assert!(!json.exists());
        let info = set_disabled_servers(&home, Path::new("/work/repo"), &["x".into()]).unwrap();
        assert!(json.exists());
        let after: Value = serde_json::from_str(&fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(after["projects"]["/work/repo"]["disabledMcpServers"][0], "x");
        assert!(info.backup_bytes.is_none(), "no prior file → no backup");
    }

    #[test]
    fn set_disabled_round_trip_preserves_bytes() {
        let (_d, home) = temp_home();
        let json = home.join(".claude.json");
        // Use a realistic file (with trailing newline + pretty-printing).
        let original = "{\n  \"numStartups\": 3,\n  \"projects\": {\"/a\": {\"otherKey\": \"hi\"}}\n}\n";
        fs::write(&json, original).unwrap();
        let info = set_disabled_servers(&home, Path::new("/a"), &["x".into()]).unwrap();
        // Restore.
        restore_mcp_file(&home, &info).unwrap();
        let after = fs::read_to_string(&json).unwrap();
        assert_eq!(after, original, "restore must yield byte-identical content");
    }

    #[test]
    fn set_disabled_overwrites_existing_list() {
        let (_d, home) = temp_home();
        let json = home.join(".claude.json");
        fs::write(&json, r#"{"projects":{"/work/repo":{"disabledMcpServers":["old"]}}}"#).unwrap();
        set_disabled_servers(&home, Path::new("/work/repo"), &["new1".into(), "new2".into()]).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&json).unwrap()).unwrap();
        let list = after["projects"]["/work/repo"]["disabledMcpServers"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], "new1");
        assert_eq!(list[1], "new2");
    }

    #[test]
    fn set_disabled_empty_list_clears_field() {
        let (_d, home) = temp_home();
        let json = home.join(".claude.json");
        fs::write(&json, r#"{"projects":{"/work/repo":{"disabledMcpServers":["old"]}}}"#).unwrap();
        set_disabled_servers(&home, Path::new("/work/repo"), &[]).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&json).unwrap()).unwrap();
        let list = after["projects"]["/work/repo"]["disabledMcpServers"].as_array().unwrap();
        assert!(list.is_empty());
    }

    // ── get/set_policy ──

    #[test]
    fn get_policy_empty_when_file_missing() {
        let (_d, home) = temp_home();
        let p = get_policy(&home).unwrap();
        assert!(p.allowlist.is_empty());
        assert!(p.denylist.is_empty());
    }

    #[test]
    fn get_policy_parses_lists() {
        let (_d, home) = temp_home();
        let settings = home.join(".claude/settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(&settings, r#"{
            "allowedMcpServers": [{"serverName": "github"}, {"serverUrl": "https://*.corp.com/*"}],
            "deniedMcpServers": [{"serverCommand": ["python", "evil.py"]}]
        }"#).unwrap();
        let p = get_policy(&home).unwrap();
        assert_eq!(p.allowlist.len(), 2);
        assert_eq!(p.allowlist[0].server_name.as_deref(), Some("github"));
        assert_eq!(p.allowlist[1].server_url.as_deref(), Some("https://*.corp.com/*"));
        assert_eq!(p.denylist.len(), 1);
        assert_eq!(p.denylist[0].server_command.as_deref(), Some(&["python".to_string(), "evil.py".to_string()][..]));
    }

    #[test]
    fn set_policy_writes_lists_and_preserves_other_settings() {
        let (_d, home) = temp_home();
        let settings = home.join(".claude/settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let original = "{\n  \"theme\": \"dark\",\n  \"permissions\": {\"allow\": [\"Bash\"]},\n  \"numStartups\": 12\n}\n";
        fs::write(&settings, original).unwrap();
        let policy = McpPolicy {
            allowlist: vec![PolicyEntry { server_name: Some("github".into()), ..Default::default() }],
            denylist: vec![PolicyEntry {
                server_command: Some(vec!["python".into(), "evil.py".into()]),
                ..Default::default()
            }],
        };
        let info = set_policy(&home, &policy).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["permissions"]["allow"][0], "Bash");
        assert_eq!(after["numStartups"], 12);
        assert_eq!(after["allowedMcpServers"][0]["serverName"], "github");
        assert_eq!(after["deniedMcpServers"][0]["serverCommand"], json!(["python", "evil.py"]));
        assert_eq!(info.kind, "mcp-policy");
        // Undo: restore to byte-identical content.
        restore_mcp_file(&home, &info).unwrap();
        let after_restore = fs::read_to_string(&settings).unwrap();
        assert_eq!(after_restore, original);
    }

    #[test]
    fn set_policy_round_trip_preserves_bytes() {
        let (_d, home) = temp_home();
        let settings = home.join(".claude/settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let original = "{\n  \"numStartups\": 7,\n  \"allowedMcpServers\": [],\n  \"deniedMcpServers\": []\n}\n";
        fs::write(&settings, original).unwrap();
        let policy = McpPolicy::default();
        let info = set_policy(&home, &policy).unwrap();
        restore_mcp_file(&home, &info).unwrap();
        let after = fs::read_to_string(&settings).unwrap();
        assert_eq!(after, original);
    }

    #[test]
    fn set_policy_creates_file_when_missing() {
        let (_d, home) = temp_home();
        let settings = home.join(".claude/settings.json");
        assert!(!settings.exists());
        let info = set_policy(&home, &McpPolicy::default()).unwrap();
        assert!(settings.exists());
        assert!(info.backup_bytes.is_none());
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        // Empty policy → keys omitted entirely. Writing `[]` would make Claude
        // Code treat it as an allowlist that blocks every MCP server.
        assert!(after.get("allowedMcpServers").is_none());
        assert!(after.get("deniedMcpServers").is_none());
    }

    #[test]
    fn set_policy_overwrites_lists() {
        let (_d, home) = temp_home();
        let settings = home.join(".claude/settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(&settings,
            r#"{"allowedMcpServers":[{"serverName":"old"}],"deniedMcpServers":[]}"#
        ).unwrap();
        let policy = McpPolicy {
            allowlist: vec![],
            denylist: vec![PolicyEntry { server_name: Some("new".into()), ..Default::default() }],
        };
        set_policy(&home, &policy).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        // Empty allowlist clears the prior `old` entry by removing the key,
        // rather than writing a block-everything `[]`.
        assert!(after.get("allowedMcpServers").is_none());
        assert_eq!(after["deniedMcpServers"][0]["serverName"], "new");
    }

    #[test]
    fn restore_mcp_file_removes_when_no_backup() {
        let (_d, home) = temp_home();
        let json = home.join(".claude.json");
        fs::write(&json, r#"{"projects":{}}"#).unwrap();
        let info = RestoreInfo {
            kind: "mcp-disabled".into(),
            original_path: json.display().to_string(),
            current_path: None,
            backup_bytes: None,
            mcp_entry: None,
            mcp_key: Some("/work".into()),
            mcp_parent_key: Some("projects".into()),
            mcp_scope: None,
        };
        restore_mcp_file(&home, &info).unwrap();
        assert!(!json.exists());
    }
}