# Ward Plan 02 — Full Claude Categories + Show Effective (High-Level)

> High-level plan. Implementer: use **superpowers:subagent-driven-development**, work TDD (write the failing test first), commit per task, and **read the referenced CCO source for exact behavior**. CCO reference repo (read-only, never copy verbatim): `/Users/balakumar/personal/claude-code-organizer`.

**Goal:** Grow the Claude adapter from 2 categories (skill, memory) to all **12**, add **project-scope** discovery, and implement **Show Effective** resolution (shadow / conflict / ancestor) with a UI toggle.

**Builds on (Plan 01 real types):** `Harness::scan_category`, `Harness::discover_scopes`, `ClaudeAdapter`, `run_scan`, `ScanResult`/`HarnessItem`, `Registry`, frontend `Organizer`, `api.scan`.

**Files:**
- Modify `src-tauri/src/harness/adapters/claude.rs`: `category_ids()` → `["skill","memory","mcp","command","agent","plan","rule","config","hook","plugin","session","setting"]`; add a `scan_category` arm per new category; extend `discover_scopes` to add **project** scopes.
- Create `src-tauri/src/harness/fs_utils` additions (or `harness/fs_utils.rs`): decode `~/.claude/projects/<encoded>` dir names → real project path (handles non-ASCII / lossy / symlinked paths).
- Create `src-tauri/src/effective.rs`: per-category shadow/conflict/ancestor resolution over the item set.
- Modify `src-tauri/src/commands.rs`: fold an `effective` field into `ScanResult` (or add `effective(harness)` command).
- Frontend: add a **Show Effective** toggle + shadowed/conflict badges in `modes/Organizer.tsx`.

**Task checklist:**
- [ ] Project-scope discovery: list `~/.claude/projects/*`, decode → `Scope{kind:"project"}` with `<repo>/.claude/*` + `<repo>/.mcp.json` + `<repo>/CLAUDE.md` roots. (Port CCO `src/harness/fs-utils.mjs` decode + `claude.mjs` scope logic.)
- [ ] Category scanners (paths per CCO `claude.mjs`): mcp (`~/.claude.json` + `<repo>/.mcp.json` + settings), command (`commands/`), agent (`agents/`), plan (`plans/`), rule (`rules/`), config (settings files + CLAUDE.md variants), hook (parsed from `settings.json` — **read-only**), plugin (`plugins/cache/`), session (`projects/<enc>/*.jsonl`), setting (`settings.json`/`settings.local.json`).
- [ ] Effective resolution: port CCO `src/effective.mjs` rules; expose per-item `effective` status (active / shadowed-by / conflict).
- [ ] Frontend: Show-Effective toggle + badges.

**CCO parity refs:** `src/harness/adapters/claude.mjs` (categories + scanners), `src/effective.mjs`, `src/harness/fs-utils.mjs`. **Port these tests as golden:** `tests/unit/test-effective-rules.mjs`, `tests/unit/test-claude-adapter-regressions.mjs`, `tests/unit/test-path-correctness.mjs`.

**Tests:** one fixture per category scanner; project-scope decode (incl. a non-ASCII path); effective shadow + conflict cases.

**Gotchas:** hooks are read-only (parsed from settings, not files) → `movable:false`; encoded-path decoding has real edge cases — port CCO's logic and its tests; some items are `locked` (root `CLAUDE.md`, managed skills).
