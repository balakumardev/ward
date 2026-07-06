# Plan 20 — Codex Write Path (CodexOps + TOML upsert) — Implementation Plan

> **For agentic workers:** implement task-by-task with TDD (failing test → implement → green → commit). Per-task reviews are being skipped for this run (user directive); a single whole-branch review runs at the very end. Still: NO stubs/TODOs, full green bar before finishing, one conventional commit per task.

**Goal:** Give the Codex harness a real write path so MCP servers are add/edit/remove-able and skills are creatable for Codex, mirroring what Plan 18/19 gave Claude — but writing `~/.codex/config.toml` surgically with `toml_edit` (comment/format preserving), and flipping Codex's `mcpEditable`/`skillCreatable` capabilities on.

**Reference (authoritative):** design spec `docs/superpowers/specs/2026-07-06-ward-mcp-marketplace-design.md` §5.2 (Codex TOML upsert), §8 (Codex write path). CCO read-only behavior parity: `/Users/balakumar/personal/claude-code-organizer/src/harness/adapters/codex.mjs` (`scanMcpServers`, `unsupportedOperations`) — READ ONLY, do not copy verbatim.

**Prerequisite already on branch:** Plan 18 added `HarnessOps::upsert_mcp_entry` (trait), `RestoreInfo{kind:"mcp-upsert"}`, the `mcpEditable` capability. Plan 19 added `skill_upsert`/`validate_skill_name` (in `claude_ops.rs`), the `skill-create` RestoreInfo kind, and the `skillCreatable` capability. This plan builds on BOTH.

## Global Constraints

- Reuse names verbatim: `WardError`, `HarnessOps`, `Ctx`, `Registry`, `CodexAdapter`, `RestoreInfo`, `Capabilities`, `ensure_under_home`, `Scope`, `HarnessItem`, `ops_for`. New type: `CodexOps`.
- All structs derive `Debug, Clone, Serialize, Deserialize, PartialEq` + `#[serde(rename_all = "camelCase")]`. Errors via `WardError`.
- **`~/.codex/config.toml` writes MUST be surgical + format-preserving** — use `toml_edit::DocumentMut` (add dep `toml_edit = "0.22"`). NEVER a `toml::Value` serialize round-trip (destroys comments, reorders keys, mangles quoted keys / nested sub-tables). Preserve every other table, comment, and key. Whole-file `backup_bytes` for undo.
- MCP item `path` for Codex is the WHOLE `config.toml` — surgical `[mcp_servers.NAME]` edit only, never full-file replace.
- **Commit `Cargo.lock`** (dependency added).
- TDD; `cargo test` from `src-tauri/`; `npm test` / `npx tsc --noEmit` / `npm run build` from repo root; all green before finishing. One conventional commit per task.

## Tasks

### Task 1 — Add `toml_edit` dep + `json_to_toml_table` + `codex_ops.rs` scaffold with MCP TOML upsert
- Add `toml_edit = "0.22"` to `src-tauri/Cargo.toml` (near the `toml = "0.8"` line). Commit `Cargo.lock`.
- Create `src-tauri/src/harness/adapters/codex_ops.rs` with `pub struct CodexOps;`.
- Implement a free fn `pub fn json_to_toml_table(config: &serde_json::Value) -> toml_edit::Table` — the inverse of `codex.rs::toml_to_json` (map string/number/bool/array/nested-object recursively into `toml_edit` items; arrays → `toml_edit::Array`; nested objects → sub-tables). Unit-test it round-trips `{command, args:[..], env:{..}}` and `{url, headers:{..}}`.
- Implement `pub fn upsert_mcp_entry_toml(target: &Path, name: &str, config: &serde_json::Value) -> Result<RestoreInfo, WardError>`: read file text (empty if absent), `parse::<DocumentMut>()`, ensure `mcp_servers` table exists, `doc["mcp_servers"][name] = Item::Table(json_to_toml_table(config))`, `create_dir_all` parent, write `doc.to_string()`, return `RestoreInfo{kind:"mcp-upsert", original_path, backup_bytes: (None if file was absent/empty else prior bytes), mcp_key:Some(name), mcp_parent_key:Some("mcp_servers")}`.
- Golden tests (TDD): insert new server into a commented multi-section config preserves comments + other tables + quoted keys (`[mcp_servers."auggie-mcp"]`); overwrite existing server; create file when absent (backup None); undo via `restore_mcp_file` byte-identical for edit / removes for create.
- Register `pub mod codex_ops;` in `src-tauri/src/harness/adapters/mod.rs`.
- Commit: `feat(codex): toml_edit dep + json_to_toml_table + surgical mcp_servers upsert`.

