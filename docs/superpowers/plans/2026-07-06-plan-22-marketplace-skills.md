# Plan 22 — Marketplace: Skills — Implementation Plan

> **For agentic workers:** implement task-by-task with TDD. Per-task reviews skipped for this run; one whole-branch review at the end. NO stubs/TODOs/placeholders — production-grade, full green bar before finishing, one conventional commit per task.

**Goal:** Fill the Marketplace's **Skills** tab (the seam Plan 21 left): search a curated set of Claude marketplace repos / GitHub `SKILL.md` sources, then install a skill into the chosen harness(es) × scope(s) via the SAME `skill_upsert` engine (install-once-to-many). Completes the two-unit Marketplace (MCP servers + Skills).

**Reference (authoritative):** design spec `docs/superpowers/specs/2026-07-06-ward-mcp-marketplace-design.md` §9.3 (skills catalog), §9.4 (install fan-out), §9.6 (frontend). Claude marketplace shape: a repo's `.claude-plugin/marketplace.json` lists plugins/skills; each skill resolves to a `SKILL.md` (frontmatter `name`/`description` + body).

**Prerequisites already on branch (post-merge of streams CONFIG + MARKET):**
- `skill_upsert(harness, scope_id, name, content)` command + `api.skillUpsert` + `skillCreatable` capability (Plan 19).
- `marketplace/{mod,registry,install}.rs`, `MarketEntry` (with `kind`, `repo_url`, `skill_path`), `marketplace_search(kind, query, cursor)`, `marketplace_install(entry, package_index, targets, env_values)`, the Marketplace mode with a `market-tab-skills` empty-state seam, and the install target matrix (Plan 21).

## Global Constraints

- Reuse names verbatim: `WardError` (`Registry` variant), `MarketEntry`, `MarketPage`, `InstallTarget`, `InstallResult`, `skill_upsert`, `marketplace_search`, `marketplace_install`, `Marketplace`. 
- Network user-triggered only; every fetcher = thin `ureq` wrapper (not unit-tested) + PURE parse fn (unit-tested against a pinned SYNTHETIC fixture).
- Skill install writes via `skill_upsert` (create-only; refuse clobber). Bind approval to the SKILL.md content shown before install (spec §9.5 — show the frontmatter/body preview).
- Fixtures SYNTHETIC only. New UI class-based (`marketplace.css`), preserve testids. `cargo test` from `src-tauri/`; `npm test`/`tsc`/`build` from repo root; all green. One conventional commit per task.

## Tasks

### Task 1 — `marketplace/skills.rs`: pure `parse_marketplace` + `ureq` fetchers
- Create `src-tauri/src/marketplace/skills.rs`:
  - `pub const CURATED_MARKETPLACES: &[(&str,&str)]` — a small curated list of `(display_name, raw marketplace.json URL)` for trusted Claude skill/plugin repos (e.g. the official superpowers/anthropics marketplace raw URLs). In-binary, no network to discover.
  - `pub fn parse_marketplace(body: &str, repo_url: &str) -> Result<Vec<MarketEntry>, WardError>` — PURE; parse a `.claude-plugin/marketplace.json` into `MarketEntry`s of `kind:"skill"` with `name`, `display_name`, `description`, `source:"marketplace"`, `repo_url`, `skill_path` (the SKILL.md path within the repo), `verified:true`. Tolerate plugin bundles that ship multiple skills (unpack each into its own entry). Fully unit-tested against a pinned synthetic `src-tauri/src/marketplace/fixtures/marketplace.json`.
  - `pub fn parse_skill_md_meta(body: &str) -> (String /*name*/, String /*description*/)` — PURE; extract frontmatter `name`/`description` from a SKILL.md (reuse the existing frontmatter parsing approach in the codebase if one exists; else a minimal parser). Unit-tested.
  - `pub fn fetch_marketplace(url: &str) -> Result<Vec<MarketEntry>, WardError>` and `pub fn fetch_skill_md(raw_url: &str) -> Result<String, WardError>` — thin `ureq` wrappers (not unit-tested), 10s timeout, errors → `WardError::Registry`.
- Register `pub mod skills;` in `src-tauri/src/marketplace/mod.rs`.
- Commit: `feat(marketplace): skills catalog (marketplace.json + SKILL.md pure parse + fetch)`.

### Task 2 — Wire skills into `marketplace_search` + skill install into `marketplace_install`
- Extend `marketplace_search(kind, query, cursor)` (commands.rs): when `kind == "skill"`, aggregate `skills::CURATED_MARKETPLACES` via `fetch_marketplace`, flatten, filter by query substring over name/description, return a `MarketPage` (cursor `None`). Keep the `kind == "mcp"` path unchanged.
- Extend `marketplace_install(entry, package_index, targets, env_values)` (install.rs): when `entry.kind == "skill"`, for each target `fetch_skill_md(entry.skill_path-or-raw-url)` → `ops`/command path `skill_upsert(target.harness, target.scope_id, entry.name, content)`; collect `InstallResult`. Keep the mcp branch unchanged. (Reuse the existing `skill_upsert` core fn — do NOT write a second skill writer.)
- Tests: `parse`/filter for skills search (pure/mock); a skill-install integration test against a temp home writing a SKILL.md via the real `skill_upsert` (mirror the mcp install test).
- Commit: `feat(marketplace): skill search + install fan-out via skill_upsert`.

### Task 3 — api.ts + mock for skills marketplace
- `src/api.ts`: `marketplaceSearch`/`marketplaceInstall` already exist (Plan 21) and take `kind` — no signature change; ensure `MarketEntry` TS type carries `repoUrl`/`skillPath` (add if missing).
- Mock: `src/mock/store.ts` `marketplaceSearch('skill', ...)` returns a synthetic skills fixture; `marketplaceInstall` for a skill entry to a target pushes a new `skill` item into that harness's scan (so the Organizer reflects it). Add `MARKET_SKILLS` synthetic fixture to `src/mock/fixtures.ts`. Dispatch cases already route (Plan 21) — extend the store methods only.
- Tests (`store.test.ts`): skills search returns entries; installing a skill to Claude global adds a skill item.
- Commit: `feat(marketplace): mock skill search + install`.

### Task 4 — Marketplace Skills tab UI
- `src/modes/Marketplace.tsx`: replace the `market-tab-skills` empty-state seam with a real skills view — reuse the search box, cards (`market-card` with the skill name/description/verified badge), and the install target matrix (`market-target-claude`/`market-target-codex` × `market-target-scope`). Card detail (`market-detail`) shows the SKILL.md frontmatter/body preview (`market-preview`) BEFORE install (bind approval to content). Install → `marketplaceInstall(entry, 0, targets, {})` → per-target toast.
- Tests (`Marketplace.test.tsx`): switching to the skills tab lists skill cards; selecting one shows the SKILL.md preview; Install calls `marketplaceInstall` with the skill entry + selected targets.
- Full green bar (`npm test`, `tsc`, `build`, `cargo test`).
- Commit: `feat(marketplace): Skills tab UI (search → SKILL.md preview → install matrix)`.

## Notes for the implementer
- Reuse everything: the target matrix, cards, search box, and toast are Plan 21's — the skills tab is a second data source through the SAME components, not a parallel UI.
- Install reuses `skill_upsert` (create-only). If a target already has a skill by that name, surface the per-target error in `InstallResult` (don't abort the batch).
- If the codebase already has a SKILL.md frontmatter parser (grep `frontmatter`/`SKILL.md` in `src-tauri/src`), reuse it instead of writing a new one.
