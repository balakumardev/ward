//! Plan 29 Task 4 — the effective-value reader across Claude Code's scope chain.
//!
//! For every def in the bundled catalog this resolves the value Claude Code
//! would actually use on this machine, and records which scope set it. The
//! precedence for ordinary `settings.json`-class keys is
//! **managed > local > project > user** (first scope whose file carries the
//! key wins). `claudeJson`-class keys are read only from `~/.claude.json`'s
//! top level and reported under the `user` scope. A key set in no scope falls
//! back to the catalog default with `source_scope == "default"` and
//! `is_set == false`.
//!
//! Every scope file is read at most once, up front. A missing / unreadable /
//! non-JSON-object file simply contributes no keys — the reader never panics.
//!
//! The system-wide managed-settings path is injected into the private
//! [`scan_settings_at`] so tests can seed it in a tempdir (or point at a
//! nonexistent path) without touching the real `/Library/Application Support`
//! location; the public [`scan_settings`] supplies the real path.

use std::path::Path;

use serde_json::{Map, Value};

use super::{load_catalog, SettingRow, SettingsCatalog};

/// Absolute path to Claude Code's system-wide managed-settings file on macOS.
/// Managed settings are pushed by MDM and outrank every user/project scope.
const MANAGED_SETTINGS_PATH: &str = "/Library/Application Support/ClaudeCode/managed-settings.json";

/// The scope files, each loaded once as its top-level JSON object (or `None`
/// when the file is missing / unreadable / not a JSON object).
#[derive(Default)]
struct ScopeValues {
    managed: Option<Map<String, Value>>,
    user: Option<Map<String, Value>>,
    project: Option<Map<String, Value>>,
    local: Option<Map<String, Value>>,
    claude_json: Option<Map<String, Value>>,
}

impl ScopeValues {
    /// Highest-precedence `settings.json`-class scope carrying `key`
    /// (managed > local > project > user), as `(scope_name, value)`. `None`
    /// when no such scope has the key.
    fn lookup_settings(&self, key: &str) -> Option<(&'static str, &Value)> {
        for (name, map) in [
            ("managed", &self.managed),
            ("local", &self.local),
            ("project", &self.project),
            ("user", &self.user),
        ] {
            if let Some(m) = map {
                if let Some(v) = m.get(key) {
                    return Some((name, v));
                }
            }
        }
        None
    }
}

/// Read `path` into its top-level JSON object. Returns `None` — never an
/// error, never a panic — when the file is absent, unreadable, invalid JSON,
/// or a non-object JSON value.
fn read_scope_map(path: &Path) -> Option<Map<String, Value>> {
    let content = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&content).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Build one [`SettingRow`] per catalog def, in catalog order, resolving the
/// effective value + source scope against the pre-loaded `scopes`.
fn compute_rows(catalog: &SettingsCatalog, scopes: &ScopeValues) -> Vec<SettingRow> {
    catalog
        .defs
        .iter()
        .map(|def| {
            // `claudeJson`-class keys live only in ~/.claude.json's top level
            // and are attributed to the `user` scope. Everything else follows
            // the managed > local > project > user precedence chain.
            let hit = if def.target_file == "claudeJson" {
                scopes
                    .claude_json
                    .as_ref()
                    .and_then(|m| m.get(&def.key))
                    .map(|v| ("user", v))
            } else {
                scopes.lookup_settings(&def.key)
            };

            match hit {
                Some((scope, value)) => SettingRow {
                    def: def.clone(),
                    effective: Some(value.clone()),
                    source_scope: Some(scope.to_string()),
                    is_set: true,
                },
                None => SettingRow {
                    def: def.clone(),
                    effective: def.default.clone(),
                    source_scope: Some("default".to_string()),
                    is_set: false,
                },
            }
        })
        .collect()
}

/// Effective value of every catalog setting on this machine. `home` is the
/// user home (`~`); `project_dir`, when `Some`, is the repo whose
/// `.claude/settings.json` + `.claude/settings.local.json` join the chain.
pub fn scan_settings(home: &Path, project_dir: Option<&Path>) -> Vec<SettingRow> {
    scan_settings_at(home, project_dir, Path::new(MANAGED_SETTINGS_PATH))
}

/// [`scan_settings`] with the managed-settings path injected (for tests).
fn scan_settings_at(home: &Path, project_dir: Option<&Path>, managed_path: &Path) -> Vec<SettingRow> {
    let catalog = load_catalog();
    let scopes = ScopeValues {
        managed: read_scope_map(managed_path),
        user: read_scope_map(&home.join(".claude").join("settings.json")),
        project: project_dir
            .and_then(|d| read_scope_map(&d.join(".claude").join("settings.json"))),
        local: project_dir
            .and_then(|d| read_scope_map(&d.join(".claude").join("settings.local.json"))),
        claude_json: read_scope_map(&home.join(".claude.json")),
    };
    compute_rows(&catalog, &scopes)
}

#[cfg(test)]
mod tests {
    use super::{scan_settings, scan_settings_at};
    use crate::settings::{load_catalog, SettingRow};
    use serde_json::json;
    use std::path::Path;

