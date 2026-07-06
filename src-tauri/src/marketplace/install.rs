//! Plan 21 Task 3 — build a version-pinned, secret-safe MCP config and
//! fan the install out to the shared `upsert_mcp_entry` engine.
//!
//! `build_mcp_config` is pure and fully unit-tested (version-pin enforcement,
//! secret omission, stdio/remote shapes). `install` dispatches per target via
//! `ops_for(&harness)?.upsert_mcp_entry(...)`, collecting one `InstallResult`
//! per target so a single failure never aborts the batch.
