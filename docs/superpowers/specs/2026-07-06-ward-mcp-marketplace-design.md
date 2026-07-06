# Ward — MCP + Skills Management & Marketplace — Design Spec

> Status: **locked** (brainstorm complete). Source of truth for Plans 18–22.
> Date: 2026-07-06. Author: Bala Kumar (via Claude).
> Supersedes nothing; extends the Plan 01–17 architecture.

## 1. Goal

Turn Ward from a *read + relocate + delete* config organizer into one that can also
**author and install** the two config units that matter most — **MCP servers** and
**Skills** — across **both** supported harnesses (Claude Code first, Codex second).

Two user-facing deliverables:

- **(A) Editable Organizer.** Today an MCP item's detail pane is **read-only JSON**
  and Codex has **no write path at all**. Make MCP servers **add / edit / remove**
  (structured form, not raw JSON) and make Skills **add** (edit already works via the
  markdown editor). Works for Claude **and** Codex.
- **(B) Marketplace mode.** A new 6th sidebar mode: **search** canonical registries →
  **install** MCP servers + Skills into the chosen harness(es) × scope(s). The wedge is
  **install-once-to-many**: one action fans out to every selected target.

Both front-ends sit on **one shared write engine**. That is the central architectural
bet: the Organizer's "Save" and the Marketplace's "Install" are the *same* surgical
upsert underneath.

## 2. Locked decisions (do not relitigate)

1. **Data = canonical first.**
   - MCP servers: official **MCP Registry API** — `GET https://registry.modelcontextprotocol.io/v0/servers`.
     Each entry is a `server.json`: `packages[]{registryType npm|pypi|oci, identifier, version, transport{type}, environmentVariables[]{name,isRequired,isSecret}, runtimeHint?}` **OR** `remotes[]{type, url, headers[]}`.
   - Skills: Claude `.claude-plugin/marketplace.json` repos + GitHub `SKILL.md`.
   - Third-party directories (skills.sh, etc.): **optional, later.**
2. **Harnesses = Claude + Codex, Claude first.** Codex has no write path in Ward yet — build it.
3. **Units = two types:** MCP servers + Skills. (Plugin bundles are unpacked into their parts; a bundle that ships an MCP server + a skill installs as both.)
4. **Scope = Claude Code + Codex only.** But build install/audit as a **target list** so Claude Desktop slots in later without a rewrite.
5. **Security:** show exact `command`/`args`/`env` (or `url`/`headers`) **before** install; **pin versions (never `@latest`)**; show registry verification status; **bind approval to the definition, not the name.**

## 3. Prior art (what CCO gives us, what is greenfield)

CCO (`/Users/balakumar/personal/claude-code-organizer`, read-only, MIT) is a
*read + relocate + delete* organizer. It has **no** create/edit-body path, **no**
marketplace, **no** registry client. So:

- **Editable MCP form, editable/creatable Skills, and the whole Marketplace are greenfield.** No CCO code to port for them.
- What CCO *does* pin down and Ward must match:
  - **Write discipline (JSON):** read → mutate one key **in memory** → **rewrite the entire file** with `JSON.stringify(obj, null, 2) + "\n"`; `mkdir -p` the parent first. Ward already does exactly this in `claude_ops.rs::write_json`.
  - **`.claude.json` dual nesting:** user servers at top-level `mcpServers`; project servers at `projects["<absRepoPath>"].mcpServers`. Ward's `McpParentKey` already models both.
  - **stdio vs remote by key presence:** `command` ⇒ stdio `{command, args, env}`; `url` ⇒ remote `{url, type?, headers?}`. Never decide by a `type` field.
  - **Opaque preservation:** on edit/move, round-trip the *whole* server object — never reconstruct it from parsed fields, or you drop unknown keys (`type`, `headers`, timeouts…). (For the structured editor this becomes: **start from the existing object and patch only the fields the form owns**, leaving every other key untouched.)

Ward already ships the security moat CCO has (58-rule scanner, 8 deobfuscation
techniques, SHA-256 tool-hash baseline, optional `claude -p` judge). The Marketplace
reuses that as the **post-install audit**; the **pre-install** trust gate (show
command/args/env, policy verdict, version pin) is new.

