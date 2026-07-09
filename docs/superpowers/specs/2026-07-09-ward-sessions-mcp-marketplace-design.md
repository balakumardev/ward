# Ward — Sessions overhaul, MCP JSON import & multi-source Marketplace

- **Date:** 2026-07-09
- **Status:** Draft for review
- **Spec covers:** four user-requested features, decomposed into Plans 23–27.
- **Related specs:** `2026-07-06-ward-mcp-marketplace-design.md` (Plans 21–22, the
  single-source Marketplace this extends).

## 1. Summary

Four features requested together:

1. **Add MCP servers by pasting the popular `mcpServers` JSON block** (not just
   the field-by-field form).
2. **Show the auto-generated session title** in the Sessions list (Claude Code
   writes one; Ward currently discards it and shows the raw UUID).
3. **Make the session detail view readable** — hide the empty "attachment"
   noise rows and the raw-JSON dumps; show real conversation, tool calls, tool
   results, and the session's on-disk path.
4. **Add more third-party MCP + skill marketplace sources** (SkillsMP,
   Smithery, Glama, Docker MCP Catalog, GitHub skill repos), including new
   install adapters so more of them actually install.

### Decisions locked with the user

| Question | Decision |
|---|---|
| Marketplace ambition | **Multi-source, curated** (built-in sources, no user-config UI) |
| Session title source | **Summary line → first user message → UUID** |
| Transcript cleanup | **Hide noise rows + collapsible, pretty-printed tool results + show path** |
| MCP paste placement | **A "Paste JSON" tab inside the existing Add-MCP pane** |
| MCP source behaviour | **Discovery + build new installers** (OCI container + `.mcpb` bundle) |
| Skill source behaviour | **SkillsMP + GitHub Git-Trees SKILL.md discovery + regression test** |

### Plan decomposition

- **Plan 23** — Sessions: real titles + readable transcript (features 2 + 3).
- **Plan 24** — MCP: paste `mcpServers` JSON to add (feature 1).
- **Plan 25** — Marketplace: source-registry refactor + MCP discovery sources
  (Glama, Smithery, official-registry search) + UI source badges / install-shape
  gating.
- **Plan 26** — Marketplace: MCP install adapters — OCI/container + `.mcpb`
  bundle (the security-sensitive download-and-run path).
- **Plan 27** — Marketplace: Skills — SkillsMP source + GitHub Git-Trees
  SKILL.md discovery + `obra/superpowers` regression fix.

### Cross-cutting invariants (unchanged, must hold everywhere)

- **One write engine.** Every MCP install/import path fans out to the existing
  `HarnessOps::upsert_mcp_entry`; every skill install to
  `claude_ops::skill_upsert`. Never a second writer.
- **Secrets omitted, not written.** Config never stores secret values; only env
  var *names* are emitted. Optional API keys/PATs live in the macOS Keychain,
  never in config, never logged.
- **Version-pinned.** Installs reject `@latest` / empty versions.
- **Async.** Every new command that does I/O is `pub async fn` wrapping
  `spawn_blocking` with sync inner logic (unit-testable).
- **Undo.** Every write returns a `RestoreInfo` for byte-verbatim restore.
- **TDD + CCO parity.** Failing test → implement → green → commit.

---

## 2. Plan 24 — MCP servers via pasted `mcpServers` JSON (feature 1)

### Motivation
Users copy MCP servers as the standard block:
```json
{ "mcpServers": { "context7": { "command": "npx", "args": ["-y", "@upstash/context7-mcp"], "env": { "API_KEY": "…" } } } }
```
Today Ward only offers a field-by-field Add form. Paste is faster and matches
what every MCP README publishes.

### Backend
- New command **`mcp_import_json(harness, scope_id, json) -> Vec<RestoreInfo>`**
  (`commands.rs`, async + `spawn_blocking`).
- Parse with `serde_json::from_str::<serde_json::Value>`. Disambiguate the top
  level with an explicit rule (no guessing):
  1. If the object has an `"mcpServers"` (or Codex `"mcp_servers"`) key → treat
     its value as the **map of servers** `{ "<name>": {…} }`.
  2. Else if the top-level object itself has a string `"command"` **or** string
     `"url"` key → treat the whole object as a **bare single server** (the UI
     supplies the name).
  3. Else → treat the top-level object as a **bare map** `{ "<name>": {…} }`
     (each value must be an object with `command` or `url`; a value that is not
     such an object is a named validation error).
