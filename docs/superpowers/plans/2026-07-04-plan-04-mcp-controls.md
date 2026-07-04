# Ward Plan 04 — MCP Controls: enable/disable + policy (High-Level)

> High-level plan. Use **superpowers:subagent-driven-development**, TDD, commit per task. CCO reference (read-only): `/Users/balakumar/personal/claude-code-organizer`.

**Goal:** Enable/disable MCP servers **per project** (disabled list) and manage an **allow/deny policy** in user settings.

**Builds on:** `mcp` category (Plan 02), `commands.rs`, mutation/JSON-edit patterns (Plan 03).

**Files:**
- Create `src-tauri/src/harness/adapters/claude_mcp.rs`: `get_disabled_servers(&Ctx,scope)`, `set_disabled_servers(&Ctx,scope,list)`, `get_policy(&Ctx)`, `set_policy(&Ctx,policy)`.
- Modify `src-tauri/src/commands.rs`: `mcp_get_disabled`, `mcp_set_disabled`, `mcp_get_policy`, `mcp_set_policy`.
- Frontend: enabled/disabled toggle on MCP items in `Organizer`; a small **MCP Policy** panel (allow/deny lists).

**Task checklist:**
- [ ] Read/write the per-project **disabled MCP** list (confirm exact settings key + file from CCO — likely `settings.local.json`).
- [ ] Read/write **policy** allow/deny lists in user settings (`~/.claude` settings).
- [ ] Frontend toggle wires to `mcp_set_disabled`; policy panel wires to `mcp_set_policy`; reflect state in item rendering.

**CCO parity refs:** `src/harness/adapters/claude.mjs` (`getDisabledMcpServers` / `setDisabledMcpServers` + MCP-policy fns); `src/server.mjs` endpoints `/api/mcp-disabled`, `/api/mcp-policy`.

**Tests:** disabling a server writes the correct key; policy add/remove; disabled state surfaces on the item.

**Gotchas:** confirm exact JSON keys/paths from CCO (don't guess); disabled state is **per-project scope**, policy is **user scope**; preserve unrelated settings keys on write (round-trip the JSON).
