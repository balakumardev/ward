//! Plan 29 Task 6 — the schema-diff tripwire (schemastore drift detector).
//!
//! Ward's `settings-catalog.json` is a hand-curated metadata table; Claude
//! Code's own settings surface grows every release. This module fetches the
//! community-maintained JSON Schema for Claude Code settings
//! (`https://json.schemastore.org/claude-code-settings.json`) and diffs its
//! top-level property keys against the catalog's keys — so a maintainer can
//! see, at a glance, which new settings need a curated def added (and which
//! curated keys the schema doesn't [yet] carry).
//!
//! Split like `marketplace/registry.rs`: a pure [`diff_keys`] (fully
//! unit-tested) and a thin [`schema_diff`] `ureq` wrapper (network, not
//! unit-tested). The tripwire is a maintainer aid, never a hot path — it is
//! only ever run on demand.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::SettingsCatalog;
use crate::error::WardError;

/// The published JSON Schema for Claude Code's `settings.json`, maintained by
/// the SchemaStore community and kept close to upstream releases.
const SCHEMA_URL: &str = "https://json.schemastore.org/claude-code-settings.json";
/// Network timeout for the on-demand fetch — same 10 s budget as the registry.
const TIMEOUT_SECS: u64 = 10;

/// The result of diffing the published schema's property keys against Ward's
/// curated catalog keys — both directions, each sorted for a stable report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDiff {
    /// Keys the schema publishes that the catalog has no def for — the
    /// "add these" list a maintainer acts on.
    pub in_schema_not_catalog: Vec<String>,
    /// Keys the catalog curates that the schema doesn't list — either curated
    /// ahead of the schema, or `~/.claude.json`-class keys the schema omits.
    pub in_catalog_not_schema: Vec<String>,
}

/// Diff two key lists both ways. `in_schema_not_catalog` = schema keys absent
/// from the catalog; `in_catalog_not_schema` = catalog keys absent from the
/// schema. Both result vecs are sorted (and de-duplicated) for a deterministic
/// report regardless of input order. Pure — fully unit-tested.
pub fn diff_keys(schema_props: &[String], catalog_keys: &[String]) -> SchemaDiff {
    use std::collections::HashSet;

    let schema_set: HashSet<&str> = schema_props.iter().map(String::as_str).collect();
    let catalog_set: HashSet<&str> = catalog_keys.iter().map(String::as_str).collect();

    let mut in_schema_not_catalog: Vec<String> = schema_props
        .iter()
        .filter(|k| !catalog_set.contains(k.as_str()))
        .cloned()
        .collect();
    let mut in_catalog_not_schema: Vec<String> = catalog_keys
        .iter()
        .filter(|k| !schema_set.contains(k.as_str()))
        .cloned()
        .collect();

    // Sort + dedup so the report is stable no matter how the inputs are
    // ordered, and a key listed twice on either side doesn't produce noise.
    in_schema_not_catalog.sort();
    in_schema_not_catalog.dedup();
    in_catalog_not_schema.sort();
    in_catalog_not_schema.dedup();

    SchemaDiff {
        in_schema_not_catalog,
        in_catalog_not_schema,
    }
}

/// Fetch the published Claude Code settings schema and diff its top-level
/// `properties` keys against `catalog`'s def keys. On-demand only (never a
/// background poll); transport/HTTP/parse failures map to
/// [`WardError::Settings`]. The pure diff is delegated to [`diff_keys`].
pub fn schema_diff(catalog: &SettingsCatalog) -> Result<SchemaDiff, WardError> {
    let resp = match ureq::get(SCHEMA_URL)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => {
            return Err(WardError::Settings(format!(
                "the settings schema host returned HTTP {code}"
            )));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(WardError::Settings(format!(
                "network error reaching the settings schema host: {t}"
            )));
        }
    };

    let body = resp
        .into_string()
        .map_err(|e| WardError::Settings(format!("read settings schema response body: {e}")))?;

    let root: Value = serde_json::from_str(&body)
        .map_err(|e| WardError::Settings(format!("parse settings schema: {e}")))?;

    let props = root
        .get("properties")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            WardError::Settings("settings schema has no top-level `properties` object".into())
        })?;

    let schema_props: Vec<String> = props.keys().cloned().collect();
    let catalog_keys: Vec<String> = catalog.defs.iter().map(|d| d.key.clone()).collect();

    Ok(diff_keys(&schema_props, &catalog_keys))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_keys_reports_both_directions() {
        let schema = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let catalog = vec!["b".to_string(), "c".to_string(), "d".to_string()];
        let diff = diff_keys(&schema, &catalog);
        assert_eq!(diff.in_schema_not_catalog, vec!["a".to_string()]);
        assert_eq!(diff.in_catalog_not_schema, vec!["d".to_string()]);
    }

    #[test]
    fn diff_keys_empty_when_identical() {
        let keys = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let diff = diff_keys(&keys, &keys);
        assert!(
            diff.in_schema_not_catalog.is_empty(),
            "no schema-only keys: {:?}",
            diff.in_schema_not_catalog
        );
        assert!(
            diff.in_catalog_not_schema.is_empty(),
            "no catalog-only keys: {:?}",
            diff.in_catalog_not_schema
        );
    }

    #[test]
    fn diff_keys_sorts_output() {
        // Unsorted inputs; each side carries keys the other lacks plus a shared
        // key that must appear in neither result.
        let schema = vec![
            "z".to_string(),
            "m".to_string(),
            "a".to_string(),
            "shared".to_string(),
        ];
        let catalog = vec!["shared".to_string(), "q".to_string(), "b".to_string()];
        let diff = diff_keys(&schema, &catalog);
        assert_eq!(
            diff.in_schema_not_catalog,
            vec!["a".to_string(), "m".to_string(), "z".to_string()],
            "schema-only keys sorted"
        );
        assert_eq!(
            diff.in_catalog_not_schema,
            vec!["b".to_string(), "q".to_string()],
            "catalog-only keys sorted"
        );
    }

    #[test]
    fn schema_diff_serializes_camel_case() {
        let diff = SchemaDiff {
            in_schema_not_catalog: vec!["newKey".to_string()],
            in_catalog_not_schema: vec!["staleKey".to_string()],
        };
        let s = serde_json::to_string(&diff).unwrap();
        assert!(
            s.contains("\"inSchemaNotCatalog\""),
            "camelCase field present: {s}"
        );
        assert!(
            s.contains("\"inCatalogNotSchema\""),
            "camelCase field present: {s}"
        );
        // Round-trips value-for-value.
        let back: SchemaDiff = serde_json::from_str(&s).unwrap();
        assert_eq!(diff, back);
    }
}