- For each `(name, config)` pair, call
  `ops.upsert_mcp_entry(&ctx, &scope_id, name, &config, None, &scopes)` — the
  same writer the Organizer form and the marketplace use. Claude → JSON, Codex →
  TOML (`toml_edit`) conversions already handled inside the impls. Returns one
  `RestoreInfo` per server (batch undo).
- **Validation / errors** (`WardError`): invalid JSON → clear parse error with
  line/col; empty object → "no servers found"; a server value that is not an
  object → named error identifying the offending key. A single bad entry reports
  which entry failed; already-applied entries are returned so the UI can report
  partial success (mirrors marketplace fan-out semantics).
- **Overwrite semantics:** `upsert` overwrites an existing `mcpServers[name]`.
  The UI must warn on name collision (see below) — the backend does not silently
  refuse (upsert is the contract), but the RestoreInfo enables undo.

### Frontend (`Organizer.tsx`)
- In the Add-MCP pane (`McpForm` add-mode, currently field-by-field), add a
  segmented **"Form / Paste JSON"** toggle. "Paste JSON" shows a `<textarea>` +
  the existing scope `<select>`.
- **Live preview:** parse on input (client-side) and list the server names
  found, flag any that already exist in the chosen scope (collision warning),
  and show a parse error inline. Disable Submit until at least one valid server.
- On submit → `api.mcpImportJson(harness, scope, text)` → `refetch()` so the new
  rows appear. Gated on `mcpEditable()` (same gate as the form).
- Bare single-server paste: if the pasted object has no names (just
  `{command,…}`), reuse the form's Name field for the single name.

### Tests
- Rust: parse wrapped multi-server; parse bare single; parse bare map; invalid
  JSON error; non-object server value error; round-trip through `upsert` for
  **both** Claude (JSON preserved) and Codex (TOML, comments/other tables
  preserved); overwrite existing name returns restorable `RestoreInfo`.
- JS: preview lists names; collision flagged; Submit gating; partial-success
  reporting.

---

## 3. Plan 23 — Sessions: real titles + readable transcript (features 2 + 3)

### 3a. Session titles

**Backend (`sessions/parse.rs`):**
- Claude Code writes `{"type":"summary","summary":"<title>","leafUuid":"<uuid>"}`
  lines. Today these hit the `_` arm → `SessionRecord::Other { record_type:
  "summary" }` and the title text is thrown away.
- Add a real record arm **`Summary { text: String, leaf_uuid: Option<String> }`**
  (tagged `kind:"summary"`, camelCase) so the text survives parsing. Keep the
  existing `System.summary` (compact_boundary) distinct — that is a different
  record.
- **Derived title** with precedence: **summary line → first user message
  (trimmed/collapsed, capped ~80 chars) → session UUID**. A file may carry
  multiple summary lines (compaction); prefer the summary whose `leafUuid`
  matches the last message, else the last summary line present.

**List title without full-parsing every transcript (perf):**
- The Sessions *list* is built by `scan_sessions` (`claude.rs`), which today only
  reads filenames (`name = stem`, `description = ""`). Full-parsing every
  transcript just to list them is too expensive for users with many sessions.
- Add a **bounded head/tail scan** `session_title(path) -> SessionMeta` that
  reads only enough of the file to extract: the summary line(s), the first user
  message, the first/last timestamp, and a message count estimate. Cap the bytes
  read (e.g. head + a tail window) so listing stays cheap. `scan_sessions` sets
  `name = title`, `description = "<relative time> · <N> messages"`.
- The detail view already full-parses on selection (`session_preview`), so it
  gets the title for free from the parsed `Conversation` (add `title:
  Option<String>` to `Conversation`).

### 3b. Readable transcript (`Sessions.tsx`)

Current problems (verified): (1) noise `Other` records — `attachment`,
`file-history-snapshot`, `progress`, `pr-link` — render as `#N · attachment`
headers with **empty bodies** (the "empty numbers saying attachment"); (2) tool
results (`toolResult.content`) are dumped **verbatim**, usually raw JSON or full
file reads (the "lots of json"); (3) the `.jsonl` path is not shown.

