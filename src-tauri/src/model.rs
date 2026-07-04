use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub context_budget: bool,
    pub mcp_controls: bool,
    pub mcp_policy: bool,
    pub mcp_security: bool,
    pub sessions: bool,
    pub effective: bool,
    pub backup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessItem {
    pub category: String,
    pub scope_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub path: String,
    pub movable: bool,
    pub deletable: bool,
    pub locked: bool,
    /// Effective status — `None`, `Some("active")`, `Some("shadowed")`,
    /// `Some("conflict")`, or `Some("ancestor")`. Active items are not
    /// tagged (None). Shadowed/conflict/ancestor are computed per project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub harness_id: String,
    pub categories: Vec<Category>,
    pub scopes: Vec<Scope>,
    pub items: Vec<HarnessItem>,
    pub capabilities: Capabilities,
}

/// A scope the user can move an item into. `kind` mirrors `Scope.kind`
/// (`global` / `project` / `project-unresolved`) plus a virtual
/// `home-overlap` marker used by callers that want to know why a scope
/// was rejected (the UI hides home-overlap destinations for file-based
/// categories).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    pub scope_id: String,
    pub label: String,
    pub kind: String,
}

/// Live-undo payload for mutations. The Organizer keeps the most
/// recent payload and offers an Undo button. No on-disk history is
/// kept — restore is local to this object.
///
/// Variants:
///   `kind = "file"`
///     - `original_path` is the file's location before the mutation.
///     - `current_path` is where the file lives *right now* after the
///       mutation (dest for a move; same as original_path for a no-op).
///       When `backup_bytes` is `Some`, restore writes the bytes back
///       to `original_path` (delete case). When `backup_bytes` is
///       `None`, restore renames `current_path` → `original_path`
///       (move case). For skill directories, `backup_bytes` holds a
///       JSON-encoded `{relative_path: base64_bytes}` map describing
///       the whole tree captured at delete time.
///   `kind = "mcp-entry"`
///     - `original_path` is the JSON file the entry was edited in.
///     - `mcp_entry` is the JSON value that was removed (and must be
///       re-inserted on undo).
///     - `mcp_key` is the entry name (`mcpServers[<key>]`).
///     - `mcp_parent_key` is the parent object name — usually
///       `mcpServers`, but for project entries inside `~/.claude.json`
///       it is the `projects` object and `mcp_scope` carries the
///       project key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreInfo {
    pub kind: String,
    pub original_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_bytes: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_entry: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_parent_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_scope: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_item_serializes_camel_case() {
        let item = HarnessItem {
            category: "skill".into(),
            scope_id: "global".into(),
            name: "brainstorming".into(),
            description: String::new(),
            path: "/Users/x/.claude/skills/brainstorming".into(),
            movable: true,
            deletable: true,
            locked: false,
            effective: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"scopeId\":\"global\""));
        assert!(json.contains("\"category\":\"skill\""));
        assert!(!json.contains("\"description\""), "empty description must be omitted");
        assert!(!json.contains("\"effective\""), "None effective must be omitted");
    }

    #[test]
    fn restore_info_round_trips_file_delete() {
        let info = RestoreInfo {
            kind: "file".into(),
            original_path: "/Users/x/.claude/memory/foo.md".into(),
            current_path: Some("/Users/x/.claude/memory/foo.md".into()),
            backup_bytes: Some(b"hello".to_vec()),
            mcp_entry: None,
            mcp_key: None,
            mcp_parent_key: None,
            mcp_scope: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: RestoreInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
    }

    #[test]
    fn restore_info_round_trips_mcp_entry() {
        let info = RestoreInfo {
            kind: "mcp-entry".into(),
            original_path: "/Users/x/.claude/.mcp.json".into(),
            current_path: None,
            backup_bytes: None,
            mcp_entry: Some(serde_json::json!({"command": "echo"})),
            mcp_key: Some("foo".into()),
            mcp_parent_key: Some("mcpServers".into()),
            mcp_scope: Some("global".into()),
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: RestoreInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
        // mcp-specific fields must serialize
        assert!(json.contains("\"mcpKey\""));
        assert!(json.contains("\"mcpParentKey\""));
        assert!(json.contains("\"mcpScope\""));
        assert!(json.contains("\"mcpEntry\""));
    }
}
