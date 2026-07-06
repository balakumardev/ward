# Plan 21 — Marketplace: MCP Servers — Implementation Plan

> **For agentic workers:** implement task-by-task with TDD. Per-task reviews are skipped for this run (user directive); one whole-branch review at the very end. NO stubs/TODOs/placeholders — production-grade, full green bar before finishing, one conventional commit per task.

**Goal:** A new 6th sidebar mode, **Marketplace**, that searches the official MCP Registry and installs MCP servers into the chosen harness(es) × scope(s) via the SAME `mcp_upsert_entry` engine (install-once-to-many). Skills tab is Plan 22 — this plan does MCP servers only, but the mode + tab scaffolding must leave a clean seam for it.

**Reference (authoritative):** design spec `docs/superpowers/specs/2026-07-06-ward-mcp-marketplace-design.md` §9 (Marketplace mode: data models §9.1, registry client §9.2, install fan-out §9.4, security posture §9.5, frontend §9.6), §10 (command/api additions). The official MCP Registry response shape: `GET https://registry.modelcontextprotocol.io/v0/servers` → `{ servers: [ server.json ], metadata: { nextCursor } }` where each `server.json` has `name`, `description`, `version`, `packages[]{registryType npm|pypi|oci, identifier, version, transport{type stdio|http|sse}, environmentVariables[]{name,isRequired,isSecret}, runtimeHint?}`, `remotes[]{type, url, headers[]{name,isRequired,isSecret}}`.

**Prerequisite already on branch:** Plan 18 `mcp_upsert_entry` command + `HarnessOps::upsert_mcp_entry` + `mcpEditable` capability + `check_policy`/`mcp_get_policy`. `ureq` is already a dependency. This plan reuses all of it — install = fan-out to `upsert_mcp_entry`.

## Global Constraints

- Reuse names verbatim: `WardError`, `ScanResult`, `Scope`, `McpPolicy`, `check_policy`, `mcp_get_policy`, `MODES`, `Organizer`, `Sidebar`, `App`. New error variant: `WardError::Registry(String)`.
- All structs derive `Debug, Clone, Serialize, Deserialize, PartialEq` + `#[serde(rename_all = "camelCase")]`. Frontend ↔ core ONLY via `invoke`.
- **Network is user-triggered only** (search on user action, install on click) — NEVER a background poll. Reuse `ureq` (rustls, blocking). Split every fetcher into a thin network wrapper (NOT unit-tested) + a **pure parse fn** (fully unit-tested against a pinned fixture) — exactly like `usage/live.rs` (`fetch_ratelimit_headers` vs `snapshot_from_headers`).
- **Security (enforce, spec §9.5):** version-pin (NEVER `@latest`, never empty version — `build_mcp_config` rejects with `WardError::Registry`); show exact `command`/`args`/`env` (or `url`/`headers`) before install; run `check_policy` and surface the verdict; **never collect secret values** — for `EnvVar{isSecret:true}` render the var NAME read-only with a note and write an EMPTY string (or omit), never a field to type a token.
- New UI class-based via `src/styles/marketplace.css` + tokens — NOT inline styles. Preserve every existing `data-testid`.
- **Fixtures SYNTHETIC only** (no real tokens — release CI scans every blob). `cargo test` from `src-tauri/`; `npm test`/`tsc`/`build` from repo root; all green before finishing. One conventional commit per task.

## Tasks

### Task 1 — `WardError::Registry` + `marketplace/mod.rs` data models
- `src-tauri/src/error.rs`: add `Registry(String)` variant to `WardError` enum, the `ErrorKind` enum, and the `serialize` match (mirror the existing `Live` variant exactly — `#[error("registry error: {0}")]`, camelCase tag `registry`). Add a serialize test.
- Create `src-tauri/src/marketplace/mod.rs` with `pub mod registry; pub mod install;` and the types from spec §9.1: `MarketEntry`, `Package`, `EnvVar`, `Remote`, `InstallTarget`, `BuiltConfig`, `InstallResult`, and a `MarketPage { entries: Vec<MarketEntry>, next_cursor: Option<String> }`. Register `pub mod marketplace;` in `src-tauri/src/lib.rs`.
- Commit: `feat(marketplace): WardError::Registry + marketplace data models`.

### Task 2 — Registry client: pure `parse_servers` + `ureq` `fetch_servers`
- `src-tauri/src/marketplace/registry.rs`:
  - `pub fn parse_servers(body: &str) -> Result<MarketPage, WardError>` — PURE; parse the official registry JSON into `Vec<MarketEntry>` (kind `"mcp"`) + `next_cursor`. Map `packages[]`/`remotes[]`, `environmentVariables` → `EnvVar{name,is_required,is_secret}`, mark `verified: true` (registry-listed). Tolerate missing `packages`/`remotes`/`metadata`. Fully unit-tested against a pinned SYNTHETIC fixture `src-tauri/src/marketplace/fixtures/registry-servers.json` (include a stdio npm package server, a pypi server, and a remote/http server; `include_str!`).
  - `pub fn fetch_servers(cursor: Option<&str>) -> Result<MarketPage, WardError>` — thin `ureq` GET of `https://registry.modelcontextprotocol.io/v0/servers?limit=100[&cursor=]`, 10s timeout, map transport/HTTP errors to `WardError::Registry`, then `parse_servers(&body)`. NOT unit-tested (network).
- Commit: `feat(marketplace): official MCP Registry client (pure parse + ureq fetch)`.