## 4. Architecture — one write engine, two front-ends

```
        Organizer detail pane            Marketplace mode
        (Add / Edit / Delete MCP,        (search registries → Install
         Add Skill)                        to ✓Claude ✓Codex × scope)
                 \                              /
                  \                            /   fan-out: one Install =
                   v                          v    N upsert calls
            ┌───────────────────────────────────────────┐
            │  Tauri commands (harness-dispatched)        │
            │   mcp_upsert_entry · mcp_delete_entry       │
            │   skill_upsert · marketplace_* ...          │
            └───────────────────────────────────────────┘
                 |                              |
        ops_for("claude") = ClaudeOps   ops_for("codex") = CodexOps  (NEW)
                 |                              |
     JSON surgical upsert of one         TOML surgical upsert of one
     mcpServers[<name>] key in           [mcp_servers.<name>] table in
     ~/.claude(/.mcp).json               ~/.codex/config.toml
     (serde_json::Value)                 (toml_edit::DocumentMut)
```

**Invariant (the critical gotcha):** an MCP item's `path` is the **whole shared config
file** (`~/.claude.json`, `~/.claude/.mcp.json`, `~/.codex/config.toml`), **not** a
per-server file. **Never full-file-save the pane content.** Always a **surgical upsert
of one key**, preserving every other key/table/comment. This is why the current pane is
read-only; this spec replaces "read-only" with "structured form → surgical upsert."

## 5. The MCP write engine

### 5.1 Claude (JSON) — `claude_ops::upsert_mcp_entry`

Implemented in `src-tauri/src/harness/adapters/claude_ops.rs` so it reuses the existing
**private** helpers verbatim: `McpParentKey`, `ensure_mcp_parent`, `insert_mcp_entry`,
`detect_mcp_parent`, `read_json_or_empty`, `write_json`.

```rust
/// Surgically upsert (insert-or-overwrite) a single MCP server entry in `target`.
/// Mirrors `claude_mcp::set_policy`: whole prior file bytes are captured for a true undo.
pub fn upsert_mcp_entry(
    target: &Path,                 // file to write
    parent: &McpParentKey,         // mcpServers | projects:<key>
    name: &str,
    config: &serde_json::Value,    // the server object (stdio or remote shape)
) -> Result<RestoreInfo, WardError> {
    let backup_bytes = std::fs::read(target).unwrap_or_default();
    let mut root = read_json_or_empty(target)?;   // {} when absent
    ensure_mcp_parent(&mut root, parent);
    insert_mcp_entry(&mut root, parent, name, config.clone());   // overwrites if present
    if let Some(dir) = target.parent() { std::fs::create_dir_all(dir)?; }
    write_json(target, &root)?;                   // pretty + trailing "\n"
    Ok(RestoreInfo {
        kind: "mcp-upsert".into(),
        original_path: target.display().to_string(),
        current_path: None,
        backup_bytes: if backup_bytes.is_empty() { None } else { Some(backup_bytes) },
        mcp_entry: None, mcp_key: Some(name.into()),
        mcp_parent_key: Some(parent.object_key().to_string()),
        mcp_scope: parent.scope_key().map(str::to_string),
    })
}
```

