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

## Building

For development:

```sh
npm install
npm run tauri dev       # hot-reload dev session
```

For a distributable `.dmg` (Apple Silicon + Intel via `lipo`):

```sh
./src-tauri/dist/sign.sh universal
```

The wrapper reads the `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
`APPLE_PASSWORD`, and `APPLE_TEAM_ID` env vars for Apple notarization.
Without them, the build succeeds unsigned. See
[`BUILD.md`](BUILD.md) for the full production-build playbook,
troubleshooting, and the post-build Gatekeeper / `stapler` checks.
