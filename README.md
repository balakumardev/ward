<h1 align="center">Ward</h1>

<p align="center">
  <strong>A native macOS command center for everything your AI coding tools silently load.</strong><br>
  See it, clean it up, scan it for poisoned MCP servers — and watch your Claude &amp; Codex usage from the menu bar.
</p>

<p align="center">
  <a href="https://github.com/balakumardev/ward/releases/latest"><b>⬇ Download for macOS</b></a> ·
  <a href="#building-from-source">Build from source</a> ·
  <a href="#privacy">Privacy</a>
</p>

<p align="center">
  <a href="https://github.com/balakumardev/ward/actions/workflows/release.yml"><img alt="Release" src="https://github.com/balakumardev/ward/actions/workflows/release.yml/badge.svg"></a>
  <a href="https://github.com/balakumardev/ward/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/balakumardev/ward?sort=semver"></a>
  <img alt="Platform" src="https://img.shields.io/badge/platform-macOS%2011%2B-black">
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <img alt="Tauri" src="https://img.shields.io/badge/built%20with-Tauri%202-24C8DB">
</p>

---

Ward organizes the configuration of **Claude Code** and **Codex CLI** from one window: skills,
memories, MCP servers, agents, hooks, commands, plans, rules, and settings — across global and
project scopes. On top of the organizer it adds an **MCP security scanner**, a **context-window
budget** view, **session** tools, a git-backed **backup center**, and a background **menu-bar
agent** that shows live usage and runs scheduled scans.

Config organizer first — the security scanner is one of five modes, not the whole app. Everything
runs **locally**; Ward is built with **Tauri 2** (a Rust core and a native macOS shell), so there's
no Electron, no bundled browser, and no server.

## Install

Grab the latest `.dmg` from the [**Releases**](https://github.com/balakumardev/ward/releases/latest)
page (universal — Apple Silicon + Intel), open it, and drag **Ward** into Applications.

These builds are **not notarized** (it's a personal project), so on first launch macOS will refuse
to open it. Either **right-click Ward.app → Open → Open**, or clear the quarantine flag once:

```sh
xattr -dr com.apple.quarantine /Applications/Ward.app
```

Ward then lives in your menu bar. Click the tray icon for the usage glance; closing the window keeps
it running in the background. Launch-at-login is on by default (toggle it from the glance popover).

## What it does

Ward has five sidebar modes, plus the menu-bar agent, and a harness switcher (**Claude ⇄ Codex**) at
the top.

- **Organizer** — a three-column view (categories → items → detail) of every config artifact both
  harnesses load: skills, memories, MCP servers, agents, hooks, commands, plans, rules, settings.
  Move items between global and project scopes, delete them, and undo — Ward computes the *effective*
  resolution so you can see what actually wins when scopes shadow each other.
- **Security** — an MCP security scanner (50+ rules) that inspects your servers and their tool
  schemas for prompt-injection, exfiltration, and other poisoned-tool patterns, with a baseline diff
  so you're alerted when a server's tools change under you.
- **Context Budget** — a per-scope breakdown of what's eating your context window (system overhead,
  MCP schemas, always-loaded `CLAUDE.md` files and items), measured with a real tokenizer where
  possible.
- **Sessions** — browse and preview session transcripts, estimate their cost, and distill/trim bulky
  ones.
- **Backups** — a git-backed mirror of your config in `~/.ward-backups/` (local commits only; pushes
  are always manual).

### Menu-bar usage glance

Click the tray icon and Ward shows, for **both** Claude and Codex, your **5-hour** and **weekly**
usage as a percent of your subscription limit, with live reset countdowns — the same view as Claude
Code's `/usage`, right in the menu bar.

For Claude this reads the real limit percentages from Anthropic's rate-limit endpoint. It is **opt-in
and gated**: the Claude row shows an *"Enable live usage"* button until you turn it on, macOS then
prompts once to grant Ward access to your existing Claude login in the Keychain, and from there the
call fires only when you open the popover or hit Refresh — never on a background timer. Codex reads
its usage from local session files.

## Privacy

Ward is local-first by design.

- The organizer, security scanner, context budget, sessions, and backups **never touch the network**
  and read only your local config under `~/.claude`, `~/.codex`, and `~/.ward`.
- The **live usage** feature is the one network call, and it is strictly opt-in. When enabled, Ward
  reads your Claude OAuth token from the macOS Keychain (with the system's own permission prompt) and
  sends it **only** as the authorization header to `api.anthropic.com` to read your rate-limit
  status. The token is never logged, stored by Ward, or sent anywhere else. Turn it off and Ward
  makes no network calls at all.
- Backups commit locally to `~/.ward-backups/`; nothing is ever pushed unless you explicitly push it.

## Building from source

### Prerequisites

- `rustc` (stable) and `node ≥ 20`
- Xcode command-line tools: `xcode-select -p` must succeed

### Develop

```sh
npm install
npm run tauri dev          # hot-reload dev session (native window)
npm run dev:mock           # full UI in a plain browser on fixture data (http://localhost:1430)
```

### Test

```sh
npm test                                            # frontend (vitest)
npx tsc --noEmit                                    # typecheck
cargo test --manifest-path src-tauri/Cargo.toml     # Rust core
```

### Build a distributable

```sh
npm run tauri build -- --target universal-apple-darwin
```

The universal `.app` and `.dmg` land in `src-tauri/target/universal-apple-darwin/release/bundle/`.
For signed + notarized builds, set `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, and
`APPLE_TEAM_ID` and see [`BUILD.md`](BUILD.md) for the full playbook. CI (`.github/workflows/release.yml`)
cuts an unsigned universal release on every push to `main`.

## Tech

Rust core (all logic as Tauri `invoke` commands) · SolidJS + TypeScript + Vite frontend · Tauri 2
native shell. The frontend only ever talks to the core through `invoke` — it never touches the
filesystem directly.

## License & attribution

[MIT](LICENSE) © 2026 Bala Kumar.

Ward is a clean-room reimplementation inspired by the MIT-licensed
[Cross-Code Organizer](https://github.com/mcpware/cross-code-organizer). Its usage engine ports the
5-hour session-block algorithm from [ccusage](https://github.com/ryoppippi/ccusage), and the live
rate-limit-header technique is used by monitors like
[claude-monitor](https://github.com/rjwalters/claude-monitor) and
[CCSeva](https://github.com/Iamshankhadeep/ccseva). No source was copied verbatim — see
[`NOTICE`](NOTICE) for full attribution.