**Target + parent resolution** (done in the command, from the frontend's args):

- **Edit existing:** `target = item.path`; `parent = detect_mcp_parent(item.path, name, scopes)` (finds `mcpServers` vs `projects[<key>].mcpServers`). The form patches the existing object so unknown keys survive.
- **Add new:** `target = resolve_mcp_json(scope_id, scopes)` — global → `~/.claude/.mcp.json`, project → `<repo>/.mcp.json` (both are already scanned by `ClaudeAdapter::scan_mcp`, so the new server appears immediately after re-scan). `parent = McpParentKey::mcp_servers()` (flat top-level).

### 5.2 Codex (TOML) — `codex_ops::upsert_mcp_entry`

New `src-tauri/src/harness/adapters/codex_ops.rs`. **`toml_edit` is required** — a
`toml::Value` serialize round-trip destroys comments, reorders keys, and mangles the
quoted-key / nested-sub-table / dotted-key shapes the real `~/.codex/config.toml` uses.
Add `toml_edit = "0.22"` to `Cargo.toml` (commit `Cargo.lock`).

```rust
use toml_edit::{DocumentMut, Item, Table, value, Array};

/// Surgically upsert `[mcp_servers.<name>]` in `target` (config.toml), preserving
/// all other tables, comments, and formatting. `config` is the JSON server object
/// (same shape as Claude): {command, args, env} or {url, headers, bearer_token_env_var}.
pub fn upsert_mcp_entry(target: &Path, name: &str, config: &serde_json::Value)
    -> Result<RestoreInfo, WardError>
{
    let backup_bytes = std::fs::read(target).unwrap_or_default();
    let text = std::fs::read_to_string(target).unwrap_or_default();
    let mut doc: DocumentMut = text.parse().map_err(|e| WardError::NotFound(format!("parse toml: {e}")))?;
    // Ensure [mcp_servers] exists as an implicit table; set the named sub-table.
    let servers = doc.entry("mcp_servers").or_insert(Item::Table(Table::new()));
    let tbl = json_to_toml_table(config);   // command/args/env or url/headers → toml_edit Table
    servers.as_table_mut().unwrap().insert(name, Item::Table(tbl));
    if let Some(dir) = target.parent() { std::fs::create_dir_all(dir)?; }
    std::fs::write(target, doc.to_string())?;
    Ok(RestoreInfo { kind: "mcp-upsert".into(), original_path: target.display().to_string(),
        current_path: None, backup_bytes: if backup_bytes.is_empty() { None } else { Some(backup_bytes) },
        mcp_entry: None, mcp_key: Some(name.into()), mcp_parent_key: Some("mcp_servers".into()), mcp_scope: None })
}
```

- **Target resolution:** global → `~/.codex/config.toml`; project → `<repo>/.codex/config.toml`.
- **Quoted keys:** `toml_edit` quotes keys with hyphens automatically (`[mcp_servers."auggie-mcp"]`). Do not pre-quote.
- **Enable/disable:** Codex uses an `enabled` bool inside `[mcp_servers.<name>]` (observed in the wild). The editor form exposes an **Enabled** checkbox for Codex MCP (persisted through the same upsert), instead of the Claude-only per-project row toggle.
- **`json_to_toml_table`:** the inverse of the existing `codex.rs::toml_to_json`. Maps string/number/bool/array/table recursively. Unit-tested against fixtures.

### 5.3 Delete — reuse the existing surface

MCP **delete** already flows through `delete_item` (`category == "mcp"`):
- Claude: `claude_ops::delete_mcp_entry` (already implemented; `detect_mcp_parent` handles both nesting levels).
- Codex: implement `CodexOps::delete_item` → remove `[mcp_servers.<name>]` via `toml_edit` (whole-file `backup_bytes` for undo). Requires flipping Codex MCP items to `deletable: true` in `codex.rs`.

### 5.4 Undo — `RestoreInfo { kind: "mcp-upsert" }`

Whole-file backup (mirrors `set_policy`/`set_disabled_servers`). Add a `"mcp-upsert"`
arm to `ClaudeOps::restore` and `CodexOps::restore` that routes to a whole-file restore
(reuse `claude_mcp::restore_mcp_file`: `backup_bytes: Some → write bytes`, `None →
remove the file`). This yields byte-identical restore for edits and clean removal for
adds. `RestoreInfo.kind` in `src/api.ts` gains `'mcp-upsert'`.

## 6. Editable MCP Organizer (Task A, front-end)

`src/modes/Organizer.tsx` detail pane, when `item.category === 'mcp'`, renders a
**structured form** instead of read-only JSON:

- **Transport toggle:** `stdio` ↔ `http` (segmented control), inferred from the config
  (`command` present ⇒ stdio, `url` present ⇒ http).
- **stdio fields:** `command` (text), `args` (add/remove rows of text), `env`
  (add/remove rows of `KEY` / `VALUE`). **Codex-only:** `Enabled` checkbox.
- **http fields:** `url` (text), `headers` (add/remove rows of `KEY` / `VALUE`).
- **Preserve unknown keys:** the form seeds from `item.mcpConfig`; on Save it patches
  only the owned fields back into a **clone** of the original object, so any extra keys
  (`type`, timeouts, Codex `tools.*.approval_mode`, …) survive.
- **Footer:** `Save` (→ `mcp_upsert_entry`), `Revert`, and the existing `Undo`. Save is
  disabled until dirty. The read-only notice is removed.
- **Add MCP Server:** a `+ Add` control in the MCP category header opens the same form
  blank (name field enabled) with a scope picker; Save → `mcp_upsert_entry` with no
  `target_path` (Rust resolves the scope's file).
- **Delete:** the existing `delete-btn` path (already wired for `category==='mcp'`).

The add/remove-row **logic** is the `McpPolicy.tsx` template, but re-authored
**class-based** (`src/styles/organizer.css` / `app.css` + tokens) — not inline styles.
**All existing `data-testid`s are preserved**; new ones added: `mcp-form`,
`mcp-transport-stdio`, `mcp-transport-http`, `mcp-command`, `mcp-arg-add`,
`mcp-arg-row`, `mcp-arg-remove`, `mcp-env-add`, `mcp-env-row`, `mcp-env-key`,
`mcp-env-value`, `mcp-env-remove`, `mcp-url`, `mcp-header-*`, `mcp-enabled`,
`mcp-add-button`, `mcp-name`, `mcp-scope-pick`, `mcp-save`, `mcp-revert`.

The `organizerApi` bridge (App.tsx) gains `upsertMcpEntry(...)` following the
established shape: `const r = await api.mcpUpsertEntry(...); await refetch(); return r;`.

## 7. Editable / creatable Skills (Task A, skills)

Editing an existing skill **already works** (the markdown editor handles `category ==
'skill'` via `save_file`). Missing piece = **create a new skill**.

- New command `skill_upsert(harness, scope_id, name, content) -> RestoreInfo`. Validates
  `name` (non-empty, kebab-safe, no path separators / `..`), resolves the scope's skills
  dir (Claude: `~/.claude/skills` or `<repo>/.claude/skills`; Codex: `~/.codex/skills`),
  writes `<dir>/<name>/SKILL.md`, `mkdir -p` first, whole-file `backup_bytes` for undo
  (`None` for a fresh create). Refuses to clobber an existing skill (returns a clear
  error) unless the caller passes an explicit overwrite intent (the edit path continues
  to use `save_file`, so `skill_upsert` is create-only in practice).
- Organizer: `+ Add Skill` control in the skill category header → dialog (name + scope) →
  scaffolds a starter `SKILL.md` (`---\nname: <name>\ndescription: <TODO>\n---\n\n# <name>\n`),
  then selects the new item so the user fills it in and Saves via the existing editor.
- `data-testid`s: `skill-add-button`, `skill-add-name`, `skill-add-scope`, `skill-add-create`.

## 8. Codex write path (Task A, backend)

New `CodexOps` (`codex_ops.rs`) implementing `HarnessOps`:

- `save_file` — reuse the `ensure_under_home` + write pattern (Codex plain-text edits to
  `AGENTS.md` / memory / rules already work through the shared `save_file` command; this
  makes it explicit and harness-consistent).
- `upsert_mcp_entry` (§5.2) + `delete_item` for `category == "mcp"` (TOML remove).
- `get_valid_destinations` / `move_item` — Codex config is single-file per scope; Codex
  file categories (memory/skill/rule) are not scope-movable the way Claude's are (the CCO
  Codex adapter never supported move). Return **no destinations** for now (matches current
  behavior) — move stays a Claude capability. `restore` handles `"file"` and `"mcp-upsert"`.
- Register: `pub mod codex_ops;` in `adapters/mod.rs`; add `"codex" => Ok(&CodexOps)` to
  `ops_for` in `commands.rs`.
- Flip `codex.rs` MCP items to `deletable: true` and attach `mcp_config` (already present)
  so the Organizer offers Edit/Delete. Config items (`config.toml`, `AGENTS*.md`) stay
  `locked` (structured whole-file edits are out of scope).

**Trait change:** add one method to `HarnessOps`:
```rust
fn upsert_mcp_entry(&self, ctx: &Ctx, scope_id: &str, name: &str,
    config: &serde_json::Value, target_path: Option<&str>, scopes: &[Scope])
    -> Result<RestoreInfo, WardError>;
```
Both `ClaudeOps` and `CodexOps` implement it (the only two impls). The `mcp_upsert_entry`
command dispatches via `ops_for(harness)`.

## 9. Marketplace mode (Task B)

New Rust module `src-tauri/src/marketplace/{mod,registry,skills,install}.rs`. New 6th
sidebar mode. **Network calls are user-triggered** (search on demand, install on click) —
never a background poll. Reuses the existing `ureq` dependency.

### 9.1 Data models (`marketplace/mod.rs`)

```rust
pub struct MarketEntry {           // unified card model, camelCase over the wire
    pub kind: String,              // "mcp" | "skill"
    pub name: String,              // registry id, e.g. "io.github.owner/server"
    pub display_name: String,
    pub description: String,
    pub source: String,            // "registry" | "github" | "marketplace"
    pub version: Option<String>,   // concrete version if known (never "latest")
    pub verified: bool,            // registry-listed / signed status
    pub packages: Vec<Package>,    // MCP only
    pub remotes: Vec<Remote>,      // MCP only
    pub repo_url: Option<String>,  // skills: source repo
    pub skill_path: Option<String>,// skills: SKILL.md path within repo
}
pub struct Package { pub registry_type: String /* npm|pypi|oci */, pub identifier: String,
    pub version: String, pub transport: String /* stdio|http|sse */,
    pub env: Vec<EnvVar>, pub runtime_hint: Option<String> }
pub struct EnvVar { pub name: String, pub is_required: bool, pub is_secret: bool }
pub struct Remote { pub transport: String, pub url: String, pub headers: Vec<EnvVar> }
```

### 9.2 Registry client (`marketplace/registry.rs`)

- `fetch_servers(cursor: Option<&str>) -> Result<RegistryPage, WardError>` — thin `ureq`
  GET of `https://registry.modelcontextprotocol.io/v0/servers?limit=100[&cursor=...]`.
  **Network wrapper only** (not unit-tested; integration/manual), exactly like
  `usage/live.rs::fetch_ratelimit_headers`.
- `parse_servers(body: &str) -> Result<RegistryPage, WardError>` — **pure**, fully
  unit-tested against pinned fixture JSON. Maps the official `server.json` shape into
  `Vec<MarketEntry>` + `next_cursor`. Tolerates missing `packages`/`remotes`.
- New error variant `WardError::Registry(String)` (mirrors the `Live` variant addition).

### 9.3 Skills catalog (`marketplace/skills.rs`)

- A **curated, in-binary list** of trusted Claude marketplace repos (their
  `.claude-plugin/marketplace.json` raw URLs). `fetch_marketplace(url)` (network wrapper)
  + `parse_marketplace(body)` (pure, unit-tested) → `Vec<MarketEntry>` of `kind: "skill"`
  with `repo_url` + `skill_path`.
- `fetch_skill_md(repo_url, skill_path)` — network wrapper returning the raw `SKILL.md`
  bytes for install.
- First cut targets `marketplace.json`-listed skills; freeform GitHub `SKILL.md` URLs are
  accepted via a "paste a SKILL.md URL" affordance (parsed the same way).

### 9.4 Install = fan-out to the shared upsert

```rust
pub struct InstallTarget { pub harness: String, pub scope_id: String }   // extensible: + "claude-desktop" later
pub struct BuiltConfig { pub name: String, pub config: serde_json::Value, // the exact server object
    pub command_preview: Vec<String>, pub env: Vec<EnvVar> }
```

- `build_mcp_config(entry, package_index, env_values) -> BuiltConfig` — **pure, tested.**
  - **Version pin:** npm → `args = ["-y", "<identifier>@<version>"]`; pypi → `uvx`/`pipx`
    run with `<identifier>==<version>`; **reject any resolved arg containing `@latest`
    or an empty version** (`WardError::Registry("refusing to install an unpinned version")`).
  - **stdio** → `{command, args, env}` (env keys from `EnvVar`; see secret handling below).
    **http/sse** → `{url, type, headers}`.
- `marketplace_install(entry, package_index, targets, env_values)` — for each target,
  dispatch `ops_for(target.harness).upsert_mcp_entry(scope, name, config, None, scopes)`;
  for skills, `fetch_skill_md` → `skill_upsert(target.harness, target.scope_id, name, body)`.
  Returns a per-target result vector (`{target, ok, error?, restore?}`) so partial
  failures are visible and individually undoable.

### 9.5 Security posture (locked item 5, enforced)

- **Exact preview before write:** the install sheet renders `command` + `args` + `env`
  (names) or `url` + `headers` (names) verbatim from `BuiltConfig` — the user sees exactly
  what will land on disk before clicking Install.
- **Version pinning:** enforced in `build_mcp_config` (no `@latest`, no empty version).
- **Verification status:** `MarketEntry.verified` (registry-listed) shown as a badge.
- **Approval bound to definition:** after install, the item flows through the existing
  Security scanner + SHA-256 tool-hash baseline (introspection hashes tool defs, not names).
- **Policy gate:** before writing, run `check_policy(name, config, current_policy)`; if the
  verdict is `Denied`, block with an explanation and a link to MCP Policy. Reuses the
  existing `mcp_get_policy` / `check_policy`.
- **Secret handling (safety-critical):** Ward **never collects secret values.** For
  `EnvVar { is_secret: true }`, the install form shows the **variable name** with a note
  ("set this in your shell / environment") and writes the key with an **empty string**
  (or omits it) — it does **not** render a field to type the secret, and never writes a
  token into `.mcp.json` / `config.toml`. Non-secret env vars (`is_secret: false`) may be
  filled in normally. This aligns with Ward's standing rule (no secrets typed into fields).

### 9.6 Frontend (`src/modes/Marketplace.tsx` + `src/styles/marketplace.css`)

- Full-width mode (own shell class, like `BudgetWithPicker`). Tabs: **MCP Servers** /
  **Skills**. Search box → cards (name, description, source/verified badge, version).
- Card → detail sheet: package/transport picker, env-var list (secret rows read-only with
  the note), **exact command/args/env preview**, policy verdict, and the
  **install target matrix**: `☐ Claude ☐ Codex` × `☐ Global ☐ <project>` (checkbox grid;
  the target-list is data-driven so Claude Desktop can be appended later).
- Install → `marketplace_install` → per-target toast with Undo.
- Wiring: append `{ id: 'marketplace', label: 'Marketplace', icon: '◱' }` to `MODES` in
  `src/components/Sidebar.tsx` (**update `Sidebar.test.tsx`** which hard-asserts the mode
  list); add a `<Show when={mode() === 'marketplace'}>` rung in `App.tsx`.
- `data-testid`s: `market-tab-mcp`, `market-tab-skills`, `market-search`, `market-card`,
  `market-detail`, `market-pkg-pick`, `market-env-row`, `market-preview`,
  `market-target-claude`, `market-target-codex`, `market-target-scope`, `market-install`,
  `market-policy-verdict`.

## 10. Command + API surface (additions)

| Tauri command | Args | Returns | api.ts |
|---|---|---|---|
| `mcp_upsert_entry` | `harness, scopeId, name, config, targetPath?` | `RestoreInfo` | `mcpUpsertEntry` |
| `skill_upsert` | `harness, scopeId, name, content` | `RestoreInfo` | `skillUpsert` |
| `marketplace_search` | `kind, query, cursor?` | `MarketPage` | `marketplaceSearch` |
| `marketplace_build_config` | `entry, packageIndex, envValues` | `BuiltConfig` | `marketplaceBuildConfig` |
| `marketplace_install` | `entry, packageIndex, targets, envValues` | `InstallResult[]` | `marketplaceInstall` |

All registered in `lib.rs` `generate_handler!`. MCP **delete** reuses the existing
`delete_item`; **undo** reuses `restore` / `bulk_restore`. Every new command mirrored in
`src/mock/dispatch.ts` + `MockStore` (+ fixtures) so `dev:mock` renders the whole feature.

`src/api.ts` also gains a concrete `McpConfig` type (`{ command?; args?; string[]; env?:
Record<string,string>; url?; headers?: Record<string,string>; type?; enabled?: boolean }`)
replacing the inline `as {...}` casts, and the `MarketEntry`/`Package`/`EnvVar`/`Remote`/
`InstallTarget`/`BuiltConfig`/`InstallResult` interfaces.

## 11. Testing strategy

- **Rust (cargo):** golden unit tests for `upsert_mcp_entry` (Claude JSON + Codex TOML),
  round-trip **preservation** (unrelated keys/tables/comments intact), undo byte-identity,
  `json_to_toml_table` ⇄ `toml_to_json`, `skill_upsert` (create + refuse-clobber + name
  validation), `parse_servers` / `parse_marketplace` against pinned registry fixtures,
  `build_mcp_config` (version-pin enforcement, secret omission, stdio/http shapes),
  `CodexOps` delete/restore, and a **scan-visibility** test (upsert → re-scan → item
  present) to prove the write target is a scanned file.
- **JS (vitest):** `MockStore` unit tests for every new store method; Organizer MCP-form
  render/edit/save/add/delete tests (by `data-testid`); Marketplace search/detail/install
  tests; updated `Sidebar.test.tsx`.
- **Fixtures:** pin a trimmed `registry-servers.json` and a `marketplace.json` sample under
  `src-tauri/src/marketplace/fixtures/` (Rust `include_str!`) and mirror in
  `src/mock/fixtures/` for the UI. **Synthetic only** — no real tokens (release CI's push
  protection scans every blob; see the fixture-secret gotcha in CLAUDE.md).
- **Green bar:** `cargo test` (src-tauri) + `npm test` + `npx tsc --noEmit` +
  `npm run build` all pass; UI verified via `npm run dev:mock` (:1430) in Chrome. Native
  window smoke is hands-on (paused for the user).

## 12. Non-goals / future

- **Claude Desktop** as a third install target — the `InstallTarget` list is built to
  accept it (`harness: "claude-desktop"`), but its adapter/write path is out of scope now.
- **Third-party skill directories** (skills.sh) — optional, later.
- **OCI/docker MCP packages** — parse & display, but install support may defer to a warning
  if Docker isn't present (never silently `docker run`).
- **Structured Codex `config.toml` scalar editing** beyond `[mcp_servers.*]` — out of scope
  (config items stay `locked`).

## 13. Plan breakdown (execute in order, SDD)

Each plan produces working, tested software on its own; author each with
`superpowers:writing-plans` immediately before executing it.

- **Plan 18 — Editable MCP (Claude) + upsert engine.** `upsert_mcp_entry` (Claude JSON) +
  `mcp-upsert` restore kind + `HarnessOps::upsert_mcp_entry` trait method + `mcp_upsert_entry`
  command + `api.mcpUpsertEntry` + mock + `McpConfig` type + Organizer structured MCP form
  (Edit/Add/Delete/Undo). Claude MCP fully editable end-to-end.
- **Plan 19 — Editable/creatable Skills.** `skill_upsert` command + validation + Organizer
  "Add Skill" flow (edit already works). Golden tests.
- **Plan 20 — Codex write path.** `toml_edit` dep + `codex_ops.rs` (`CodexOps`) + TOML
  `upsert_mcp_entry` + `json_to_toml_table` + MCP delete/restore + `ops_for("codex")` +
  flip codex MCP item flags + Codex `Enabled` field in the form. MCP editable for **both**
  harnesses.
- **Plan 21 — Marketplace: MCP servers.** `marketplace/{mod,registry,install}.rs` + registry
  parse (pure, fixture-tested) + `build_mcp_config` (version-pin + secret-safe) +
  `marketplace_search`/`marketplace_build_config`/`marketplace_install` + 6th Marketplace
  mode (Sidebar + App + Marketplace.tsx + css) + install target matrix + policy gate + mock.
- **Plan 22 — Marketplace: Skills catalog.** `marketplace/skills.rs` (curated
  `marketplace.json` repos + GitHub `SKILL.md`, pure parse + network wrapper) + Skills tab +
  install-skill fan-out to `skill_upsert`. Completes install-once-to-many for both units.
```
