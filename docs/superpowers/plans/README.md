# Ward — Implementation Plans

The [design spec](../specs/2026-07-04-ward-native-tauri-design.md) is a multi-subsystem
product, so it is implemented as a **sequence of plans**. Each plan ships working,
independently testable software and builds on the previous one. Execute in order.

| Plan | Subsystem | Ships |
|---|---|---|
| **01** | **Foundation** | Scaffold (Tauri 2 + Solid-TS + Rust), core data model, harness framework, Claude adapter read-only scan (skills + memory), Organizer 3-column browse + detail pane. **Ward launches and browses your Claude config natively.** |
| 02 | Full Claude categories + Effective | Remaining 10 categories (mcp, command, agent, plan, rule, config, hook, plugin, session, setting) + Show-Effective resolution. |
| 03 | Mutations | Move / delete / undo, frontmatter editor save, bulk operations, valid-destination resolution. |
| 04 | MCP controls | Enable/disable per project + policy allow/deny lists. |
| 05 | Security scanner | MCP introspection (JSON-RPC/stdio) + 4-layer pipeline + hash baseline + in-place master-detail UI + optional `claude -p` judge. |
| 06 | Context Budget | Token composition model + tokenizer + `@import` expansion + budget mode UI. |
| 07 | Sessions | JSONL viewer, per-model cost, distill (~90% cut), image trim. |
| 08 | Backups | git export/commit/push + launchd scheduler. |
| 09 | Codex adapter | `config.toml` parsing, 11 categories, capability-gated parity. |
| 10 | Native shell | Menu-bar glance+alert tray, background launchd scans, native notifications, fs-watch live refresh. |
| 11 | Ward-as-MCP-server | Expose scan/move/delete/audit as MCP tools (stdio). |
| 12 | Packaging | `.dmg`, Developer ID signing + notarization, WebDriver E2E, polish. |

**Status:** Plan 01 written. Plans 02–12 will be authored just-in-time as each prior plan lands (learnings from earlier plans sharpen later ones — YAGNI on writing them all upfront).