    /// Write `json` to `path`, creating parent dirs.
    fn write_json(path: &Path, json: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, json).unwrap();
    }

    /// The row for `key` (panics if the catalog somehow lacks it).
    fn row<'a>(rows: &'a [SettingRow], key: &str) -> &'a SettingRow {
        rows.iter()
            .find(|r| r.def.key == key)
            .unwrap_or_else(|| panic!("no row for key '{key}'"))
    }

    #[test]
    fn reads_user_scope_value_and_marks_source() {
        let home = tempfile::tempdir().unwrap();
        write_json(
            &home.path().join(".claude/settings.json"),
            r#"{"theme":"light"}"#,
        );
        let managed = home.path().join("no-managed.json");

        let rows = scan_settings_at(home.path(), None, &managed);
        let theme = row(&rows, "theme");
        assert_eq!(theme.effective, Some(json!("light")));
        assert_eq!(theme.source_scope.as_deref(), Some("user"));
        assert!(theme.is_set);
    }

    #[test]
    fn local_overrides_project_overrides_user() {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_json(
            &home.path().join(".claude/settings.json"),
            r#"{"model":"user-model"}"#,
        );
        write_json(
            &project.path().join(".claude/settings.json"),
            r#"{"model":"project-model","verbose":true}"#,
        );
        write_json(
            &project.path().join(".claude/settings.local.json"),
            r#"{"model":"local-model"}"#,
        );
        let managed = home.path().join("no-managed.json");

        let rows = scan_settings_at(home.path(), Some(project.path()), &managed);

        // Present in all three → local wins.
        let model = row(&rows, "model");
        assert_eq!(model.effective, Some(json!("local-model")));
        assert_eq!(model.source_scope.as_deref(), Some("local"));
        assert!(model.is_set);

        // Present only in project → attributed to project.
        let verbose = row(&rows, "verbose");
        assert_eq!(verbose.effective, Some(json!(true)));
        assert_eq!(verbose.source_scope.as_deref(), Some("project"));
        assert!(verbose.is_set);
    }

    #[test]
    fn unset_key_reports_default_and_is_set_false() {
        let home = tempfile::tempdir().unwrap();
        // A present-but-empty user file: the key is absent, not the file.
        write_json(&home.path().join(".claude/settings.json"), "{}");
        let managed = home.path().join("no-managed.json");

        let rows = scan_settings_at(home.path(), None, &managed);
        let cleanup = row(&rows, "cleanupPeriodDays");
        assert_eq!(cleanup.effective, Some(json!(30))); // catalog default
        assert_eq!(cleanup.source_scope.as_deref(), Some("default"));
        assert!(!cleanup.is_set);
    }

    #[test]
    fn claudejson_targeted_key_read_from_claude_json() {
        let home = tempfile::tempdir().unwrap();
        // autoConnectIde is a claudeJson-target key → lives in ~/.claude.json.
        write_json(
            &home.path().join(".claude.json"),
            r#"{"autoConnectIde":true}"#,
        );
        // A stray value in settings.json must be IGNORED for a claudeJson key.
        write_json(
            &home.path().join(".claude/settings.json"),
            r#"{"autoConnectIde":false}"#,
        );
        let managed = home.path().join("no-managed.json");

        let rows = scan_settings_at(home.path(), None, &managed);
        let auto = row(&rows, "autoConnectIde");
        assert_eq!(auto.effective, Some(json!(true))); // from claude.json, not settings.json
        assert_eq!(auto.source_scope.as_deref(), Some("user"));
        assert!(auto.is_set);
    }

    #[test]
    fn missing_files_do_not_panic() {
        let home = tempfile::tempdir().unwrap(); // empty — no scope files at all
        let managed = home.path().join("no-managed.json");

        let rows = scan_settings_at(home.path(), None, &managed);
        let catalog = load_catalog();
        assert_eq!(rows.len(), catalog.defs.len());
        assert!(
            rows.iter().all(|r| !r.is_set),
            "nothing is set when no files exist"
        );
        // Effective mirrors each def's default (including `None` for keys with
        // no documented default), and everything is attributed to "default".
        assert!(rows.iter().all(|r| r.effective == r.def.default));
        assert!(rows
            .iter()
            .all(|r| r.source_scope.as_deref() == Some("default")));
    }

    #[test]
    fn managed_overrides_local_project_user() {
        // Managed precedence, exercised through the real read path by seeding
        // the injected managed file in a tempdir (no system dir written).
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_json(
            &home.path().join(".claude/settings.json"),
            r#"{"theme":"light"}"#,
        );
        write_json(
            &project.path().join(".claude/settings.json"),
            r#"{"theme":"light"}"#,
        );
        write_json(
            &project.path().join(".claude/settings.local.json"),
            r#"{"theme":"light"}"#,
        );
        let managed = home.path().join("managed-settings.json");
        write_json(&managed, r#"{"theme":"dark"}"#);

        let rows = scan_settings_at(home.path(), Some(project.path()), &managed);
        let theme = row(&rows, "theme");
        assert_eq!(theme.effective, Some(json!("dark")));
        assert_eq!(theme.source_scope.as_deref(), Some("managed"));
        assert!(theme.is_set);
    }

    #[test]
    fn bad_json_scope_is_skipped_not_panicked() {
        // A malformed higher-precedence file is ignored; the reader falls
        // through to the next scope instead of panicking.
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_json(&home.path().join(".claude/settings.json"), "{not valid json");
        write_json(
            &project.path().join(".claude/settings.json"),
            r#"{"theme":"light"}"#,
        );
        let managed = home.path().join("no-managed.json");

        let rows = scan_settings_at(home.path(), Some(project.path()), &managed);
        let theme = row(&rows, "theme");
        assert_eq!(theme.effective, Some(json!("light")));
        assert_eq!(theme.source_scope.as_deref(), Some("project"));
        assert!(theme.is_set);
    }

    #[test]
    fn public_scan_settings_does_not_panic() {
        // The public entry supplies the real system managed path; it must not
        // panic and must return exactly one row per catalog def regardless of
        // whether a managed file exists on this machine.
        let home = tempfile::tempdir().unwrap();
        let rows = scan_settings(home.path(), None);
        let catalog = load_catalog();
        assert_eq!(rows.len(), catalog.defs.len());
    }
}
