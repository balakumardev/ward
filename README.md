# Ward

A **native macOS command center** for everything your AI coding tools silently load —
see it, clean it up, and scan it for poisoned MCP servers.

Ward organizes the configuration of **Claude Code** and **Codex CLI** from one window:
skills, memories, MCP servers, agents, hooks, commands, plans, rules, and settings —
across global and project scopes. It adds an integrated **MCP security scanner**,
a **context-window budget** view, **session** tools, and a git-backed **backup center**,
plus a background **menu-bar agent** that runs scheduled scans and fires native
notifications.

Built with **Tauri 2.0** (Rust core + native shell). Config organizer first — the
security scanner is one of five sidebar modes, not the whole app.

> Status: **design complete, pre-implementation.**
> See the design spec: [`docs/superpowers/specs/2026-07-04-ward-native-tauri-design.md`](docs/superpowers/specs/2026-07-04-ward-native-tauri-design.md)

Ward is a clean-room reimplementation inspired by the MIT-licensed
[Cross-Code Organizer](https://github.com/mcpware/cross-code-organizer) — see [`NOTICE`](NOTICE).