- **Hide noise:** suppress the `Other` record types in `DROP_TYPES`
  (reuse the exact set `sessions/distill.rs` already drops). Provide a small
  **"show N system events"** toggle so nothing is a mystery — hidden by default.
- **Tool calls / results as collapsible cards:**
  - Tool call → a compact card: `🔧 <name>` + the existing `inputSummary`,
    expandable to the full input if present.
  - Tool result → a collapsible card, **collapsed by default** past a small line
    threshold, with an expand affordance. When `content` parses as JSON,
    pretty-print it (2-space) in a mono block; otherwise render as pre-wrap text.
    Keep the existing `max-height` scroll only as a safety net for huge blocks.
  - Detect JSON cheaply: trim, first non-space char is `{`/`[`, `JSON.parse`
    succeeds → pretty-print; else raw.
- **Conversation text / thinking:** unchanged structured rendering (text blocks,
  foldable `thinking`), which already works.
- **Show path:** render `selected().path` (the `.jsonl` path) in the detail
  header — the frontend already holds `item.path`; **no backend change** needed
  for this. Add a copy-path affordance.
- **List rows:** title (from `name`) + sub-line `relative time · N messages`;
  full path in a tooltip/title attr.

### Tests
- Rust: summary line parsed into `Summary` (not `Other`); title precedence
  (summary present; summary absent → first user msg; both absent → UUID);
  `session_title` bounded scan returns the right title + count without reading
  the whole file; `Conversation.title` populated.
- JS: noise records hidden by default + toggle reveals them; tool result
  pretty-prints valid JSON and leaves non-JSON as text; long result collapsed by
  default; path rendered in header; list row shows title + meta.

---

## 4. Plan 25 — Marketplace: multi-source discovery (feature 4, part 1)

### 4a. Source-registry refactor
Today MCP discovery is a single hardcoded `const REGISTRY_URL` consumed directly
by `fetch_servers`, with no seam. Skills already aggregate a `CURATED_MARKETPLACES`
list via `search_skills_with`. Generalize MCP to match:

- Introduce a **source descriptor** list for MCP (mirroring the skills side):
  each source has `{ id, display_name, kind, fetch_fn, parse_fn }`. The `"mcp"`
  arm of `marketplace_search` becomes an **aggregator** that queries each source,
  tolerates a failing source (skips it), merges + dedupes, and tags each
  `MarketEntry.source` with the source id.
- Carry `MarketEntry.source` into the UI as a **source badge / group**
  (the field already exists; today it is discarded). Add a per-source filter.

### 4b. MCP discovery sources (no key required)

Concrete, live-probed endpoints:

- **Official registry (enhance existing):** add `search` (server-side name
  substring) and `updated_since` (RFC3339, enables incremental sync + tombstones)
  query params to the existing `registry.rs` client. Cursor pagination unchanged.
- **Glama** — `GET https://glama.ai/api/mcp/v1/servers` (+ `/servers/{id}`),
  no auth, Relay cursor `?first=N&after=<endCursor>`, `RateLimit` headers
  (~100/window). Shape: `{ pageInfo:{endCursor,hasNextPage}, servers:[{ id, name,
  namespace, slug, description, repository:{url}, spdxLicense,
  environmentVariablesJsonSchema, tools[], url }] }`. **Discovery-only** — carries
  a repo URL + env schema, **no** command/transport, so no direct install config.
  (Note: Glama flagged v1 as temporary; re-confirm before shipping.)
- **Smithery** — `GET https://api.smithery.ai/servers` (+
  `/servers/{qualifiedName}`), no key for reads (**optional** user Bearer for
  future enforcement), page-based `page`+`pageSize`≤100, filters `q`,`verified`.
  Detail `connections[]`: **http** `{type:"http", deploymentUrl, configSchema}`
  → installable as a **remote** through the existing writer; **stdio**
  `{type:"stdio", bundleUrl:"…server.mcpb", runtime, configSchema}` → needs the
  `.mcpb` installer (Plan 26).
