//! Plan 29 — Settings mode wire models + the bundled settings catalog.
//!
//! The catalog is a hand-curated metadata table (`settings-catalog.json`,
//! compiled in via `include_str!`) that supplies the label / description /
//! default / render-hint for every Claude Code `settings.json` key — because
//! Claude Code's published JSON Schema carries no descriptions or defaults and
//! lags releases. Later tasks add the effective-value reader, the surgical
//! writer, the schema-diff tripwire, and the curated env-var list; this task
//! defines the shared models plus the loader/validator they all build on.

use serde::{Deserialize, Serialize};

/// One curated setting definition — the metadata Ward shows for a single
/// Claude Code `settings.json` (or `~/.claude.json`) key. Sourced from the
/// bundled `settings-catalog.json`, never introspected from the harness.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingDef {
    /// The settings key, e.g. `"theme"` or `"permissions"`.
    pub key: String,
    /// Short human title.
    pub label: String,
    /// One to two sentences on what the setting does.
    pub description: String,
    /// Grouping label; should appear in [`SettingsCatalog::categories`].
    pub category: String,
    /// Render/validation hint: `bool | enum | number | string | array | object`.
    pub value_type: String,
    /// Documented default value; `None` when the docs list none. Serializes
    /// as `default` and is omitted from the wire form when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// Allowed values — populated (and required) when `value_type == "enum"`.
    /// Serializes as `enumValues`.
    #[serde(default)]
    pub enum_values: Vec<String>,
    /// Destination file: `"settings.json"` (default) or `"claudeJson"` (the
    /// `~/.claude.json` global-config class). Serializes as `targetFile`.
    pub target_file: String,
    /// Writable scopes for this key — a subset of `user | project | local`.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// True when the key is only settable via managed-settings.json.
    /// Serializes as `managedOnly`.
    #[serde(default)]
    pub managed_only: bool,
    /// First Claude Code version that supports the key, when known.
    /// Serializes as `minVersion`; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    /// Link to the documenting page, when known. Serializes as `docsUrl`;
    /// omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    /// For `value_type == "object"`: which bespoke editor renders it —
    /// `permissions | hooks | env | sandbox | statusLine | json`. Omitted
    /// from the wire form when absent (non-object types).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<String>,
}

/// A catalog def paired with its current effective value on this machine. The
/// effective-value reader (a later task) fills `effective` / `source_scope` /
/// `is_set`; this model is defined here so the reader and the command layer
/// share one shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingRow {
    pub def: SettingDef,
    /// The effective value across the scope chain; `None` means the key is
    /// unset (the UI falls back to the def's `default`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective: Option<serde_json::Value>,
    /// Which scope set the effective value: `user | project | local |
    /// managed | default`. Serializes as `sourceScope`; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_scope: Option<String>,
    /// True when the key is explicitly set in some scope (vs. defaulted).
    /// Serializes as `isSet`.
    pub is_set: bool,
}

/// The whole bundled catalog: the ordered category rail plus every def.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsCatalog {
    pub categories: Vec<String>,
    pub defs: Vec<SettingDef>,
}

/// Parse the bundled `settings-catalog.json` into a [`SettingsCatalog`].
///
/// The file is compiled into the binary via `include_str!`, so this never
/// touches the filesystem. A parse failure is a build/authoring bug, not a
/// runtime condition — the `load_catalog_parses` test keeps the bundled file
/// valid, so a bad edit fails `cargo test` rather than shipping a panic.
pub fn load_catalog() -> SettingsCatalog {
    const CATALOG_JSON: &str = include_str!("settings-catalog.json");
    serde_json::from_str(CATALOG_JSON).expect("settings-catalog.json must be valid")
}

