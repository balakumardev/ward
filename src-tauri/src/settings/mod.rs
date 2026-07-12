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

pub mod env;
pub mod read;
pub mod schema;
pub mod write;

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
    /// For `value_type == "number"`: the minimum valid value, when the docs
    /// document one (e.g. `cleanupPeriodDays` minimum 1). The UI applies it as
    /// the input's `min` bound and clamps out-of-range edits before writing.
    /// Serializes as `min`; omitted when absent (the number is unconstrained).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// For `value_type == "number"`: the maximum valid value, when documented
    /// (e.g. a `0-1` probability). Serializes as `max`; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// For `value_type == "number"`: the input step, when documented. Serializes
    /// as `step`; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// For `value_type == "number"`: true when only whole numbers are valid, so
    /// the UI rounds fractional edits before writing. Serializes as `integer`;
    /// omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integer: Option<bool>,
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
            min: None,
            max: None,
            step: None,
            integer: None,
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
            // Numeric bounds: min + integer set, max + step left None (to assert
            // both the camelCase-safe serialize AND the skip-when-None behavior).
            min: Some(1.0),
            max: None,
            step: None,
            integer: Some(true),
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
        // The set numeric-bound fields serialize (already camelCase-safe names).
        assert!(s.contains("\"min\":1.0"), "min present: {s}");
        assert!(s.contains("\"integer\":true"), "integer present: {s}");
        // `None` optionals are omitted.
        assert!(!s.contains("editor"), "None editor omitted: {s}");
        assert!(!s.contains("minVersion"), "None minVersion omitted: {s}");
        assert!(!s.contains("\"max\""), "None max omitted: {s}");
        assert!(!s.contains("\"step\""), "None step omitted: {s}");
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

    /// The curated catalog (Task 3) must be internally valid, cover the full
    /// documented key set (≥100 defs), and match the docs on a set of anchor
    /// keys spanning every value type and both target files.
    #[test]
    fn catalog_is_comprehensive_and_valid() {
        let cat = load_catalog();
        assert!(
            validate_catalog(&cat).is_ok(),
            "bundled catalog must validate: {:?}",
            validate_catalog(&cat)
        );
        assert!(
            cat.defs.len() >= 100,
            "catalog must cover the full documented key set (≥100 defs); got {}",
            cat.defs.len()
        );

        let by_key = |k: &str| -> &SettingDef {
            cat.defs
                .iter()
                .find(|d| d.key == k)
                .unwrap_or_else(|| panic!("catalog is missing the '{k}' def"))
        };

        // Every category a def uses must appear in the category rail.
        let cats: std::collections::HashSet<&str> =
            cat.categories.iter().map(String::as_str).collect();
        for d in &cat.defs {
            assert!(
                cats.contains(d.category.as_str()),
                "def '{}' uses category '{}' not listed in `categories`",
                d.key,
                d.category
            );
        }

        // number + documented default.
        let cleanup = by_key("cleanupPeriodDays");
        assert_eq!(cleanup.value_type, "number");
        assert_eq!(cleanup.default, Some(json!(30)));

        // enum with the documented value set.
        let theme = by_key("theme");
        assert_eq!(theme.value_type, "enum");
        assert!(theme.enum_values.iter().any(|v| v == "dark"));
        assert!(theme.enum_values.iter().any(|v| v == "light"));

        // object with the permissions editor.
        let perms = by_key("permissions");
        assert_eq!(perms.value_type, "object");
        assert_eq!(perms.editor.as_deref(), Some("permissions"));

        // object with the hooks editor.
        let hooks = by_key("hooks");
        assert_eq!(hooks.value_type, "object");
        assert_eq!(hooks.editor.as_deref(), Some("hooks"));

        // ~/.claude.json global-config class routes to claudeJson.
        assert_eq!(by_key("autoConnectIde").target_file, "claudeJson");

        // managed-only keys are flagged and scoped to managed.
        let allowed_mcp = by_key("allowedMcpServers");
        assert!(allowed_mcp.managed_only);
        assert_eq!(allowed_mcp.scopes, vec!["managed".to_string()]);
    }

    /// Numeric defs carry validation bounds ONLY where the doc prose documents
    /// one — never invented. `cleanupPeriodDays` documents "minimum 1; setting 0
    /// fails validation" and counts whole days; `feedbackSurveyRate` documents a
    /// "Probability (0-1)". The two skill-listing numbers document no explicit
    /// numeric range, so they stay unconstrained.
    #[test]
    fn catalog_number_defs_carry_only_documented_bounds() {
        let cat = load_catalog();
        let by_key = |k: &str| -> &SettingDef {
            cat.defs
                .iter()
                .find(|d| d.key == k)
                .unwrap_or_else(|| panic!("catalog is missing the '{k}' def"))
        };

        // "minimum 1 (setting 0 fails validation)" + whole days → min 1, integer, step 1.
        let cleanup = by_key("cleanupPeriodDays");
        assert_eq!(cleanup.value_type, "number");
        assert_eq!(cleanup.min, Some(1.0));
        assert_eq!(cleanup.integer, Some(true));
        assert_eq!(cleanup.step, Some(1.0));

        // "Probability (0-1)" → min 0, max 1, fractional (no integer/step).
        let rate = by_key("feedbackSurveyRate");
        assert_eq!(rate.value_type, "number");
        assert_eq!(rate.min, Some(0.0));
        assert_eq!(rate.max, Some(1.0));
        assert_eq!(rate.integer, None);

        // No explicit documented numeric range → left unconstrained (not invented).
        for k in ["skillListingBudgetFraction", "skillListingMaxDescChars"] {
            let d = by_key(k);
            assert_eq!(d.value_type, "number");
            assert_eq!(d.min, None, "{k} min must be unset");
            assert_eq!(d.max, None, "{k} max must be unset");
            assert_eq!(d.step, None, "{k} step must be unset");
            assert_eq!(d.integer, None, "{k} integer must be unset");
        }
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