- **Docker MCP Catalog** — `GET https://desktop.docker.com/mcp/catalog/v3/catalog.json`
  (~1.6 MB, ~318 curated, no auth, single file). Per-server source-of-truth YAML
  at `raw.githubusercontent.com/docker/mcp-registry/main/servers/<name>/server.yaml`
  with `config.secrets[]`. Every entry is a **container `image:`** → needs the
  OCI installer (Plan 26).
- *(Optional)* **toolsdk-ai** — raw GitHub JSON index
  `.../toolsdk-mcp-registry/main/indexes/packages-list.json`, no auth.
- **Skip:** mcp.so (no API, scrape-only), PulseMCP v0beta (hard sunset ramp,
  ~dead Sep 2026), awesome-* README repos.

### 4c. Install-shape gating (UI)
Because most discovery entries are not installable through the current writer, a
`MarketEntry` carries an **install capability**: `installable` (npm/pypi/http
remote we can emit) vs `discovery` (repo/env only) vs `container` / `bundle`
(needs Plan 26 adapters). The detail sheet shows an **Install** button only when
the shape is supported; otherwise a **"View / open in browser"** action to the
repo/homepage. No fabricated installs.

### Tests
- Per-source parser golden tests against pinned fixtures (Glama, Smithery
  detail, Docker catalog slice). Aggregator merges + tolerates a failing source.
- Official-registry `search`/`updated_since` params applied.
- Install-shape classification: http remote → installable; repo-only →
  discovery (View); image → container; `.mcpb` → bundle.

---

## 5. Plan 26 — Marketplace: MCP install adapters (feature 4, part 2) — SECURITY-SENSITIVE

The user chose "build new installers." Two adapters, both feeding the **same**
`upsert_mcp_entry` writer (config emission only — no second writer):

### 5a. OCI / container (Docker MCP Catalog + registry OCI packages)
- Emit a config that runs the server via Docker:
  `{"command":"docker","args":["run","-i","--rm","-e","<SECRET_NAME>",…,
  "<image>@<digest>"]}`.
- **Secret omission preserved:** pass env var **names** via `-e NAME` (Docker
  inherits the value from the host env at runtime) — never `-e NAME=value`.
- **Pin by digest** where the catalog provides one; else pin the tag. Reject
  unpinned.
- **Preflight:** detect `docker` on PATH; if absent, a clear actionable error
  (not a fabricated command). Surface the image + required secrets in the
  preview before install.