/// Validate a catalog's internal consistency. Returns `Err` (naming the
/// offending key) when any def is malformed:
///   - empty `key` / `label` / `description` / `category` / `value_type`;
///   - `value_type == "enum"` with an empty `enum_values`;
///   - `value_type == "object"` with `editor == None`;
///   - a duplicate `key`;
///   - a `target_file` outside `{ "settings.json", "claudeJson" }`.
pub fn validate_catalog(cat: &SettingsCatalog) -> Result<(), String> {
    use std::collections::HashSet;

    let mut seen: HashSet<&str> = HashSet::new();
    for def in &cat.defs {
        if def.key.is_empty() {
            return Err("a def has an empty key".to_string());
        }
        let k = def.key.as_str();
        if def.label.is_empty() {
            return Err(format!("def '{k}' has an empty label"));
        }
        if def.description.is_empty() {
            return Err(format!("def '{k}' has an empty description"));
        }
        if def.category.is_empty() {
            return Err(format!("def '{k}' has an empty category"));
        }
        if def.value_type.is_empty() {
            return Err(format!("def '{k}' has an empty valueType"));
        }
        if def.value_type == "enum" && def.enum_values.is_empty() {
            return Err(format!("def '{k}' is an enum but has no enumValues"));
        }
        if def.value_type == "object" && def.editor.is_none() {
            return Err(format!("def '{k}' is an object but has no editor"));
        }
        if def.target_file != "settings.json" && def.target_file != "claudeJson" {
            return Err(format!(
                "def '{k}' has an invalid targetFile '{}'",
                def.target_file
            ));
        }
        if !seen.insert(k) {
            return Err(format!("duplicate key '{k}'"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A minimal well-formed def, tweaked per test.
    fn def(key: &str, value_type: &str) -> SettingDef {
        SettingDef {
            key: key.into(),
            label: "Label".into(),
            description: "Desc".into(),
            category: "General".into(),
            value_type: value_type.into(),
            default: None,
            enum_values: Vec::new(),
            target_file: "settings.json".into(),
            scopes: Vec::new(),
            managed_only: false,
            min_version: None,
            docs_url: None,
            editor: None,
        }
    }

    #[test]
    fn setting_def_serializes_camel_case_round_trip() {
        let d = SettingDef {
            key: "theme".into(),
            label: "Theme".into(),
            description: "Colour theme.".into(),
            category: "Appearance".into(),
            value_type: "enum".into(),
            default: Some(json!("dark")),
            enum_values: vec!["dark".into(), "light".into()],
            target_file: "settings.json".into(),
            scopes: vec!["user".into()],
            managed_only: false,
            min_version: None,
            docs_url: Some("https://code.claude.com/docs/en/settings".into()),
            editor: None,
        };
        let s = serde_json::to_string(&d).unwrap();
        // camelCase renames land on the wire.
        assert!(s.contains("\"valueType\":\"enum\""), "valueType present: {s}");
        assert!(
            s.contains("\"enumValues\":[\"dark\",\"light\"]"),
            "enumValues present: {s}"
        );
        assert!(
            s.contains("\"targetFile\":\"settings.json\""),
            "targetFile present: {s}"
        );
        assert!(s.contains("\"managedOnly\":false"), "managedOnly present: {s}");
        // `None` optionals are omitted.
        assert!(!s.contains("editor"), "None editor omitted: {s}");
        assert!(!s.contains("minVersion"), "None minVersion omitted: {s}");
        // Round-trips byte-for-value.
        let back: SettingDef = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn setting_row_serializes_camel_case_round_trip() {
        let row = SettingRow {
            def: def("verbose", "bool"),
            effective: Some(json!(true)),
            source_scope: Some("user".into()),
            is_set: true,
        };
        let s = serde_json::to_string(&row).unwrap();
        assert!(s.contains("\"sourceScope\":\"user\""), "sourceScope present: {s}");
        assert!(s.contains("\"isSet\":true"), "isSet present: {s}");
        let back: SettingRow = serde_json::from_str(&s).unwrap();
        assert_eq!(row, back);
    }

    #[test]
    fn setting_row_omits_none_effective_and_source() {
        let row = SettingRow {
            def: def("verbose", "bool"),
            effective: None,
            source_scope: None,
            is_set: false,
        };
        let s = serde_json::to_string(&row).unwrap();
        assert!(!s.contains("effective"), "None effective omitted: {s}");
        assert!(!s.contains("sourceScope"), "None sourceScope omitted: {s}");
        assert!(s.contains("\"isSet\":false"), "isSet present: {s}");
    }

    #[test]
    fn load_catalog_parses() {
        let cat = load_catalog();
        assert!(!cat.defs.is_empty(), "bundled catalog must have defs");
        assert!(
            !cat.categories.is_empty(),
            "bundled catalog must have categories"
        );
    }

    #[test]
    fn validate_catalog_ok_on_bundled() {
        assert!(
            validate_catalog(&load_catalog()).is_ok(),
            "bundled catalog must validate"
        );
    }

    #[test]
    fn validate_catalog_rejects_dupe_keys() {
        let cat = SettingsCatalog {
            categories: vec!["General".into()],
            defs: vec![def("verbose", "bool"), def("verbose", "bool")],
        };
        let err = validate_catalog(&cat).unwrap_err();
        assert!(err.contains("verbose"), "err names the dup key: {err}");
        assert!(err.contains("duplicate"), "err says duplicate: {err}");
    }

    #[test]
    fn validate_catalog_rejects_enum_without_values() {
        let cat = SettingsCatalog {
            categories: vec!["Appearance".into()],
            defs: vec![def("theme", "enum")], // enum_values left empty
        };
        let err = validate_catalog(&cat).unwrap_err();
        assert!(err.contains("theme"), "err names the offending key: {err}");
    }

    #[test]
    fn validate_catalog_rejects_object_without_editor() {
        let cat = SettingsCatalog {
            categories: vec!["Permissions".into()],
            defs: vec![def("permissions", "object")], // editor left None
        };
        let err = validate_catalog(&cat).unwrap_err();
        assert!(
            err.contains("permissions"),
            "err names the offending key: {err}"
        );
    }

    #[test]
    fn validate_catalog_rejects_empty_required_field() {
        // Empty key.
        let cat = SettingsCatalog {
            categories: vec![],
            defs: vec![def("", "bool")],
        };
        assert!(validate_catalog(&cat).is_err(), "empty key must be rejected");

        // Empty label — the error must still name the key.
        let mut d = def("verbose", "bool");
        d.label = String::new();
        let cat = SettingsCatalog {
            categories: vec![],
            defs: vec![d],
        };
        let err = validate_catalog(&cat).unwrap_err();
        assert!(err.contains("verbose"), "err names the offending key: {err}");
    }

    #[test]
    fn validate_catalog_rejects_bad_target_file() {
        let mut d = def("model", "string");
        d.target_file = "config.toml".into();
        let cat = SettingsCatalog {
            categories: vec![],
            defs: vec![d],
        };
        let err = validate_catalog(&cat).unwrap_err();
        assert!(err.contains("model"), "err names the offending key: {err}");
    }

    #[test]
    fn validate_catalog_accepts_claude_json_target() {
        let mut d = def("autoConnectIde", "bool");
        d.target_file = "claudeJson".into();
        let cat = SettingsCatalog {
            categories: vec!["General".into()],
            defs: vec![d],
        };
        assert!(
            validate_catalog(&cat).is_ok(),
            "claudeJson is a valid targetFile"
        );
    }
}
