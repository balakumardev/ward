//! Plan 21 Task 2 — official MCP Registry client.
//!
//! Split into a pure `parse_servers` (fully unit-tested against a pinned
//! synthetic fixture) and a thin `fetch_servers` `ureq` wrapper (network,
//! not unit-tested), mirroring `usage/live.rs`.