### 5b. `.mcpb` bundle (Smithery stdio servers)
`.mcpb` (MCP Bundle) install means **download + extract + run** a bundle — the
highest-risk path. Design for safety:
- **Explicit per-install consent:** a dialog naming the exact source, server,
  version, bundle URL, and declared runtime — the user must confirm each install.
  (Ward is acting on the user's behalf; this is a genuine download-and-execute.)
- **Download** the `bundleUrl` to a Ward-managed dir `~/.ward/mcpb/<name>@<ver>/`;
  **verify** size + (where available) a digest; **version/digest pin** — never a
  moving reference.
- **Extract** the bundle and emit a config that runs its declared entrypoint via
  the declared `runtime` (node/python/binary): e.g.
  `{"command":"node","args":["~/.ward/mcpb/<name>@<ver>/server/index.js"], "env":{…names…}}`.
- **Secrets** from `configSchema` are surfaced as required env var *names*
  (omitted from config, same invariant).
- **Undo** removes the config entry (RestoreInfo) and leaves the downloaded
  bundle for manual cleanup (or offers to delete it).
- **Fallback:** if download/extract/consent is declined or fails, no config is
  written and the error is explicit.

> This is the riskiest feature in the batch. If review prefers, 5b can ship
> behind a feature flag or be deferred to a follow-up while 5a (container) and
> the discovery sources ship first. Flagged for the review gate.

### Tests
- OCI: config emits `-e NAME` (no values); digest pin; missing-Docker preflight
  error; secret names surfaced not written.
- `.mcpb`: (with a local fixture bundle, no network in tests) extract + entrypoint
  config; version pin; consent-required gating; undo removes the entry; declined
  → no write.

---

## 6. Plan 27 — Marketplace: Skills — SkillsMP + tree-scan + regression fix (feature 4, part 3)

### 6a. SkillsMP source (clean, no parser change)
- `GET https://skillsmp.com/api/v1/skills/search?q=<term>` (params
  `page`,`limit`≤100,`sortBy=stars|recent`,`category`), anonymous (no key;
  **optional** user `Authorization: Bearer sk_live_…` raises the 50/day cap).
- Shape: `{ success, data:{ skills:[{ id, name, author, description, githubUrl,
  skillUrl, stars, updatedAt }], pagination:{…} } }`. Transform
  `githubUrl` (`github.com/{o}/{r}/tree/{branch}/{path}`) →
  `raw.githubusercontent.com/{o}/{r}/{branch}/{path}/SKILL.md` — **exactly** what
  the existing SKILL.md fetch + `marketplace_preview_skill` need. **No parser
  change.**
- **Dedup** near-duplicate mirror rows on `owner/repo` + leaf skill name.
- Search-only (needs `q`) — query-driven, aggregated with the curated roots.

### 6b. GitHub Git-Trees SKILL.md discovery (fixes the regression generally)
- **The regression (verified):** `obra/superpowers`'s current
  `.claude-plugin/marketplace.json` is a single **source-only** plugin
  (`{name:"superpowers", version, source:"./"}`, no `skills[]`), so
  `parse_marketplace` skips it → that curated root yields **zero** skills today.
- Add a **discovery step** for source-only plugins / roots: GitHub Git-Trees API
  `GET /repos/{o}/{r}/git/trees/{branch}?recursive=1` (unauth 60/hr; **optional**
  user PAT lifts to 5,000/hr), emit one `MarketEntry` per `**/SKILL.md`, fetch
  metadata via the existing frontmatter parser. This unlocks the long tail of
  reputable source-only skill repos, not just superpowers.
- **Regression test:** assert each curated root parses to **≥1 skill**, so a
  source-only manifest can never again silently show nothing.

### 6c. Optional keys/PATs storage
- Smithery Bearer, SkillsMP `sk_live_`, GitHub PAT are all **optional** and
  user-supplied. Store in the **macOS Keychain** (consistent with the existing
  Claude-token pattern), never in config, never logged. Absent key → the anon
  tier is used; the UI notes when a rate limit is hit and offers to add a key.

### Tests
- SkillsMP: `githubUrl` → raw SKILL.md transform (incl. nested paths); dedup of
  mirror rows; anon path (no key).
- Tree-scan: a fixture repo tree → one entry per `SKILL.md`; PAT vs anon rate
  handling (mocked); each curated root ≥1 skill (the regression guard).

---

## 7. UI summary (Marketplace)
- Source **badges** on result cards + a per-source group/filter (drive off
  `MarketEntry.source`).
- Detail sheet: **Install** when the shape is supported; **View** (open repo)
  otherwise; container/bundle installs show their preflight + consent details.
- "Add key" affordances (Keychain) surfaced only when a source rate-limits.

## 8. Security considerations (consolidated)
- **`.mcpb` download+execute** — explicit per-install consent, version/digest
  pin, Ward-managed dir; the single highest-risk feature (flagged, can be
  deferred/flagged).
- **OCI** — env var *names* only (`-e NAME`), digest pin, Docker preflight.
- **Optional keys/PATs** — Keychain only, never config, never logged (matches
  the global secrets rule).
- **Secrets omitted** from all emitted configs (existing invariant, extended to
  the new adapters).

## 9. Out of scope / deferred
- User-editable / user-added marketplace sources (a settings UI). The user chose
  curated built-in sources; the source-registry refactor (Plan 25) leaves a clean
  seam to add this later.
- Claude Desktop as an install target (the `InstallTarget` data shape already
  anticipates it).
- PulseMCP (sunsetting), mcp.so (scrape-only).

## 10. Testing strategy
- TDD throughout; per-source parsers get golden tests against **pinned
  fixtures** (no network in tests). Aggregators tested for failure tolerance.
- Reuse `sessions/cost` pricing; reuse `upsert_mcp_entry` / `skill_upsert`
  writers (assert no second writer via the fan-out tests).
- New synthetic fixtures must not contain real-looking secrets (past push-protection
  lesson) — use schema-shape-only fixtures.