### Task 3 — `build_mcp_config` (version-pin + secret-safe) + install fan-out
- `src-tauri/src/marketplace/install.rs`:
  - `pub fn build_mcp_config(entry: &MarketEntry, package_index: usize, env_values: &HashMap<String,String>) -> Result<BuiltConfig, WardError>` — PURE. For a `packages[package_index]`: stdio → `{command, args, env}` with the runner per `registryType` (npm → `command:"npx", args:["-y", "<identifier>@<version>"]`; pypi → `command:"uvx", args:["<identifier>@<version>"]` (or `==<version>` form — pick one and test it); oci → surface but do not fabricate a docker run). **Reject** any resolved version that is empty or `latest` (`WardError::Registry("refusing to install an unpinned version")`). `env` keys from `EnvVar`: `is_secret:true` → write `""` (or omit) and record it in `BuiltConfig` as secret-needed; `is_secret:false` → use `env_values` value. If `entry` has no packages but has `remotes[remote_index]` → `{url, type, headers}` (headers from non-secret values). `command_preview` = the flattened `[command, ...args]` (or the url). Fully unit-tested: npm pin, pypi pin, latest-rejection, secret omission, remote shape.
  - `pub fn install(entry, package_index, targets: &[InstallTarget], env_values) -> Vec<InstallResult>` — for each target, `build_mcp_config` then dispatch `ops_for(&target.harness)?.upsert_mcp_entry(&ctx, &target.scope_id, &entry_name, &config, None, &scopes)`; collect `InstallResult{ target, ok, error?, restore? }`. (Reuse `harness_ctx`.) Do NOT abort the batch on one failure.
- Golden tests for `build_mcp_config`. `install` is integration-tested lightly via the ClaudeOps path against a temp home (mirror the commands.rs test style).
- Commit: `feat(marketplace): build_mcp_config (version-pinned, secret-safe) + install fan-out`.

### Task 4 — `marketplace_*` commands + api.ts + mock
- `src-tauri/src/commands.rs`: `#[tauri::command] pub fn marketplace_search(kind, query, cursor: Option<String>) -> Result<MarketPage, WardError>` (fetch + filter by query substring over name/description); `marketplace_build_config(entry, package_index, env_values) -> BuiltConfig`; `marketplace_install(entry, package_index, targets, env_values) -> Vec<InstallResult>`. Register all three in `lib.rs` `generate_handler!`.
- `src/api.ts`: add the TS interfaces (`MarketEntry`, `Package`, `EnvVar`, `Remote`, `InstallTarget`, `BuiltConfig`, `InstallResult`, `MarketPage`) mirroring the Rust camelCase, and wrappers `marketplaceSearch`, `marketplaceBuildConfig`, `marketplaceInstall`.
- Mock: `src/mock/dispatch.ts` cases + `src/mock/store.ts` methods (`marketplaceSearch` returns a synthetic fixture list filtered by query; `marketplaceBuildConfig` builds a preview; `marketplaceInstall` returns per-target ok results and, for Claude/global, pushes a new MCP item into the scan so the Organizer reflects it) + a `src/mock/fixtures.ts` `MARKET_ENTRIES` synthetic list (stdio npm, pypi, remote).
- Tests: `api.test.ts` (each wrapper invokes the right command with camelCase args), `store.test.ts` (marketplaceInstall to Claude global adds an MCP item; search filters).
- Commit: `feat(marketplace): marketplace_search/build_config/install commands + api + mock`.

### Task 5 — Marketplace mode (Sidebar + App + Marketplace.tsx)
- `src/components/Sidebar.tsx`: append `{ id: 'marketplace', label: 'Marketplace', icon: '◱' }` to `MODES`. **Update `src/components/Sidebar.test.tsx`** (it hard-asserts the mode id list) to include `'marketplace'`.
- `src/App.tsx`: add a `<Show when={mode() === 'marketplace'}>` rung rendering `<Marketplace scan={result()} api={...} />`; import it. Wire an api bridge exposing `marketplaceSearch`/`marketplaceBuildConfig`/`marketplaceInstall` + `refetch` (so an install re-scans the Organizer).
- Create `src/modes/Marketplace.tsx` (full-width own shell class) + `src/styles/marketplace.css`:
  - Tabs `market-tab-mcp` / `market-tab-skills` (skills tab present but shows a "coming soon"/empty state — Plan 22 fills it). Search box `market-search` → cards `market-card` (name, description, `verified` badge, version).
  - Card → detail sheet `market-detail`: package/transport picker `market-pkg-pick`, env-var rows `market-env-row` (secret rows read-only with the note, non-secret editable), **exact command/args/env preview `market-preview`**, policy verdict `market-policy-verdict` (call `mcp_check_policy` with the current policy), and the install target matrix: `market-target-claude`, `market-target-codex` × `market-target-scope` (checkbox grid, data-driven from the scan's scopes + a static harness list so Claude Desktop can be appended later). Install button `market-install` → `marketplaceInstall` → per-target toast.
- Tests (`src/modes/Marketplace.test.tsx`): search renders cards; selecting a card shows the command/args preview + policy verdict; toggling targets + Install calls `marketplaceInstall` with the selected `targets`; a secret env var renders read-only (no typeable value).
- Full green bar (`npm test`, `tsc`, `build`, `cargo test`).
- Commit: `feat(marketplace): Marketplace mode UI (search → detail → install matrix)`.

## Notes for the implementer
- The Marketplace mode is greenfield (CCO has no marketplace). Model the full-width shell on `BudgetWithPicker` (`src/modes/Budget.tsx`, `<div class="bud-shell">`).
- Install fan-out reuses the EXISTING `mcp_upsert_entry` path — do NOT write a second MCP writer.
- Keep the target list a data structure (`InstallTarget[]`) so a future `harness:"claude-desktop"` slots in without a rewrite (spec §12).
- The Skills tab is a seam only — Plan 22 implements skill search/install. Leave `market-tab-skills` rendering a clean empty state, not a stub that errors.
