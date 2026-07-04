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
}
