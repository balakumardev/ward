# Ward Plan 11 — Ward-as-MCP-Server (High-Level)

> High-level plan. Use **superpowers:subagent-driven-development**, TDD, commit per task. CCO reference (read-only): `/Users/balakumar/personal/claude-code-organizer`.

**Goal:** Expose Ward itself as an **MCP server (stdio)** so AI clients (Claude Code, Codex) can call it — parity with CCO's `mcp-server.mjs`.

**Builds on:** the core impls already built (`scan_impl`, mutation ops from Plan 03, `security_scan` from Plan 05).

**Files:**
- Create `src-tauri/src/mcp/server.rs`: a stdio MCP server exposing tools **`scan_inventory`, `move_item`, `delete_item`, `list_destinations`, `audit_security`** — each delegating to the existing core functions (no logic duplication). Use the `rmcp` crate or a hand-rolled JSON-RPC stdio loop (match the introspector's approach from Plan 05).
- Modify `src-tauri/src/main.rs`: add a **`--mcp` mode** that runs the stdio server headless instead of launching the GUI.

**Task checklist:**
- [ ] Define the 5 MCP tools + input schemas.
- [ ] Wire each tool to the existing core function.
- [ ] `--mcp` headless entrypoint (no window, stdio only).
- [ ] Smoke: `ward --mcp` responds to `initialize` + `tools/list` + a `scan_inventory` call.

**CCO parity refs:** `src/mcp-server.mjs` (the 5 tools + their zod schemas → Rust schema equivalents).

**Tests:** JSON-RPC `initialize` + `tools/list` returns 5 tools; `scan_inventory` returns a `ScanResult`; `audit_security` returns findings; runs fully headless.

**Gotchas:** **reuse** the GUI's core functions — do not fork the logic; runs headless (no Tauri window); mirror CCO's tool names/params so existing users' muscle memory carries over.
