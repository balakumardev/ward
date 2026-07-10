# Plugins Mode Implementation Plan (Plan 28)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a 7th sidebar mode, **Plugins**, that browses Claude Code's plugin marketplaces, installs/uninstalls/updates plugins via the `claude plugin` CLI, and enables/disables installed plugins by directly editing `~/.claude/settings.json` — all with Ward's undo model.

**Architecture:** Hybrid, mirroring the Marketplace mode. Ward **reads** the on-disk plugin state (`~/.claude/plugins/**` JSON, `enabledPlugins`) directly for a fast browse/status UI; **writes** enable/disable as a surgical single-key flip to `settings.json` (undoable); and **shells out** to the `claude plugin …` CLI for install/uninstall/marketplace-add/update (the only supported path — it does git-clone + cache build). Every mutation is user-triggered and network is never a background poll.

**Tech Stack:** Rust (Tauri 2 `invoke` commands, `serde_json`, `ureq`, `which`), SolidJS + TS + Vite frontend, vitest, `spawn_blocking` for all IO/CLI.

## Global Constraints

- **No stubs / TODOs / placeholders.** Every function fully implemented and wired end-to-end (Ward quality bar).
- **TDD:** failing test → implement → green → commit. `cargo test`, `npm test`, `npx tsc --noEmit`, `cargo check` all pass before each commit.
- **Naming lock:** reuse existing names exactly — `WardError`, `Harness`, `Ctx`, `Registry`, `ClaudeAdapter`, `HarnessOps`, `RestoreInfo`, `ops_for`, `Capabilities`, `MODES`, `api`. New names introduced here are locked once defined (see per-task Interfaces).
- **Models:** all structs derive `Debug, Clone, Serialize, Deserialize, PartialEq` with `#[serde(rename_all="camelCase")]`.
- **Errors:** `thiserror` enum + the existing manual `impl Serialize` with `#[serde(tag="kind", content="message")]` + `rename_all="camelCase"` (`src-tauri/src/error.rs`).
- **Async commands:** any command doing file/network/CLI I/O is `pub async fn` wrapping `tauri::async_runtime::spawn_blocking(move || -> Result<T, WardError> {...})`. Inner logic stays sync for fast unit tests (Plan 17 rule).
- **WKWebView gotchas:** never `window.confirm/alert/prompt` — use an in-app modal (`role="dialog"`, `aria-modal`, backdrop, Esc/Enter, focus trap). Programmatic writes never touch FS from JS — only via `invoke`.
- **Frontend styling:** class-based via `src/styles/plugins.css` (imported by the component) + tokens from `src/styles/tokens.css`. No inline styles. Preserve/introduce `data-testid`s.
- **Secrets:** never write or log token values. Synthetic-only test fixtures (no real creds — GitHub push protection).
- **`enabledPlugins` shape:** an OBJECT keyed `"<name>@<marketplace>"` → bool. Absent key = enabled (subject to the plugin's own `defaultEnabled`). Never an array.
- **Reload caveat:** plugin changes need a Claude Code restart or `/reload-plugins`, which Ward cannot trigger — every mutation surfaces a "restart to apply" toast.
- **Reference templates (read these; port, don't reinvent):** `set_policy` + `restore_mcp_file` (`src-tauri/src/harness/adapters/claude_mcp.rs:130-187`), `write_json_pretty` (same file), `which_git`/`run_git` (`src-tauri/src/backup/git.rs:541-601`), existing `claude` subprocess (`src-tauri/src/security/judge.rs:37-73`), `scan_plugins`/`plugin_enabled_map` (`src-tauri/src/harness/adapters/claude.rs:728-798`), registry parse split (`src-tauri/src/marketplace/registry.rs`), the Marketplace UI (`src/modes/Marketplace.tsx`), command registration (`src-tauri/src/lib.rs:323-368`), `ops_for` (`src-tauri/src/commands.rs:56-62`).

---

### Task 1: Add `WardError::Plugin` variant + `pluginsManageable` capability

**Files:**
- Modify: `src-tauri/src/error.rs` (enum + `ErrorKind` serialize map)
- Modify: `src-tauri/src/model.rs:5-23` (`Capabilities` struct)
- Modify: `src-tauri/src/harness/adapters/claude.rs:172-179` (Claude capabilities → true)
- Modify: `src-tauri/src/harness/adapters/codex.rs:118-132` (Codex capabilities → false)
- Modify: `src/api.ts:35-46` (TS `Capabilities` mirror)

**Interfaces:**
- Produces: `WardError::Plugin(String)` → serializes `{"kind":"plugin","message":"..."}`. `Capabilities.plugins_manageable: bool` → wire `pluginsManageable`.

- [ ] **Step 1: Write the failing test** — in `src-tauri/src/error.rs` tests module:

```rust
#[test]
fn plugin_error_serializes_camel_case() {
    let e = WardError::Plugin("claude CLI not found".into());
    let s = serde_json::to_string(&e).unwrap();
    assert_eq!(s, r#"{"kind":"plugin","message":"claude CLI not found"}"#);
}
```

- [ ] **Step 2: Run to verify it fails** — `cd src-tauri && cargo test plugin_error_serializes_camel_case` → FAIL (no `Plugin` variant).

- [ ] **Step 3: Implement** — add `#[error("plugin error: {0}")] Plugin(String)` to the `WardError` enum, and a matching arm in the manual `ErrorKind` serialize map (follow the existing `Registry` arm exactly). In `model.rs` add `pub plugins_manageable: bool,` to `Capabilities` (camelCase via the struct's existing `rename_all`). In `claude.rs` capabilities literal set `plugins_manageable: true`; in `codex.rs` set `plugins_manageable: false`. In `src/api.ts` add `pluginsManageable: boolean;` to the `Capabilities` interface.

- [ ] **Step 4: Run to verify it passes** — `cargo test plugin_error_serializes_camel_case` → PASS; `cargo check` clean; `npx tsc --noEmit` clean.

- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat(plugins): add WardError::Plugin + pluginsManageable capability"`

---

### Task 2: Plugin wire models (`plugins/mod.rs`)

**Files:**
- Create: `src-tauri/src/plugins/mod.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod plugins;` near the other `pub mod` lines)

**Interfaces:**
- Produces:
  - `PluginEntry { kind: String /* always "plugin" */, name, marketplace, display_name, description, version, source: serde_json::Value, author, category, tags: Vec<String>, installed: bool, enabled: bool, scope: Option<String>, unique_installs: Option<u64>, always_on_tokens: Option<u64>, on_invoke_tokens: Option<u64>, component_counts: Option<ComponentCounts> }`
  - `ComponentCounts { commands: u64, agents: u64, skills: u64, hooks: u64, mcp_servers: u64, lsp_servers: u64 }`
  - `MarketplaceRef { name, source: serde_json::Value, install_location: Option<String>, last_updated: Option<String> }`
  - `PluginScan { marketplaces: Vec<MarketplaceRef>, plugins: Vec<PluginEntry>, cli_available: bool }`
  - `plugin_key(name, marketplace) -> String` = `format!("{name}@{marketplace}")`.

  > **Real on-disk shapes (verified on this machine, CC v2.1.206):** `installed_plugins.json` = `{version, plugins:{"<name>@<mkt>":[{scope,projectPath?,installPath,version,installedAt,lastUpdated}]}}`. `known_marketplaces.json` keyed by marketplace name → `{source:{source,repo},installLocation,lastUpdated}`. `plugin-catalog-cache.json` = `{version,fetchedAt,catalog:{generated_at,models:[…],plugins:{"<name>@<mkt>":{plugin,tokens:{"<model>":{always_on,on_invoke}},components:{commands[],agents[],skills[],hooks[],mcpServers[],lspServers[]},unique_installs,last_updated,marketplace_entry,version,source,…}}}}` (cache only covers `claude-plugins-official`).

- [ ] **Step 1: Write the failing test** — in `plugins/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plugin_entry_serializes_camel_case_and_key_joins() {
        let e = PluginEntry {
            kind: "plugin".into(), name: "code-formatter".into(),
            marketplace: "claude-plugins-official".into(),
            display_name: "Code Formatter".into(), description: "fmt".into(),
            version: Some("2.1.0".into()), source: serde_json::json!({"source":"github","repo":"a/b"}),
            author: Some("Anthropic".into()), category: Some("dev".into()),
            tags: vec!["fmt".into()], installed: true, enabled: false,
            scope: Some("user".into()), unique_installs: Some(1234),
            always_on_tokens: Some(1005), on_invoke_tokens: Some(15353),
            component_counts: Some(ComponentCounts { commands: 0, agents: 0, skills: 5, hooks: 0, mcp_servers: 1, lsp_servers: 0 }),
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("\"displayName\":\"Code Formatter\""));
        assert!(j.contains("\"uniqueInstalls\":1234"));
        assert!(j.contains("\"alwaysOnTokens\":1005"));
        assert!(j.contains("\"mcpServers\":1"));
        assert_eq!(plugin_key("code-formatter", "claude-plugins-official"),
                   "code-formatter@claude-plugins-official");
        let back: PluginEntry = serde_json::from_str(&j).unwrap();
        assert_eq!(e, back);
    }
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test plugin_entry_serializes` → FAIL (module missing).

- [ ] **Step 3: Implement** — define the three structs (standard derives + `rename_all="camelCase"`; use `#[serde(default, skip_serializing_if="Option::is_none")]` on the `Option` fields) and `pub fn plugin_key`. Add `pub mod plugins;` to `lib.rs`. Declare submodules `pub mod catalog; pub mod cli; pub mod enable;` (empty files created in later tasks — create stub files now with only `//! …` doc comments so the module compiles, OR add the `pub mod` lines in the tasks that create them; prefer the latter to keep each task's `cargo check` green).

- [ ] **Step 4: Run to verify it passes** — `cargo test plugin_entry_serializes` → PASS; `cargo check` clean.

- [ ] **Step 5: Commit** — `git commit -am "feat(plugins): wire models (PluginEntry, MarketplaceRef, PluginScan)"`

---

### Task 3: On-disk catalog readers (`plugins/catalog.rs`)

**Files:**
- Create: `src-tauri/src/plugins/catalog.rs`
- Create: `src-tauri/src/plugins/fixtures/installed_plugins.json`, `known_marketplaces.json`, `marketplace.json`, `plugin-catalog-cache.json` (synthetic)
- Modify: `src-tauri/src/plugins/mod.rs` (`pub mod catalog;`)

**Interfaces:**
- Consumes: models from Task 2.
- Produces (all pure, unit-tested against fixtures; a `home: &Path` reader wraps them):
  - `parse_installed(v: &serde_json::Value) -> HashMap<String /*key*/, InstalledInfo>` where `InstalledInfo { version, scope, install_path }` — collapse multi-install to newest by `lastUpdated`|`installedAt` (port the logic from `claude.rs:745`).
  - `parse_known_marketplaces(v: &serde_json::Value) -> Vec<MarketplaceRef>`.
  - `parse_marketplace_manifest(v: &serde_json::Value, marketplace_name: &str) -> Vec<PluginEntry>` (installed/enabled left false; filled by the merge step).
  - `parse_catalog_cache(v: &serde_json::Value) -> HashMap<String /*key*/, CatalogMeta>` where `CatalogMeta { unique_installs: Option<u64>, always_on_tokens: Option<u64>, on_invoke_tokens: Option<u64>, component_counts: Option<ComponentCounts> }`. Read `unique_installs` directly; `always_on_tokens`/`on_invoke_tokens` from `tokens[<first model in catalog.models>]` (`always_on`/`on_invoke`); `component_counts` from the length of each `components.<kind>` array (`commands/agents/skills/hooks/mcpServers/lspServers`).
  - `scan_plugins(home: &Path) -> PluginScan` — reads all four files + `enabledPlugins` (reuse `crate::harness::adapters::claude::plugin_enabled_map` — make it `pub(crate)`), merges into `PluginEntry` list (mark `installed`/`enabled`/`version`/`scope` from installed_plugins; `unique_installs`/token/component fields from the catalog-cache map), and sets `cli_available` from `crate::plugins::cli::claude_available()` (Task 5). Order: marketplace plugins first, then any installed plugin whose marketplace manifest is absent.

- [ ] **Step 1: Write failing tests** — cover each parser against the fixtures (e.g. `parse_installed_collapses_multi_install_to_newest`, `parse_known_marketplaces_reads_name_and_source`, `parse_marketplace_manifest_maps_fields`, `parse_catalog_cache_reads_installs_tokens_components`, and a `scan_plugins_merges_installed_and_enabled` integration test using a tempdir seeded with the fixtures + a `settings.json` carrying `enabledPlugins`). Use the verified realistic shapes: installed key `"code-formatter@claude-plugins-official"`; manifest `plugins:[{name,source,description,version,author,category,tags,displayName}]`; cache `catalog.plugins["<name>@claude-plugins-official"]` = `{plugin, unique_installs: N, tokens:{"claude-opus-4-7":{always_on,on_invoke}}, components:{skills:[…],mcpServers:[…],…}}` with `catalog.models:["claude-opus-4-7",…]`. Assert `parse_catalog_cache` reads `unique_installs`, the first-model `always_on`/`on_invoke`, and the per-kind component counts.

- [ ] **Step 2: Run to verify they fail** — `cargo test plugins::catalog` → FAIL.

- [ ] **Step 3: Implement** the parsers + `scan_plugins`. Make `plugin_enabled_map` `pub(crate)` in `claude.rs`. Missing file / bad JSON / wrong type → empty (never panic), matching `scan_plugins` tolerance at `claude.rs:732`.

- [ ] **Step 4: Run to verify they pass** — `cargo test plugins::catalog` → PASS.

- [ ] **Step 5: Commit** — `git commit -am "feat(plugins): on-disk catalog readers + merge into PluginScan"`

---

### Task 4: Remote marketplace fetch (`plugins/catalog.rs` addition)

**Files:**
- Modify: `src-tauri/src/plugins/catalog.rs`

**Interfaces:**
- Produces: `fetch_remote_marketplace(url: &str) -> Result<Vec<PluginEntry>, WardError>` — thin `ureq` GET (10s timeout, like `registry.rs`) → `parse_marketplace_manifest`. Reuse the pure parser from Task 3, so only the network wrapper is new (untested; the parse is already covered). `WardError::Plugin` on network/parse failure.

- [ ] **Step 1: Write the failing test** — assert the parse path via the existing fixture (the network wrapper itself isn't unit-tested, mirroring `registry.rs`'s split): `fetch_marketplace_uses_manifest_parser` calls `parse_marketplace_manifest` on the `marketplace.json` fixture and asserts ≥1 entry with expected name. (This locks the parser reuse; the `ureq` call is exercised in the hands-on smoke.)

- [ ] **Step 2: Run to verify it fails / passes** — parser test PASS (already implemented); add the `fetch_remote_marketplace` fn.

- [ ] **Step 3: Implement** `fetch_remote_marketplace` (ureq GET → `serde_json::from_str` → `parse_marketplace_manifest`, marketplace name derived from the URL or a `name` field in the manifest).

- [ ] **Step 4: Verify** — `cargo test plugins::catalog` PASS; `cargo check` clean.

- [ ] **Step 5: Commit** — `git commit -am "feat(plugins): fetch remote marketplace.json (user-triggered)"`

---

### Task 5: `claude` resolver + CLI wrappers (`plugins/cli.rs`)

**Files:**
- Create: `src-tauri/src/plugins/cli.rs`
- Modify: `src-tauri/src/plugins/mod.rs` (`pub mod cli;`)

**Interfaces:**
- Produces:
  - `which_claude() -> Option<PathBuf>` — robust GUI-app resolver (port `which_git` `git.rs:562-601`): try well-known absolute paths in order — `~/.claude/local/claude`, `/opt/homebrew/bin/claude`, `/usr/local/bin/claude`, `~/.local/bin/claude`, `~/.npm-global/bin/claude` — each verified by running `--version`; then fall back to `/usr/bin/env which claude` (accept only absolute). Cache in a `OnceLock`.
  - `claude_available() -> bool` = `which_claude().is_some()`.
  - Pure arg builders (unit-tested, no spawn): `install_args(plugin, marketplace, scope) -> Vec<String>` = `["plugin","install","<plugin>@<marketplace>","--scope","<scope>"]`; `uninstall_args(plugin, scope) -> Vec<String>` = `["plugin","uninstall","<plugin>","--scope","<scope>","-y"]`; `marketplace_add_args(src, scope)`, `marketplace_update_args(name: Option<&str>)`, `list_json_args()`, `marketplace_list_json_args()`.
  - Spawn wrappers: `run_claude(args: &[String]) -> Result<String /*stdout*/, WardError>` — resolve via `which_claude`, `Command::new(bin).args(args).env("PATH", …).output()`; non-zero exit → `WardError::Plugin(stderr)`; missing binary → `WardError::Plugin("claude CLI not found on PATH")`. Typed fns `install/uninstall/marketplace_add/marketplace_update` call `run_claude` with the arg builders.

- [ ] **Step 1: Write failing tests** — arg builders only (spawning real `claude` is out of scope for units, mirroring the scheduler `write_plist`/`load_plist` split): `install_args_pins_scope_and_key`, `uninstall_args_is_noninteractive` (asserts `-y` present), `marketplace_add_args_shape`. Plus `which_claude_returns_absolute_or_none` (asserts any result is absolute — tolerant of CI without `claude`).

- [ ] **Step 2: Run to verify they fail** — `cargo test plugins::cli` → FAIL.

- [ ] **Step 3: Implement** the resolver + builders + spawn wrappers.

- [ ] **Step 4: Verify** — `cargo test plugins::cli` PASS.

- [ ] **Step 5: Commit** — `git commit -am "feat(plugins): claude CLI resolver + install/uninstall/marketplace wrappers"`

---

### Task 6: Enable/disable via surgical settings flip (`plugins/enable.rs`)

**Files:**
- Create: `src-tauri/src/plugins/enable.rs`
- Modify: `src-tauri/src/plugins/mod.rs` (`pub mod enable;`)
- Modify: `src-tauri/src/harness/adapters/claude_ops.rs:313` (restore dispatch — add `"plugin-enable"` arm)

**Interfaces:**
- Produces: `set_plugin_enabled(home: &Path, plugin_key: &str, enabled: bool) -> Result<RestoreInfo, WardError>` — read `~/.claude/settings.json` prior bytes → set `enabledPlugins[plugin_key] = enabled` (create the object if absent) → `write_json_pretty` → `RestoreInfo { kind: "plugin-enable", original_path, backup_bytes: prior, mcp_key: Some(plugin_key), .. }`. **Port `set_policy` (`claude_mcp.rs:130-168`) structure exactly**, but note: enable/disable ALWAYS writes an explicit bool (there is no "empty ⇒ remove" case — disabling writes `false`, it does not delete the key). Restore arm: `"plugin-enable" => claude_mcp::restore_mcp_file(...)`.

- [ ] **Step 1: Write failing tests** — `set_enabled_flips_and_preserves_siblings` (seed a `settings.json` with `{"theme":"dark","enabledPlugins":{"x@m":true}}`, disable `x@m`, assert `theme` preserved and `x@m=false`), `set_enabled_creates_object_when_absent`, `restore_reverts_to_prior_bytes` (capture RestoreInfo, mutate, restore, assert byte-equal to original).

- [ ] **Step 2: Run to verify they fail** — `cargo test plugins::enable` → FAIL.

- [ ] **Step 3: Implement** `set_plugin_enabled` + the restore arm.

- [ ] **Step 4: Verify** — `cargo test plugins::enable` PASS.

- [ ] **Step 5: Commit** — `git commit -am "feat(plugins): enable/disable via surgical settings.json flip with undo"`

---

### Task 7: Tauri commands + registration

**Files:**
- Modify: `src-tauri/src/commands.rs` (new command fns near the marketplace block ~`:419-486`)
- Modify: `src-tauri/src/lib.rs:341-344` (`generate_handler!` list)

**Interfaces:**
- Produces (all `pub async fn` + `spawn_blocking`, resolving `home` via the existing ctx helper used by `mcp_upsert_entry`):
  - `plugins_scan() -> Result<PluginScan, WardError>`
  - `plugins_cli_available() -> Result<bool, WardError>`
  - `plugins_set_enabled(plugin_key: String, enabled: bool) -> Result<RestoreInfo, WardError>`
  - `plugins_install(plugin: String, marketplace: String, scope: String) -> Result<PluginScan, WardError>` (install then re-scan)
  - `plugins_uninstall(plugin: String, scope: String) -> Result<PluginScan, WardError>`
  - `plugins_marketplace_add(src: String, scope: String) -> Result<PluginScan, WardError>`
  - `plugins_marketplace_update(name: Option<String>) -> Result<PluginScan, WardError>`
- Consumes: `plugins::{catalog, cli, enable}`.

- [ ] **Step 1: Write the failing test** — a `cargo test` in `commands.rs` tests module asserting `plugins_scan`'s inner sync helper returns a `PluginScan` on a seeded tempdir (extract the sync body into `plugins::catalog::scan_plugins` already tested in Task 3; here assert the command wiring compiles + the enable command returns a RestoreInfo). Keep it light — the heavy logic is tested in Tasks 3/5/6.

- [ ] **Step 2: Run to verify it fails** — FAIL (commands missing).

- [ ] **Step 3: Implement** the commands (each `spawn_blocking` wrapping the sync `plugins::*` calls) and add all seven to `generate_handler!`.

- [ ] **Step 4: Verify** — `cargo test` PASS; `cargo check` clean.

- [ ] **Step 5: Commit** — `git commit -am "feat(plugins): tauri commands (scan, cli-available, enable, install, uninstall, marketplace add/update)"`

---

### Task 8: Frontend api.ts wrappers + types

**Files:**
- Modify: `src/api.ts` (invoke wrappers + TS types), `src/mock/*` (bridge answers)

**Interfaces:**
- Produces on `api`: `pluginsScan()`, `pluginsCliAvailable()`, `pluginsSetEnabled(pluginKey, enabled)`, `pluginsInstall(plugin, marketplace, scope)`, `pluginsUninstall(plugin, scope)`, `pluginsMarketplaceAdd(src, scope)`, `pluginsMarketplaceUpdate(name?)`. TS types `PluginEntry`, `MarketplaceRef`, `PluginScan` mirroring the Rust wire shapes (camelCase).

- [ ] **Step 1: Write the failing test** — vitest `src/__tests__/plugins-api.test.ts` asserting `api.pluginsScan` invokes `"plugins_scan"` and `api.pluginsSetEnabled("x@m", false)` invokes `"plugins_set_enabled"` with `{pluginKey:"x@m", enabled:false}` (mock `invoke`).

- [ ] **Step 2: Run to verify it fails** — `npm test plugins-api` → FAIL.

- [ ] **Step 3: Implement** the wrappers + types; extend the mock bridge (`src/mock/`) with an in-memory plugin store seeded from a fixture answering all seven commands (enable/disable mutates the store; install adds an entry; `cli_available: true`).

- [ ] **Step 4: Verify** — `npm test plugins-api` PASS; `npx tsc --noEmit` clean.

- [ ] **Step 5: Commit** — `git commit -am "feat(plugins): api.ts wrappers + mock bridge"`

---

### Task 9: Sidebar mode + App render (capability-gated scaffold)

**Files:**
- Modify: `src/components/Sidebar.tsx:3-10` (add `{ id: 'plugins', label: 'Plugins', icon: '⧉' }` — pick an unused glyph), `src/App.tsx:175-224` (nested `<Show when={mode()==='plugins'}>`)
- Create: `src/modes/Plugins.tsx` (scaffold), `src/styles/plugins.css`

**Interfaces:**
- Consumes: `capabilities.pluginsManageable`, `api`.
- Produces: `<Plugins scan api />` rendered when mode is `plugins`; a "not applicable" panel when `!pluginsManageable` (Codex), matching the Marketplace/Backups gating pattern.

- [ ] **Step 1: Write the failing test** — vitest `renders plugins mode, shows unsupported panel for codex` (render App/Plugins with `pluginsManageable:false` → assert an unsupported message; with true → assert the mode container `data-testid="plugins-mode"`).

- [ ] **Step 2–4:** scaffold `Plugins.tsx` returning `<div data-testid="plugins-mode">` (tabs added next task), wire Sidebar + App gate, style shell in `plugins.css`. `npm test` PASS, `tsc` clean.

- [ ] **Step 5: Commit** — `git commit -am "feat(plugins): sidebar mode + capability-gated App render"`

---

### Task 10: Plugins.tsx — Discover tab (browse + install)

**Files:**
- Modify: `src/modes/Plugins.tsx`, `src/styles/plugins.css`

**Interfaces:**
- Consumes: `api.pluginsScan`, `api.pluginsInstall`, `api.pluginsMarketplaceAdd`, `api.pluginsCliAvailable`.

- [ ] **Step 1: Write failing tests** — vitest: `discover lists available plugins with source badge`, `install opens in-app confirm modal then calls pluginsInstall`, `cli-absent banner shows when pluginsCliAvailable is false and disables Install`. Use the mock bridge.

- [ ] **Step 2–4:** implement the Discover tab — marketplace picker (from `scan.marketplaces` + an "Add marketplace" field → `pluginsMarketplaceAdd`, gated by confirm modal), search box (client-side filter over name/description like `filter_market_page`), plugin cards (reuse Marketplace card classes + a source badge `data-testid="plugin-source"`), an **in-app confirm modal** (`data-testid="plugin-install-confirm"`, `role="dialog"`) → `pluginsInstall` → toast "Installed — restart Claude Code or run /reload-plugins to apply" (`data-testid="plugin-reload-toast"`). CLI-absent banner (`data-testid="plugin-cli-banner"`). All `npm test` PASS, `tsc` clean, `vite build` clean.

- [ ] **Step 5: Commit** — `git commit -am "feat(plugins): Discover tab — browse marketplaces + gated install"`

---

### Task 11: Plugins.tsx — Installed tab (enable/disable/uninstall/update)

**Files:**
- Modify: `src/modes/Plugins.tsx`, `src/styles/plugins.css`

**Interfaces:**
- Consumes: `api.pluginsSetEnabled`, `api.pluginsUninstall`, `api.pluginsMarketplaceUpdate`.

- [ ] **Step 1: Write failing tests** — `installed tab toggles enable calls pluginsSetEnabled`, `uninstall opens confirm modal then calls pluginsUninstall`, `disabled plugin renders as disabled`.

- [ ] **Step 2–4:** implement the Installed tab — list of `scan.plugins.filter(installed)` with an enable/disable **toggle** (`data-testid="plugin-enable-toggle"`) → `pluginsSetEnabled(plugin_key, next)` → reload toast; **Uninstall** button → confirm modal → `pluginsUninstall`; **Update** button → `pluginsMarketplaceUpdate`. Undo affordance surfaced via the returned `RestoreInfo` reusing the existing Organizer undo toast pattern if present, else a simple toast. `npm test` PASS, `tsc` + `vite build` clean.

- [ ] **Step 5: Commit** — `git commit -am "feat(plugins): Installed tab — enable/disable/uninstall/update"`

---

### Task 12: Polish + full green gate

**Files:** `src/styles/plugins.css`, any small fixes.

- [ ] **Step 1:** run the full suite — `cd src-tauri && cargo test && cargo check`, then `cd .. && npm test && npx tsc --noEmit && npm run build`. All green.
- [ ] **Step 2:** `src-tauri/target/debug/ward --scan --harness claude` still emits valid JSON (headless unaffected).
- [ ] **Step 3:** visual polish pass in `npm run dev:mock` via Chrome DevTools MCP — tab switch, card layout, confirm modal, toggle, banner, toast. Fix any layout/token issues.
- [ ] **Step 4: Commit** — `git commit -am "chore(plugins): polish + full green gate"`

---

## Self-Review (author checklist — done)

- **Spec coverage:** browse/search official marketplaces (Tasks 3,4,10) ✓; add plugins / install (Tasks 5,7,10) ✓; remove (Tasks 5,7,11) ✓; enable/disable (Tasks 6,7,11) ✓; proper UI/UX + capability gate (Tasks 9-12) ✓; undo (Task 6) ✓; CLI-absent + reload caveats (Tasks 10,11) ✓.
- **Type consistency:** `plugin_key` join, `PluginEntry`/`MarketplaceRef`/`PluginScan`, `set_plugin_enabled`, `which_claude`, and the seven `plugins_*` commands are referenced identically across tasks.
- **Placeholders:** none — every task names exact files, the test, and the port target (`file:line`). Large ports (parsers, CLI, components) cite the template to match rather than re-inlining hundreds of lines, per Ward's CCO-parity plan convention.
