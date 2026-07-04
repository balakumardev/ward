# Ward Plan 03 — Mutations: move / delete / undo / edit / bulk (High-Level)

> High-level plan. Use **superpowers:subagent-driven-development**, TDD, commit per task. CCO reference (read-only): `/Users/balakumar/personal/claude-code-organizer`.

**Goal:** Make the Organizer editable — move items between scopes, delete with **undo**, edit + save frontmatter/content, **bulk** ops, and valid-destination resolution.

**Builds on:** `HarnessItem` (`movable`/`deletable`/`locked`), `commands.rs`, `ensure_under_home`, `Organizer`, `api.ts`.

**Files:**
- Create `src-tauri/src/harness/adapters/claude_ops.rs`: `move_item`, `delete_item`, `restore_item`, `restore_mcp_entry`, `get_valid_destinations`.
- Modify `src-tauri/src/harness/mod.rs`: add optional operations to the `Harness` trait (or an `operations()` accessor) — signatures `move_item(&Ctx,&HarnessItem,dest:&str)->Result<RestoreInfo>`, `delete_item(...)->Result<RestoreInfo>`, `restore(&Ctx,&RestoreInfo)`, `get_valid_destinations(&Ctx,&HarnessItem)->Vec<Destination>`.
- Add `RestoreInfo` + `Destination` to `model.rs`.
- Modify `src-tauri/src/commands.rs`: `move_item`, `delete_item`, `restore`, `restore_mcp`, `save_file`, `list_destinations`.
- Frontend `modes/Organizer.tsx`: detail actions (Move ▾ from destinations, Delete, Undo), frontmatter/content editor (textarea + Save), bulk selection bar (shift-click multi-select → batch move/delete with one combined undo).

**Task checklist:**
- [ ] `get_valid_destinations` per category (global↔project) — port CCO `claude-operations.mjs`.
- [ ] `move_item`: fs rename/copy into destination scope dir; MCP entries move by **editing JSON** (`.mcp.json` / `~/.claude.json` / settings), not moving files.
- [ ] `delete_item`: capture `RestoreInfo` (path + bytes / JSON entry), then unlink/rm.
- [ ] `restore` / `restore_mcp`: live undo from `RestoreInfo` (NOT a history engine — CCO's `history.mjs` is dormant/unused).
- [ ] `save_file`: write edited markdown via `ensure_under_home`.
- [ ] Bulk: sequential per-item ops accumulating a combined undo payload.

**CCO parity refs:** `src/harness/adapters/claude-operations.mjs`; endpoints in `src/server.mjs` (`/api/move`, `/api/delete`, `/api/restore`, `/api/restore-mcp`, `/api/save-frontmatter`, `/api/destinations`). **Port golden:** `tests/unit/test-move-destinations.mjs`.

**Tests:** move skill global→project; delete + restore round-trip; MCP entry move edits JSON; locked item rejects move/delete; destination validity.

**Gotchas:** MCP move = JSON edit, not file move; `locked` items reject mutation; every write through `ensure_under_home`; keep undo payload before deleting; bulk undo must reverse all sub-ops.