### Task 2 — `impl HarnessOps for CodexOps` (upsert/delete/restore/save_file/destinations) + register in `ops_for`
- Implement `impl HarnessOps for CodexOps`:
  - `save_file` — mirror `ClaudeOps::save_file` (`ensure_under_home` + create_dir_all + write).
  - `upsert_mcp_entry(ctx, scope_id, name, config, target_path, scopes)` — resolve target: `target_path` if Some (edit; `ensure_under_home`), else the scope's config.toml (global → `ctx.home/.codex/config.toml`; project → `<scope.root>/.codex/config.toml`). Call `upsert_mcp_entry_toml`.
  - `delete_item(ctx, item, scopes)` — for `category=="mcp"`: read config.toml, capture whole-file `backup_bytes`, `doc["mcp_servers"].as_table_mut().remove(&item.name)`, write, return `RestoreInfo{kind:"mcp-upsert", backup_bytes:Some(prior), original_path}` (whole-file restore). For `category` in {memory, skill, rule}: delete the file/dir (skill = remove dir; memory/rule = remove file), capturing bytes/tree for undo, `kind:"file"` (mirror ClaudeOps delete_single_file / delete_skill_dir semantics). Reject other categories with a clear error.
  - `restore(ctx, info)` — `"file"` → restore bytes/tree (mirror ClaudeOps restore_file: bytes→write, skill-tree JSON→rebuild); `"mcp-upsert"` → `claude_mcp::restore_mcp_file(ctx.home, info)` (whole-file, generic under home); `"skill-create"` → remove the created dir (`ensure_under_home` + remove_dir_all). Unknown → error.
  - `get_valid_destinations` → `Vec::new()` (Codex config is single-file per scope; move stays a Claude capability). `move_item` → `Err(WardError::NotFound("Codex does not support moving items"))`.
- In `src-tauri/src/commands.rs` `ops_for`, add the arm `"codex" => Ok(&CodexOps),`.
- Tests: round-trip delete+restore for a Codex MCP server (preserves other tables); delete+restore a Codex skill dir; `ops_for("codex")` returns CodexOps (upsert works end-to-end); move returns unsupported.
- Commit: `feat(codex): CodexOps HarnessOps impl + ops_for("codex") wiring`.

### Task 3 — Flip Codex item flags + capabilities (make Codex writable in the UI)
- In `src-tauri/src/harness/adapters/codex.rs`:
  - MCP items (`scan_mcp_servers`): set `deletable: true` (they already carry `mcp_config`). Keep `movable: false`.
  - `capabilities()`: set `mcp_editable: true` and `skill_creatable: true` (Codex now has a write path). Leave `mcp_controls`/`mcp_policy` as they are (Codex has no per-project disable/allowlist model).
- Update the codex capabilities unit test to assert `mcp_editable` and `skill_creatable` are now `true`.
- Update the Codex mock scan fixture (`src/mock/fixtures.ts`) capabilities: `mcpEditable: true`, `skillCreatable: true`; ensure its MCP items have `deletable: true` and an `mcpConfig`.
- Full green bar (`cargo test`, `npm test`, `tsc`, `build`).
- Commit: `feat(codex): flip mcp deletable + mcpEditable/skillCreatable capabilities on`.

### Task 4 — Codex `Enabled` field in the MCP form (frontend)
- In `src/modes/Organizer.tsx` `McpForm`: when the active harness is Codex, render an `Enabled` checkbox (`data-testid="mcp-enabled"`) bound to `config.enabled` (default true when absent). On Save, include `enabled` in the patched config (Codex uses the `enabled` bool in `[mcp_servers.NAME]`; Claude ignores it — only render the checkbox for Codex). Thread the harness id into `Organizer`/`McpForm` (App already has `harness()`; pass `props.scan.harnessId`).
- Test: rendering an editable Codex MCP item shows `mcp-enabled`; toggling + Save includes `enabled` in the config passed to `upsertMcpEntry`; a Claude MCP item does NOT show `mcp-enabled`.
- Full green bar.
- Commit: `feat(codex): Enabled toggle for Codex MCP servers in the form`.

## Notes for the implementer
- Everything Codex-write is greenfield (CCO never wrote Codex config). Match CCO only on the READ side (config.toml paths, both `mcp_servers`/`mcpServers` key spellings already handled by the existing `codex.rs` reader).
- `restore_mcp_file` (in `claude_mcp.rs`) is a generic whole-file restore under home — reuse it for the Codex `mcp-upsert` undo; do not write a Codex-specific copy.
- Keep the inner logic sync (unit tests stay fast). These are `#[tauri::command]`-reachable only through the existing sync `mcp_upsert_entry`/`delete_item`/`restore` commands (already registered) — no new commands needed for MCP; `skill_upsert` (Plan 19) already routes Codex via its `harness` arg.
