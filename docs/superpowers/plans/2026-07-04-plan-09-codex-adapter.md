# Ward Plan 09 — Codex Adapter (High-Level)

> High-level plan. Use **superpowers:subagent-driven-development**, TDD, commit per task. CCO reference (read-only): `/Users/balakumar/personal/claude-code-organizer`.

**Goal:** Add the **second harness** — Codex CLI — with capability-gated parity, proving the `Harness` trait generalizes.

**Builds on:** `Harness` trait + `Registry` (Plan 01), `run_scan`, the harness-param `scan` command, `Sidebar` harness switcher.

**Files:**
- Create `src-tauri/src/harness/adapters/codex.rs`: `id "codex"`, `executable "codex"`, **11 categories** (`config, memory, skill, mcp, profile, rule, plugin, session, history, shell, runtime`); parse `~/.codex/config.toml` via the `toml` crate; **capabilities**: `mcp_security / sessions / backup = true`, `context_budget / mcp_controls / mcp_policy / effective = false`.
- Modify `src-tauri/src/harness/adapters/mod.rs` + `commands.rs::build_registry`: register `CodexAdapter`.
- Add `toml = "0.8"` to `src-tauri/Cargo.toml`.
- Frontend: wire the `Sidebar` harness switcher to re-`scan` the selected harness; make the UI **capability-driven** (hide Budget / Effective / MCP-controls when `ScanResult.capabilities.*` is false).

**Task checklist:**
- [ ] Codex paths + scopes (`~/.codex/` global + project `<repo>/.codex/config.toml`, `AGENTS.md`, `.agents/skills`).
- [ ] `config.toml` parsing (model / sandbox / approval / profiles + `[mcp_servers]`).
- [ ] Per-category scanners for the 11 Codex categories.
- [ ] Register adapter; frontend harness switch + capability-gating.

**CCO parity refs:** `src/harness/adapters/codex.mjs`. **Port golden:** `tests/unit/test-codex-adapter.mjs`.

**Tests:** parse a fixture `config.toml`; 11 categories discovered; capabilities exactly `{mcpSecurity,sessions,backup}=true` else false; switching harness re-scans.

**Gotchas:** UI must read `ScanResult.capabilities` and hide unsupported modes (Codex has no effective/budget/mcp-controls); Codex `config.toml` **non-MCP settings** (model/sandbox/approval/profiles) are a differentiator — surface them, don't drop them.
