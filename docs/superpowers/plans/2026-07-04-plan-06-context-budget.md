# Ward Plan 06 — Context Budget + Tokenizer (High-Level)

> High-level plan. Use **superpowers:subagent-driven-development**, TDD, commit per task. CCO reference (read-only): `/Users/balakumar/personal/claude-code-organizer`.

**Goal:** Show per-scope **context-window** budget composition (tokens, not $) — system overhead + per-item/category token breakdown.

**Builds on:** categories (Plan 02), `commands.rs`, `ScanResult`.

**Files:**
- Create `src-tauri/src/tokenizer.rs`: count tokens via `tiktoken-rs`; fallback to `bytes/4`; report `measured` vs `estimated`.
- Create `src-tauri/src/harness/adapters/claude_budget.rs`: overhead constants + `@import` expansion + per-item composition. Constants (port verbatim, they are intentionally rounded): `SYSTEM_LOADED≈18000`, `SYSTEM_DEFERRED≈7000`, `MCP_TOOL_SCHEMA≈3100`/unique server, `CLAUDEMD_WRAPPER≈100`, `AUTOCOMPACT_BUFFER≈13000`, `WARNING_THRESHOLD≈20000`, `MAX_OUTPUT≈32000`. Always-loaded categories = skill, rule, command, agent.
- Modify `src-tauri/src/commands.rs`: `context_budget(harness, scope)`.
- Create frontend `src/modes/Budget.tsx`: token meter + per-category/per-item breakdown + warning threshold.

**Task checklist:**
- [ ] Tokenizer with measured/estimated flag + `bytes/4` fallback.
- [ ] `expand_imports` for CLAUDE.md `@import` (depth ≤ 5, verbatim merge).
- [ ] Budget composition: sum system overhead + always-loaded items + MCP schema tokens per unique server + CLAUDE.md.
- [ ] `context_budget` command + `modes/Budget.tsx` UI.

**CCO parity refs:** `src/harness/adapters/claude-context-budget.mjs` (exact constants + `@import` + composition), `src/tokenizer.mjs`; endpoint `/api/context-budget`.

**Tests:** `@import` expansion (incl. depth cap); per-category totals; MCP schema counted once per unique server; measured-vs-estimated flag.

**Gotchas:** constants drift each Claude Code release — keep rounded, don't over-tune; tokenizer optional (degrade to bytes/4); only Claude advertises `contextBudget:true` (Codex hides this mode).
