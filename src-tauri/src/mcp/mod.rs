//! mcp/mod.rs — Model Context Protocol support modules for Ward.
//!
//! Plan 05 ships the introspection client (`introspect.rs`) used by the
//! security scanner to talk to MCP servers over stdio and read their
//! tool definitions.
//!
//! Plan 11 ships the headless MCP server (`server.rs`) that exposes
//! Ward itself as an MCP server, mirroring CCO's `mcp-server.mjs`.

pub mod introspect;
pub mod server;