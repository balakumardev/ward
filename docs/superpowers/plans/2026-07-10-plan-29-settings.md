# Settings Command Center Implementation Plan (Plan 29)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Add an 8th sidebar mode, **Settings**, that shows *every* Claude Code `settings.json` setting with a curated human explanation + default, displays the current effective value and which scope set it, and lets the user edit / reset each one — simple types inline, the complex objects (permissions, hooks, env, sandbox, statusLine) via bespoke editors — all with Ward's byte-exact undo.

**Architecture:** A hand-curated metadata catalog (bundled JSON, user-maintained via a repo monitor) is the source of labels/descriptions/defaults/render-hints — because Claude Code's published JSON Schema has no descriptions/defaults and lags releases. Reads compute the effective value across the scope chain (managed > local > project > user); writes are surgical single-key edits (port `set_policy`) routed to the correct file (`settings.json` or `~/.claude.json`), with byte-exact undo. A schema-diff command flags drift so the user knows what to add to the catalog next.

**Tech Stack:** Rust (Tauri 2 `invoke`, `serde_json`, `ureq`), SolidJS + TS + Vite, vitest, `spawn_blocking`.

## Global Constraints

- **No stubs / TODOs / placeholders.** Comprehensive: the catalog ships ALL documented settings keys day one; every simple type is editable inline; the 5 named complex objects get bespoke editors.
- **TDD:** failing test → implement → green → commit. `cargo test`, `npm test`, `npx tsc --noEmit`, `cargo check` pass before each commit.
- **Naming lock:** reuse `WardError`, `Capabilities`, `RestoreInfo`, `HarnessOps`, `ops_for`, `MODES`, `api`, and Plan 28's proven patterns.
- **Models:** derive `Debug, Clone, Serialize, Deserialize, PartialEq` + `#[serde(rename_all="camelCase")]`.
- **Errors:** `WardError::Settings(String)` (new) via the existing manual `impl Serialize` (`#[serde(tag="kind", content="message")]`, camelCase), mirroring the `Registry`/`Plugin` arms.
- **Async commands:** every FS/network command is `pub async fn` + `tauri::async_runtime::spawn_blocking`; inner logic sync for fast unit tests.
- **Writer discipline:** port `claude_mcp::set_policy` (`claude_mcp.rs:130-168`) exactly — read prior bytes → mutate ONE key → `write_json_pretty` → `RestoreInfo{backup_bytes:prior}`. **Unset = remove the key** (the empty-remove path, `claude_mcp.rs:145-154`), never write `null`/`[]`. Reuse `claude_mcp::{write_json_pretty}` and `restore_mcp_file` (already `pub(crate)` from Plan 28).
- **Target-file routing:** each catalog key declares `targetFile` — `settings.json` (default) OR `claudeJson` (the `~/.claude.json` global-config class: `autoConnectIde`, `autoInstallIdeExtension`, `externalEditorContext`, `teammateDefaultModel`, `workflowSizeGuideline`). Writing these to settings.json is a schema error, so route correctly.
- **Permissions/MCP-policy overlap:** `allowedMcpServers`/`deniedMcpServers` are already written by Security mode's `set_policy`. Do NOT add a second writer for those two keys — either exclude them from the Settings editor or delegate to the existing `mcp_set_policy`. `permissions.allow/ask/deny` (Bash/tool rules) are distinct and DO get a Settings editor.
- **WKWebView:** no `window.confirm/alert/prompt` — in-app modal for the complex-object editors and any destructive confirm (reuse `askConfirm` pattern from `Plugins.tsx`).
- **Styling:** class-based `src/styles/settings.css` + tokens. `--sh-1/-2/-3` are box-shadow tokens — never in border/background/color; use `var(--border)`/`--surface*`/`--text*`/`--accent`/`--ok`/`--warn`/`--crit`. Preserve/introduce `data-testid`s.
- **Managed scope is read-only:** keys set in managed-settings render read-only with an indicator (a user can't override managed).
- **Secrets:** the `env`-object editor must not log values; no secret written to any Ward-owned file/log.
- **Reference templates:** `set_policy`/`restore_mcp_file` (`claude_mcp.rs`), `scan_settings` (`claude.rs:682-726`, the scope-source-file logic + value carriage), Plan 28's `plugins/mod.rs`+`Plugins.tsx` (models, commands, mode, capability gate, mock, in-app modal), command registration (`lib.rs`).

---

### Task 1: `WardError::Settings` + `settingsEditable` capability
**Files:** `src-tauri/src/error.rs`, `src-tauri/src/model.rs` (`Capabilities`), `claude.rs`/`codex.rs` capability literals, `src/api.ts` (mirror). Mirror Plan 28 Task 1 exactly (it added `Plugin`/`pluginsManageable`).
- Produces: `WardError::Settings(String)` (wire `{"kind":"settings","message":"settings error: ..."}` — Display-equals-message, like Registry/Plugin); `Capabilities.settings_editable` (Claude `true`, Codex `false`); TS `settingsEditable: boolean`.
- [ ] Failing test `settings_error_serializes_camel_case` (asserts `{"kind":"settings","message":"settings error: x"}`) → implement (add variant + serialize arm mirroring `Plugin`; add cap field at every construction site + TS) → green → commit `feat(settings): add WardError::Settings + settingsEditable capability`.

### Task 2: Settings models + catalog loader (`settings/mod.rs`)
**Files:** create `src-tauri/src/settings/mod.rs`; `pub mod settings;` in `lib.rs`.
- Produces:
  - `SettingDef { key, label, description, category, value_type: String /* bool|enum|number|string|array|object */, default: Option<serde_json::Value>, enum_values: Vec<String>, target_file: String /* settings.json|claudeJson */, scopes: Vec<String>, managed_only: bool, min_version: Option<String>, docs_url: Option<String>, editor: Option<String> /* for object types: permissions|hooks|env|sandbox|statusLine|json */ }`
  - `SettingRow { def: SettingDef, effective: Option<serde_json::Value>, source_scope: Option<String> /* user|project|local|managed|default */, is_set: bool }`
  - `SettingsCatalog { categories: Vec<String>, defs: Vec<SettingDef> }`
  - `load_catalog() -> SettingsCatalog` via `include_str!("settings-catalog.json")` + parse.
  - `validate_catalog(&SettingsCatalog) -> Result<(), String>` — every def has non-empty key/label/description/category/value_type; `value_type=="enum"` ⇒ non-empty `enum_values`; `value_type=="object"` ⇒ `editor` set; no duplicate keys; `target_file ∈ {settings.json, claudeJson}`.
- [ ] Failing tests: serde camelCase round-trip of `SettingDef`; `load_catalog_parses` (loads the bundled file — a minimal stub JSON in Task 2, real content in Task 3); `validate_catalog_rejects_dupes_and_missing_fields`. Implement → green → commit. (Ship a small valid stub `settings-catalog.json` in this task so it compiles/loads; Task 3 replaces it with the full set.)

### Task 3: The curated catalog — ALL settings keys (`settings-catalog.json`)
**Files:** `src-tauri/src/settings/settings-catalog.json` (replace the Task 2 stub).
This is the comprehensive data task. **Fetch the live docs** (`https://code.claude.com/docs/en/settings`, `.../permissions`, `.../env-vars`) and transcribe EVERY documented `settings.json` key into a `SettingDef` record with an accurate human `label`, `description` (from the docs prose — concise, one to two sentences on what it does), `category`, `value_type`, `default` (from docs; omit if none), `enum_values` (for enums), `target_file`, `scopes`, `managed_only`, `docs_url`.
- Coverage floor (the integrity test enforces ≥100 defs): booleans (~48: `autoCompactEnabled`, `autoMemoryEnabled`, `includeCoAuthoredBy`, `verbose`, `disableAllHooks`, `spinnerTipsEnabled`, `respectGitignore`, `fileCheckpointingEnabled`, …), enums (`theme`, `editorMode`, `autoUpdatesChannel`, `preferredNotifChannel`, `outputStyle`, `effortLevel`, `viewMode`, `permissions.defaultMode`, …), numbers (`cleanupPeriodDays` def 30, `maxSkillDescriptionChars`, …), strings (`model`, `apiKeyHelper`, `autoMemoryDirectory`, `plansDirectory`, `statusLine.*`, …), arrays (`enabledMcpjsonServers`, `disabledMcpjsonServers`, `additionalDirectories`, `availableModels`, `fallbackModel`, …), and the object types with `editor` set: `permissions` (editor `permissions`), `hooks` (`hooks`), `env` (`env`), `sandbox` (`sandbox`), `statusLine` (`statusLine`), plus `enabledPlugins`/`extraKnownMarketplaces` (editor `json`, or note managed-by-Plugins-mode). Route the `~/.claude.json` global-config class to `targetFile: claudeJson`. Mark managed-only keys `managed_only: true`.
- [ ] Failing test `catalog_is_comprehensive_and_valid`: `validate_catalog(&load_catalog()).is_ok()` AND `defs.len() >= 100` AND spot-checks (e.g. `cleanupPeriodDays` default 30 number; `theme` enum contains `dark`/`light`; `permissions` value_type object editor permissions; `autoConnectIde` targetFile claudeJson). Implement the full JSON → green → commit `feat(settings): curated settings catalog (all documented keys)`. Include a header note in the JSON documenting the record format so the user can extend it.

### Task 4: Effective-value reader (`settings/read.rs`)
**Files:** create `settings/read.rs`; `pub mod read;`.
- Produces: `scan_settings(home: &Path, project_dir: Option<&Path>) -> Vec<SettingRow>` — for each catalog def, read across the scope chain and compute effective value + `source_scope` (precedence managed > local > project > user; `default` when unset). Reuse the scope-source-file resolution from `claude::scan_settings` (`claude.rs:684-692`) for user/project/local; managed file path `/Library/Application Support/ClaudeCode/managed-settings.json`. `claudeJson`-targeted keys read from `~/.claude.json` top-level. Missing/bad file → skip that scope (never panic).
- [ ] Failing tests: `reads_user_scope_value_and_marks_source`; `local_overrides_project_overrides_user`; `unset_key_reports_default_and_is_set_false`; `claudejson_targeted_key_read_from_claude_json`. Seed tempdirs. Implement → green → commit.

### Task 5: Surgical writer (`settings/write.rs`)
**Files:** create `settings/write.rs`; `pub mod write;`; add restore arm in `claude_ops.rs`.
- Produces:
  - `set_setting(home, scope: &str, key: &str, target_file: &str, value: serde_json::Value) -> Result<RestoreInfo, WardError>` — resolve the target path for (scope, target_file); port `set_policy`: read prior bytes → set `root[key]=value` (dotted keys like `permissions` are single top-level keys — the value is the whole object) → `write_json_pretty` → `RestoreInfo{kind:"setting-write"}`. Refuse to write when `scope=="managed"` (`WardError::Settings("managed settings are read-only")`).
  - `unset_setting(home, scope, key, target_file) -> Result<RestoreInfo, WardError>` — remove the key (the empty-remove path), byte-exact undo.
  - Restore arm `"setting-write"` added to `ClaudeOps::restore` → `restore_mcp_file`.
- [ ] Failing tests: `set_preserves_siblings_and_undo_byte_exact`; `unset_removes_key_not_null`; `set_writes_object_value_whole` (e.g. a `permissions` object); `managed_scope_refused`; `claudejson_target_routes_to_claude_json`. Implement → green → commit.

### Task 6: Schema-diff tripwire (`settings/schema.rs`)
**Files:** create `settings/schema.rs`; `pub mod schema;`.
- Produces: `schema_diff(catalog: &SettingsCatalog) -> Result<SchemaDiff, WardError>` where `SchemaDiff { in_schema_not_catalog: Vec<String>, in_catalog_not_schema: Vec<String> }` — `ureq` GET `https://json.schemastore.org/claude-code-settings.json` (10s timeout, user-triggered), read its `properties` keys, diff vs catalog keys. Pure `diff_keys(schema_props: &[String], catalog_keys: &[String]) -> SchemaDiff` unit-tested; the fetch wrapper untested (mirrors registry split). `WardError::Settings` on fetch/parse error.
- [ ] Failing test `diff_keys_reports_both_directions` (pure). Implement → green → commit.

### Task 7: Curated env-var list (`settings/env.rs`)
**Files:** create `settings/env.rs`; `pub mod env;`.
- Produces: `env_catalog() -> Vec<EnvVarDef>` where `EnvVarDef { name, description, category }` — a curated subset of the documented `CLAUDE_*`/`ANTHROPIC_*`/`DISABLE_*`/`OTEL_*`/limit env vars (the user-facing ones), for a search-driven secondary list. These are edited into the `env` object via the Task 5 writer (key `env`).
- [ ] Failing test `env_catalog_nonempty_and_has_descriptions`. Implement (transcribe a curated subset from the env-vars doc, ~30-60 entries) → green → commit.

### Task 8: Tauri commands + registration
**Files:** `src-tauri/src/commands.rs`, `lib.rs`.
- Produces (async spawn_blocking; home via `dirs::home_dir()` like Plan 28 commands): `settings_catalog() -> Vec<SettingRow>` (load catalog + read effective for the active project); `settings_set(scope, key, target_file, value) -> Result<RestoreInfo,_>`; `settings_unset(scope, key, target_file) -> Result<RestoreInfo,_>`; `settings_schema_diff() -> SchemaDiff`; `settings_env_list() -> Vec<EnvVarDef>`. Register all in `generate_handler!`.
- [ ] Light test (the empty-key/managed-scope guard is the pure seam) → implement → green → commit.

### Task 9: api.ts wrappers + types + mock
**Files:** `src/api.ts`, `src/mock/*`, new test.
- Produces: TS types `SettingDef`/`SettingRow`/`SettingsCatalog`/`SchemaDiff`/`EnvVarDef` (camelCase); `api.settingsCatalog()`, `settingsSet(scope,key,targetFile,value)`, `settingsUnset(scope,key,targetFile)`, `settingsSchemaDiff()`, `settingsEnvList()`. `'setting-write'` added to `RestoreInfo.kind`. Mock bridge answers all 5 via existing dispatch, seeded with a representative catalog (bool/enum/number/string/array + the 5 object types, mixed set/unset across scopes).
- [ ] Failing vitest (assert command names + arg shapes) → implement → green → commit.

### Task 10: Sidebar mode + App scaffold (capability-gated)
**Files:** `src/components/Sidebar.tsx` (MODES: `{ id:'settings', label:'Settings', icon:'⚙' }` — unique; existing ⌘ ⛨ ▣ ⧉ ↺ ◱ ⊞), `src/App.tsx` (nested `<Show>` + `settingsApi` bridge), create `src/modes/Settings.tsx` scaffold + `src/styles/settings.css`.
- Self-gate inside Settings.tsx on `settingsEditable` (unsupported panel for Codex), mirror Plan 28's Plugins gate. Root `data-testid="settings-mode"`.
- [ ] Failing test `renders settings mode; unsupported for codex` → implement → green → commit.

### Task 11: Settings.tsx — category rail + list + simple editors + scope switcher + reset
**Files:** `src/modes/Settings.tsx`, `settings.css`.
- Left rail = catalog `categories` (`data-testid="settings-cat"`); main = searchable list of `SettingRow` (`settings-search`, `setting-row`); each row shows label, description, effective value + source-scope chip (`setting-source`), default, and an inline editor by `valueType`: bool→toggle (`setting-toggle`), enum→dropdown (`setting-enum`), number→number input, string→text input. **Scope switcher** (User/Project/Local, `settings-scope`) picks the write target → `api.settingsSet`. **Reset to default** per row (`setting-reset`) → `api.settingsUnset`. Managed source → read-only + indicator. Undo via returned RestoreInfo + `api.restore` (reuse Plan 28's undo-toast pattern). Object-type rows show an "Edit…" button (wired in Tasks 12-14).
- [ ] Failing tests: `list renders rows with source chip`; `toggling a bool calls settingsSet with scope+key+value`; `reset calls settingsUnset`; `managed row is read-only`; `search filters`. Implement → green → commit.

### Task 12: Array editor + env key-value editor + generic JSON object editor
**Files:** `src/modes/Settings.tsx`, `settings.css`.
- Array `valueType` → inline list editor (add/remove string entries, `setting-array`). `env` object → an "Edit…" modal with a key/value table (`setting-env-editor`, add/remove rows) writing the `env` object. Generic object types without a bespoke editor → an "Edit…" modal with a **validated JSON textarea** (`setting-json-editor`) that parses before save (invalid JSON blocks save with an inline error) and writes the whole object. All modals use the in-app `askConfirm`/dialog pattern (no `window.confirm`).
- [ ] Failing tests: `array editor add/remove calls settingsSet with the new array`; `env editor writes an object`; `json editor blocks invalid JSON and saves valid`. Implement → green → commit.

### Task 13: Bespoke editors — permissions + statusLine + sandbox
**Files:** `src/modes/Settings.tsx`, `settings.css`.
- `permissions` editor (`setting-perms-editor`): three array editors (allow/ask/deny rule strings) + `defaultMode` dropdown + `additionalDirectories` array → writes the `permissions` object. (Do NOT touch `allowedMcpServers`/`deniedMcpServers` here.)
- `statusLine` editor (`setting-statusline-editor`): fields `type` (fixed `command`), `command` (text), `padding` (number) → writes the object.
- `sandbox` editor (`setting-sandbox-editor`): filesystem + network allow/deny array editors → writes the nested object.
- [ ] Failing tests per editor (assert the composed object written via settingsSet). Implement → green → commit.

### Task 14: Bespoke editor — hooks
**Files:** `src/modes/Settings.tsx`, `settings.css`.
- `hooks` editor (`setting-hooks-editor`): list hook entries grouped by event (`PreToolUse`/`PostToolUse`/`Stop`/…); each entry = matcher (string) + a command list `{type:"command", command, timeout?}`; add/edit/remove → writes the `hooks` object in Claude Code's documented shape. Reference the read shape from `claude::scan_hooks` (`claude.rs:573-651`).
- [ ] Failing tests: `add hook writes the correct hooks object shape`; `remove hook`. Implement → green → commit.

### Task 15: schema-diff UI + env-var panel + polish + full green gate
**Files:** `src/modes/Settings.tsx`, `settings.css`.
- "Check for new settings" button (`settings-schema-diff`) → `api.settingsSchemaDiff()` → shows added/removed/renamed keys (so the user knows what to add to the catalog). Env-var secondary panel (`settings-env-list`, search-driven) from `api.settingsEnvList()` with an "add to env" affordance that opens the env editor. Polish accumulated review Minors.
- [ ] Full gate: `cargo test` + `cargo check` (no new warnings); `npm test` + `npx tsc --noEmit` + `npm run build`; headless `ward --scan --harness claude` still valid. Commit `chore(settings): schema-diff UI + env panel + polish + green gate`.

---

## Self-Review (author checklist)
- **Spec coverage:** show every setting + explanation (Tasks 3,11) ✓; current value + source scope (Task 4,11) ✓; edit simple + array + object types (Tasks 11-14) ✓; bespoke editors for permissions/hooks/env/sandbox/statusLine (Tasks 12-14) ✓; enable/disable/configure with undo (Task 5,11) ✓; keep-current tripwire (Task 6,15) ✓; env vars (Task 7,15) ✓; capability gate + proper UI/UX (Tasks 10-15) ✓.
- **Type consistency:** `SettingDef`/`SettingRow`/`SettingsCatalog`/`SchemaDiff`/`EnvVarDef`, `set_setting`/`unset_setting`, `settings_*` commands referenced identically across tasks.
- **Known risks pre-adjudicated:** managed read-only; MCP-policy keys excluded from the Settings writer (Security mode owns them); `claudeJson` routing; unset=remove-key; catalog is user-maintained (schema-diff flags drift — not auto-sync).
