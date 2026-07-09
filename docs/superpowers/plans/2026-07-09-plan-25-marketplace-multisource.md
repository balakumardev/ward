# Plan 25 — Marketplace: multi-source MCP discovery (Glama + Smithery)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn Ward's single hardcoded MCP registry into a multi-source aggregator (mirroring the skills side), add **Glama** and **Smithery** as discovery sources, add server-side `search` to the official registry, and gate the UI on an **install shape** (Install for what we can emit today; "View" for discovery/container entries) with a per-source badge.

**Architecture:** The `"mcp"` arm of `marketplace_search` moves from a direct `fetch_servers()` call to a `search_mcp_with(sources, query)` aggregator (the exact shape of the existing `search_skills_with`). Each source is a `fn(&str /*query*/) -> Result<Vec<MarketEntry>, WardError>` that does its own server-side search and mints its own `source` string. A new `MarketEntry.install_shape` (`"installable" | "container" | "discovery"`) is computed from packages/remotes at parse time; the UI shows Install only for `"installable"`, else a "View" link to `repo_url`.

**Tech Stack:** Rust (`ureq`, `serde_json`), SolidJS + TS.

## Global Constraints

- **One write engine + secrets omitted + version-pinned:** unchanged. New sources only expand *discovery*; the install path still flows through `build_mcp_config` → `upsert_mcp_entry`, secrets are never written, versions must be pinned. (Spec §1/§4.)
- **Errors:** reuse `WardError::Registry(String)` for all marketplace network/parse paths — do NOT add a new variant (appendix §8).
- **Async:** `marketplace_search` stays `async` + `spawn_blocking`.
- **Rust↔TS parity:** adding `install_shape` touches BOTH `MarketEntry` (`mod.rs`) and `api.ts`, the round-trip test `market_entry_serializes_camel_case` (`mod.rs`), and EVERY `MarketEntry { … }` literal: `registry.rs` `parse_one`, `skills.rs` `build_skill_entry`, and the `install.rs` test fixtures (`npm_entry`, `pypi_entry`, `remote_entry`, `skill_entry`, inline `empty`) + `mod.rs` test literal. Use `#[serde(default)]` so a missing field deserializes cleanly, but STILL add the field to every literal (a struct literal must name every field).
- New/changed UI uses classes + tokens (`src/styles/marketplace.css`), not inline styles. Preserve every `data-testid`.
- Pinned test fixtures only; **no real-looking secrets** in fixtures (push-protection lesson).
- TDD; one commit per task; conventional prefix. Reuse exact names: `MarketEntry`, `MarketPage`, `Package`, `Remote`, `EnvVar`, `parse_servers`, `parse_one`, `dedupe_by_name`, `search_skills_with`, `filter_market_page`, `marketplace_search`, `build_mcp_config`.

---

## Task 1: `MarketEntry.install_shape` + classifier

**Files:**
- Modify: `src-tauri/src/marketplace/mod.rs` (field + classifier + round-trip test + test literal)
- Modify: `src-tauri/src/marketplace/registry.rs` (`parse_one` sets it)
- Modify: `src-tauri/src/marketplace/skills.rs` (`build_skill_entry` sets it → `"discovery"`)
- Modify: `src-tauri/src/marketplace/install.rs` (5 test fixtures add the field)
- Modify: `src/api.ts` (`MarketEntry.installShape`)
- Test: `src-tauri/src/marketplace/mod.rs`

**Interfaces:**
- Produces: `MarketEntry.install_shape: String` (serde `installShape`); `pub fn classify_install_shape(packages: &[Package], remotes: &[Remote]) -> String` in `mod.rs`.

- [ ] **Step 1: Write the failing classifier test** — append to `mod.rs` `tests`:

```rust
#[test]
fn classify_install_shape_covers_the_three_cases() {
    let npm = Package { registry_type: "npm".into(), identifier: "x".into(), version: "1.0.0".into(), transport: "stdio".into(), env: vec![], runtime_hint: None };
    let oci = Package { registry_type: "oci".into(), identifier: "img".into(), version: "1.0.0".into(), transport: "stdio".into(), env: vec![], runtime_hint: None };
    let http = Remote { transport: "streamable-http".into(), url: "https://x/mcp".into(), headers: vec![] };
    assert_eq!(classify_install_shape(&[npm.clone()], &[]), "installable");     // npm/pypi package
    assert_eq!(classify_install_shape(&[], &[http]), "installable");            // http/sse remote
    assert_eq!(classify_install_shape(&[oci], &[]), "container");               // oci image
    assert_eq!(classify_install_shape(&[], &[]), "discovery");                  // repo/env only
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test --lib marketplace::tests::classify_install_shape_covers_the_three_cases`
Expected: FAIL to compile — `classify_install_shape` undefined.

- [ ] **Step 3: Add the classifier** — in `mod.rs`, after the struct defs:

```rust
/// Classify how an entry can be installed today, so the UI shows Install only
/// for what `build_mcp_config` can emit. `"installable"` = an npm/pypi package
/// or an http/sse remote (buildable now); `"container"` = an OCI image (needs
/// the container installer, Plan 26); `"discovery"` = repo/env only (browse, no
/// direct install).
pub fn classify_install_shape(packages: &[Package], remotes: &[Remote]) -> String {
    let installable_pkg = packages
        .iter()
        .any(|p| matches!(p.registry_type.as_str(), "npm" | "pypi"));
    let has_remote = !remotes.is_empty();
    if installable_pkg || has_remote {
        return "installable".into();
    }
    if packages.iter().any(|p| p.registry_type == "oci") {
        return "container".into();
    }
    "discovery".into()
}
```

- [ ] **Step 4: Add the field** — in `mod.rs` `MarketEntry`, after `remotes` (before `repo_url`):

```rust
    /// How this entry installs: `"installable"` | `"container"` | `"discovery"`.
    /// Computed at parse time via `classify_install_shape`.
    #[serde(default)]
    pub install_shape: String,
```

- [ ] **Step 5: Update the round-trip test + set the field in every literal**

In `mod.rs`'s `market_entry_serializes_camel_case` test literal, add `install_shape: "installable".into(),` and assert the JSON contains `"installShape":"installable"`.

In `registry.rs` `parse_one`, add before the `MarketEntry { … }` return:
```rust
    let install_shape = super::classify_install_shape(&packages, &remotes);
```
and add `install_shape,` to the struct literal.

In `skills.rs` `build_skill_entry`, add `install_shape: "discovery".into(),` to the `MarketEntry { … }` literal (skills are never MCP-installable).

In `install.rs`, add `install_shape: "installable".into(),` to each `MarketEntry { … }` test fixture (`npm_entry`, `pypi_entry`, `remote_entry`, inline `empty`) and `install_shape: "discovery".into(),` to `skill_entry`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib marketplace`
Expected: PASS.

- [ ] **Step 7: Mirror in TS** — in `src/api.ts` `MarketEntry`, after `remotes`:

```ts
  installShape?: string; // "installable" | "container" | "discovery"
```

Run: `npx tsc --noEmit` → clean.

- [ ] **Step 8: Full backend suite + commit**

```bash
cd src-tauri && cargo test    # 0 failed
cd .. && git add src-tauri/src/marketplace/mod.rs src-tauri/src/marketplace/registry.rs src-tauri/src/marketplace/skills.rs src-tauri/src/marketplace/install.rs src/api.ts
git commit -m "feat(marketplace): classify MarketEntry install_shape (installable/container/discovery)"
```

---

## Task 2: MCP multi-source aggregator + official-registry `search`

**Files:**
- Modify: `src-tauri/src/marketplace/registry.rs` (new `search_servers(query)`; make `dedupe_by_name` `pub(crate)`)
- Modify: `src-tauri/src/commands.rs` (`search_mcp_with` aggregator + `CURATED_MCP_SOURCES`; the `"mcp"` arm calls the aggregator)
- Test: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `parse_servers`, `dedupe_by_name` (registry.rs), `filter_market_page` (commands.rs).
- Produces: `pub fn registry::search_servers(query: &str) -> Result<Vec<MarketEntry>, WardError>`; `pub(crate) registry::dedupe_by_name`; `commands.rs` `type McpFetcher = fn(&str) -> Result<Vec<MarketEntry>, WardError>;` + `CURATED_MCP_SOURCES: &[(&str, McpFetcher)]` + `fn search_mcp_with(sources, query) -> MarketPage`.

- [ ] **Step 1: Write the failing aggregator test** — append to `commands.rs` `tests`:

```rust
#[test]
fn search_mcp_with_merges_and_dedupes_sources() {
    fn src_a(_q: &str) -> Result<Vec<crate::marketplace::MarketEntry>, WardError> {
        Ok(vec![mk_entry("io.x/dup", "1.0.0", "a"), mk_entry("io.x/only-a", "1.0.0", "a")])
    }
    fn src_b(_q: &str) -> Result<Vec<crate::marketplace::MarketEntry>, WardError> {
        Ok(vec![mk_entry("io.x/dup", "2.0.0", "b"), mk_entry("io.x/only-b", "1.0.0", "b")])
    }
    fn src_fail(_q: &str) -> Result<Vec<crate::marketplace::MarketEntry>, WardError> {
        Err(WardError::Registry("boom".into()))
    }
    let sources: &[(&str, McpFetcher)] = &[("a", src_a), ("b", src_b), ("fail", src_fail)];
    let page = search_mcp_with(sources, "");
    // A failing source is skipped, not fatal; dup collapses to the higher version.
    assert_eq!(page.entries.len(), 3);
    let dup = page.entries.iter().find(|e| e.name == "io.x/dup").unwrap();
    assert_eq!(dup.version.as_deref(), Some("2.0.0"));
    assert!(page.next_cursor.is_none()); // multi-source drops the single-cursor model
}
```
Add a small `mk_entry` helper in the tests module if one doesn't exist:
```rust
fn mk_entry(name: &str, version: &str, source: &str) -> crate::marketplace::MarketEntry {
    crate::marketplace::MarketEntry {
        kind: "mcp".into(), name: name.into(), display_name: name.into(),
        description: String::new(), source: source.into(), version: Some(version.into()),
        verified: true, packages: vec![], remotes: vec![], install_shape: "discovery".into(),
        repo_url: None, skill_path: None,
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test --lib commands::tests::search_mcp_with_merges_and_dedupes_sources`
Expected: FAIL to compile — `search_mcp_with` / `McpFetcher` / `CURATED_MCP_SOURCES` undefined.

- [ ] **Step 3: Add `search_servers` + expose `dedupe_by_name`** — in `registry.rs`:

Change `fn dedupe_by_name` to `pub(crate) fn dedupe_by_name`.

Add (keep the existing `fetch_servers` for its tests):
```rust
/// Search the official registry server-side via its `search` param (name
/// substring), returning entries (no cursor — the multi-source aggregator does
/// not page). Empty query lists the first page.
pub fn search_servers(query: &str) -> Result<Vec<MarketEntry>, WardError> {
    let mut req = ureq::get(REGISTRY_URL)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .query("limit", PAGE_LIMIT);
    let q = query.trim();
    if !q.is_empty() {
        req = req.query("search", q);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => {
            return Err(WardError::Registry(format!("the MCP registry returned HTTP {code}")));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(WardError::Registry(format!("network error reaching the MCP registry: {t}")));
        }
    };
    let body = resp
        .into_string()
        .map_err(|e| WardError::Registry(format!("read registry response body: {e}")))?;
    Ok(parse_servers(&body)?.entries)
}
```

- [ ] **Step 4: Add the aggregator + source list + rewire the command** — in `commands.rs`:

```rust
/// A single MCP discovery source: server-side search over a query → entries.
type McpFetcher = fn(&str) -> Result<Vec<crate::marketplace::MarketEntry>, WardError>;

/// Built-in MCP discovery sources. Each mints its own `source` string and does
/// its own server-side search; a failing source is skipped, never fatal.
const CURATED_MCP_SOURCES: &[(&str, McpFetcher)] = &[
    ("registry", crate::marketplace::registry::search_servers),
];

/// Fan out over the MCP sources (mirrors `search_skills_with`), merge, dedupe
/// by name (highest version wins across sources), then substring-filter.
fn search_mcp_with(sources: &[(&str, McpFetcher)], query: &str) -> crate::marketplace::MarketPage {
    let mut entries = Vec::new();
    for (_id, fetch) in sources {
        if let Ok(mut es) = fetch(query) {
            entries.append(&mut es);
        }
    }
    let entries = crate::marketplace::registry::dedupe_by_name(entries);
    filter_market_page(
        crate::marketplace::MarketPage { entries, next_cursor: None },
        query,
    )
}
```

Change the `"mcp"` arm of `marketplace_search` from:
```rust
        "mcp" => {
            let page = crate::marketplace::registry::fetch_servers(cursor.as_deref())?;
            Ok(filter_market_page(page, &query))
        }
```
to:
```rust
        "mcp" => Ok(search_mcp_with(CURATED_MCP_SOURCES, &query)),
```
(`cursor` is now unused by the `"mcp"` arm — it's still a command param for wire-compat; prefix with `_` at the binding if the compiler warns, or leave it since the `"skill"`/`_` arms exist. Verify no unused-variable warning; if one appears, rename the command param to `_cursor` is wrong (breaks the JS payload key) — instead add `let _ = &cursor;` at the top of the closure, or keep the skill/other arms referencing it. Simplest: keep `cursor` named and add `let _ = cursor.as_deref();` is unnecessary — confirm with a build.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib commands::tests::search_mcp_with_merges_and_dedupes_sources` then `cargo test` (full).
Expected: PASS, 0 failed, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/marketplace/registry.rs src-tauri/src/commands.rs
git commit -m "feat(marketplace): MCP multi-source aggregator + official-registry search"
```

---

## Task 3: Glama source (discovery)

Glama's public API (`GET https://glama.ai/api/mcp/v1/servers`, no key) returns rich metadata but no command/transport — so every Glama entry is `install_shape: "discovery"` with a `repo_url` for the "View" link.

**Files:**
- Create: `src-tauri/src/marketplace/glama.rs`
- Modify: `src-tauri/src/marketplace/mod.rs` (`pub mod glama;`)
- Modify: `src-tauri/src/commands.rs` (add `("glama", glama::search)` to `CURATED_MCP_SOURCES`)
- Create: `src-tauri/src/marketplace/fixtures/glama-servers.json` (pinned, synthetic)
- Test: `src-tauri/src/marketplace/glama.rs`

**Interfaces:**
- Produces: `pub fn glama::parse_servers(body: &str) -> Result<Vec<MarketEntry>, WardError>`; `pub fn glama::search(query: &str) -> Result<Vec<MarketEntry>, WardError>`.

- [ ] **Step 1: Create the pinned fixture** — `src-tauri/src/marketplace/fixtures/glama-servers.json` (synthetic, matches Glama's shape; NO real secrets):

```json
{
  "pageInfo": { "endCursor": "abc", "hasNextPage": true },
  "servers": [
    { "id": "glama-1", "name": "acme-notes", "namespace": "acme", "slug": "acme-notes",
      "description": "Notes server", "repository": { "url": "https://github.com/acme/notes-mcp" },
      "spdxLicense": "MIT", "environmentVariablesJsonSchema": { "type": "object", "properties": { "NOTES_TOKEN": { "type": "string" } } },
      "url": "https://glama.ai/mcp/servers/acme-notes" },
    { "id": "glama-2", "name": "", "description": "no name — skipped", "url": "x" }
  ]
}
```

- [ ] **Step 2: Write the failing parser test** — in `glama.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = include_str!("fixtures/glama-servers.json");

    #[test]
    fn parses_glama_entries_as_discovery_with_repo_url() {
        let entries = parse_servers(FIXTURE).unwrap();
        assert_eq!(entries.len(), 1); // the empty-name row is skipped
        let e = &entries[0];
        assert_eq!(e.name, "acme-notes");
        assert_eq!(e.source, "glama");
        assert_eq!(e.install_shape, "discovery");
        assert_eq!(e.repo_url.as_deref(), Some("https://github.com/acme/notes-mcp"));
        assert!(e.packages.is_empty() && e.remotes.is_empty());
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cd src-tauri && cargo test --lib marketplace::glama`
Expected: FAIL — module/`parse_servers` don't exist.

- [ ] **Step 4: Implement `glama.rs`** — declare `pub mod glama;` in `mod.rs`, then create `glama.rs`:

```rust
//! Glama MCP directory source (https://glama.ai/api/mcp/v1/servers). No key
//! required. Glama carries rich metadata but no command/transport, so every
//! entry is `install_shape: "discovery"` with a repo URL for the "View" link.

use std::time::Duration;

use serde_json::Value;

use super::{classify_install_shape, MarketEntry};
use crate::error::WardError;

const GLAMA_URL: &str = "https://glama.ai/api/mcp/v1/servers";
const TIMEOUT_SECS: u64 = 10;
const PAGE_LIMIT: &str = "50";

/// Parse a Glama `/servers` response body into discovery `MarketEntry`s.
pub fn parse_servers(body: &str) -> Result<Vec<MarketEntry>, WardError> {
    let root: Value = serde_json::from_str(body)
        .map_err(|e| WardError::Registry(format!("parse glama response: {e}")))?;
    let mut out = Vec::new();
    if let Some(servers) = root.get("servers").and_then(|v| v.as_array()) {
        for s in servers {
            let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if name.is_empty() {
                continue;
            }
            let description = s.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let repo_url = s
                .get("repository")
                .and_then(|r| r.get("url"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string());
            out.push(MarketEntry {
                kind: "mcp".into(),
                name: name.clone(),
                display_name: name,
                description,
                source: "glama".into(),
                version: None,
                verified: false, // Glama is a community directory, not the signed registry
                packages: vec![],
                remotes: vec![],
                install_shape: classify_install_shape(&[], &[]), // "discovery"
                repo_url,
                skill_path: None,
            });
        }
    }
    Ok(out)
}

/// Server-side search over Glama (query filters by name/description).
pub fn search(query: &str) -> Result<Vec<MarketEntry>, WardError> {
    let mut req = ureq::get(GLAMA_URL)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .query("first", PAGE_LIMIT);
    let q = query.trim();
    if !q.is_empty() {
        req = req.query("query", q);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => {
            return Err(WardError::Registry(format!("Glama returned HTTP {code}")));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(WardError::Registry(format!("network error reaching Glama: {t}")));
        }
    };
    let body = resp
        .into_string()
        .map_err(|e| WardError::Registry(format!("read glama response body: {e}")))?;
    parse_servers(&body)
}
```

- [ ] **Step 5: Wire it into the source list** — in `commands.rs`, add to `CURATED_MCP_SOURCES`:
```rust
    ("glama", crate::marketplace::glama::search),
```

- [ ] **Step 6: Run tests + full suite + commit**

```bash
cd src-tauri && cargo test --lib marketplace::glama && cargo test
cd .. && git add src-tauri/src/marketplace/glama.rs src-tauri/src/marketplace/mod.rs src-tauri/src/commands.rs src-tauri/src/marketplace/fixtures/glama-servers.json
git commit -m "feat(marketplace): add Glama as an MCP discovery source"
```

---

## Task 4: Smithery source (http = installable, stdio = discovery)

Smithery (`GET https://api.smithery.ai/servers`, no key for reads) lists servers; the detail `connections[]` distinguishes **http** (a `deploymentUrl` → an installable remote) from **stdio** (`.mcpb` bundle — discovery-only, since the `.mcpb` installer was dropped). To keep it to a single list call (no per-server detail fetch in the search path), classify from the list-level `remote` boolean: `remote: true` → `install_shape: "discovery"` with a homepage/repo link and NO fabricated remote config (we don't have the `deploymentUrl` without the detail call), `remote: false`/stdio → `"discovery"`. **All Smithery search-list entries are `"discovery"`** in this cut — the `deploymentUrl` needed to make an http entry installable requires the detail endpoint, which is a future enhancement. Surface them for browse + "View".

**Files:**
- Create: `src-tauri/src/marketplace/smithery.rs`
- Modify: `src-tauri/src/marketplace/mod.rs` (`pub mod smithery;`)
- Modify: `src-tauri/src/commands.rs` (add `("smithery", smithery::search)`)
- Create: `src-tauri/src/marketplace/fixtures/smithery-servers.json` (pinned, synthetic)
- Test: `src-tauri/src/marketplace/smithery.rs`

**Interfaces:**
- Produces: `pub fn smithery::parse_servers(body: &str) -> Result<Vec<MarketEntry>, WardError>`; `pub fn smithery::search(query: &str) -> Result<Vec<MarketEntry>, WardError>`.

- [ ] **Step 1: Pinned fixture** — `fixtures/smithery-servers.json`:

```json
{
  "servers": [
    { "qualifiedName": "@acme/weather", "displayName": "Weather", "description": "Weather MCP",
      "homepage": "https://smithery.ai/server/@acme/weather", "remote": true, "isDeployed": true, "useCount": 42, "verified": true, "iconUrl": "x" },
    { "qualifiedName": "", "description": "no name — skipped" }
  ],
  "pagination": { "currentPage": 1, "pageSize": 50, "totalPages": 1, "totalCount": 1 }
}
```

- [ ] **Step 2: Failing parser test** — in `smithery.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = include_str!("fixtures/smithery-servers.json");

    #[test]
    fn parses_smithery_entries_as_discovery() {
        let entries = parse_servers(FIXTURE).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.name, "@acme/weather");
        assert_eq!(e.display_name, "Weather");
        assert_eq!(e.source, "smithery");
        assert_eq!(e.install_shape, "discovery");
        assert_eq!(e.repo_url.as_deref(), Some("https://smithery.ai/server/@acme/weather"));
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cd src-tauri && cargo test --lib marketplace::smithery`
Expected: FAIL — module missing.

- [ ] **Step 4: Implement `smithery.rs`** — declare `pub mod smithery;` in `mod.rs`, then:

```rust
//! Smithery MCP registry source (https://api.smithery.ai/servers). No key
//! required for reads. The list endpoint doesn't carry the `deploymentUrl`
//! needed to build an installable remote (that's the per-server detail call),
//! and stdio servers install as `.mcpb` bundles which Ward does not run — so
//! every Smithery search-list entry is `install_shape: "discovery"` with a
//! homepage link for "View".

use std::time::Duration;

use serde_json::Value;

use super::{classify_install_shape, MarketEntry};
use crate::error::WardError;

const SMITHERY_URL: &str = "https://api.smithery.ai/servers";
const TIMEOUT_SECS: u64 = 10;
const PAGE_SIZE: &str = "50";

pub fn parse_servers(body: &str) -> Result<Vec<MarketEntry>, WardError> {
    let root: Value = serde_json::from_str(body)
        .map_err(|e| WardError::Registry(format!("parse smithery response: {e}")))?;
    let mut out = Vec::new();
    if let Some(servers) = root.get("servers").and_then(|v| v.as_array()) {
        for s in servers {
            let name = s.get("qualifiedName").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if name.is_empty() {
                continue;
            }
            let display_name = s
                .get("displayName")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
                .unwrap_or_else(|| name.clone());
            let description = s.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let repo_url = s
                .get("homepage")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string());
            let verified = s.get("verified").and_then(|v| v.as_bool()).unwrap_or(false);
            out.push(MarketEntry {
                kind: "mcp".into(),
                name: name.clone(),
                display_name,
                description,
                source: "smithery".into(),
                version: None,
                verified,
                packages: vec![],
                remotes: vec![],
                install_shape: classify_install_shape(&[], &[]), // "discovery"
                repo_url,
                skill_path: None,
            });
        }
    }
    Ok(out)
}

pub fn search(query: &str) -> Result<Vec<MarketEntry>, WardError> {
    let mut req = ureq::get(SMITHERY_URL)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .query("pageSize", PAGE_SIZE);
    let q = query.trim();
    if !q.is_empty() {
        req = req.query("q", q);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => {
            return Err(WardError::Registry(format!("Smithery returned HTTP {code}")));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(WardError::Registry(format!("network error reaching Smithery: {t}")));
        }
    };
    let body = resp
        .into_string()
        .map_err(|e| WardError::Registry(format!("read smithery response body: {e}")))?;
    parse_servers(&body)
}
```

- [ ] **Step 5: Wire it in** — `commands.rs` `CURATED_MCP_SOURCES`:
```rust
    ("smithery", crate::marketplace::smithery::search),
```

- [ ] **Step 6: Run tests + full suite + commit**

```bash
cd src-tauri && cargo test --lib marketplace::smithery && cargo test
cd .. && git add src-tauri/src/marketplace/smithery.rs src-tauri/src/marketplace/mod.rs src-tauri/src/commands.rs src-tauri/src/marketplace/fixtures/smithery-servers.json
git commit -m "feat(marketplace): add Smithery as an MCP discovery source"
```

---

## Task 5: Frontend — source badges + install-shape gating (Install vs View)

**Files:**
- Modify: `src/modes/Marketplace.tsx`
- Modify: `src/styles/marketplace.css`
- Test: `src/modes/Marketplace.test.tsx`

**Interfaces:**
- Consumes: `MarketEntry.source` (now displayed), `MarketEntry.installShape`, `MarketEntry.repoUrl`.

- [ ] **Step 1: Write the failing tests** — in `Marketplace.test.tsx` (reuse the existing render/mock helpers — READ the file first). Assert: (a) a card shows its `source` badge; (b) a `discovery` entry's detail shows a **View** link (to `repoUrl`) and NOT an enabled Install; (c) an `installable` entry still shows Install.

```tsx
test('cards show a source badge and discovery entries show View instead of Install', async () => {
  // build a market page with one installable (registry) + one discovery (glama) entry via the existing mock
  const { getByTestId, findAllByTestId, findByTestId, queryByTestId } = renderMarketplaceWith([
    mkMcp({ name: 'io.x/inst', source: 'registry', installShape: 'installable', packages: [npmPkg()] }),
    mkMcp({ name: 'io.x/disc', source: 'glama', installShape: 'discovery', repoUrl: 'https://github.com/x/y' }),
  ]);
  // source badges rendered
  expect((await findAllByTestId('market-source')).length).toBeGreaterThanOrEqual(2);
  // select the discovery entry → View link, no enabled Install
  (await findByTestId('market-card-io.x/disc')).click(); // adapt to the harness's card selector
  expect(await findByTestId('market-view')).toBeTruthy();
  expect(queryByTestId('market-install')).toBeNull(); // Install not rendered for discovery
});
```
(Adapt selectors/mocks to the actual `Marketplace.test.tsx` harness.)

- [ ] **Step 2: Run it to verify it fails**

Run: `npm test -- Marketplace`
Expected: FAIL — no `market-source` badge, no `market-view`, Install always rendered.

- [ ] **Step 3: Add the source badge to the card** — in `Marketplace.tsx` `mkt-card`, after the verified badge:

```tsx
        <span class="mkt-source" data-testid="market-source">{e.source}</span>
```

- [ ] **Step 4: Gate Install vs View in the detail sheet** — wrap the existing `market-install` action so it renders Install ONLY for `installable`, else a View link. Replace the `<div class="mkt-actions">…Install…</div>` (the shared action block) with:

```tsx
<div class="mkt-actions">
  <Show
    when={selected()?.kind === 'skill' || selected()?.installShape === 'installable'}
    fallback={
      <Show when={selected()?.repoUrl} fallback={
        <span class="mkt-shape-note" data-testid="market-shape-note">
          {selected()?.installShape === 'container'
            ? 'Container image — install with your container runtime.'
            : 'Discovery only — no direct install.'}
        </span>
      }>
        <a class="mkt-view-btn" data-testid="market-view" href={selected()!.repoUrl} target="_blank" rel="noreferrer noopener">
          View source ↗
        </a>
      </Show>
    }
  >
    <button data-testid="market-install" class="mkt-install-btn" disabled={!canInstall()} onClick={doInstall}>
      {installing() ? 'Installing…' : `Install to ${selectedTargets().length} target${selectedTargets().length === 1 ? '' : 's'}`}
    </button>
    <Show when={!isSkill() && verdict() === 'denied'}>
      <span class="mkt-deny-note" data-testid="market-deny-note">
        Blocked by your MCP policy — adjust it in the Organizer to install this server.
      </span>
    </Show>
  </Show>
</div>
```

(Skills keep their existing Install path — the `kind === 'skill'` branch of the `when` preserves it. MCP `installable` entries keep Install; `container`/`discovery` show View or a shape note. Opening an external link is a plain `<a target="_blank" rel="noreferrer noopener">` — the app's `opener` capability already allows external URLs; no new capability needed since this is a normal anchor, not `invoke`.)

- [ ] **Step 5: Style it** — append to `src/styles/marketplace.css`:

```css
.mkt-source { font-size: 10px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-dim); border: 1px solid var(--border); border-radius: var(--r-sm); padding: 1px 6px; }
.mkt-view-btn { display: inline-block; text-decoration: none; color: var(--accent); border: 1px solid var(--border-accent); border-radius: var(--r-sm); padding: 6px 12px; font-size: 13px; }
.mkt-view-btn:hover { background: var(--accent-bg-2); }
.mkt-shape-note { color: var(--text-dim); font-size: 12px; }
```

- [ ] **Step 6: Run tests + typecheck + full JS suite + commit**

```bash
npm test -- Marketplace
npx tsc --noEmit
npm test
git add src/modes/Marketplace.tsx src/styles/marketplace.css src/modes/Marketplace.test.tsx
git commit -m "feat(marketplace): source badges + install-shape gating (Install vs View)"
```

---

## Self-Review (completed by plan author)

**Spec coverage** (spec §4):
- Single-registry → multi-source aggregator mirroring `search_skills_with` → Task 2. ✓
- Glama + Smithery discovery sources → Tasks 3, 4. ✓ (Docker + OCI moved to Plan 26 — Docker entries are container-shape and need the OCI installer.)
- Official-registry `search` param → Task 2. ✓
- Source badges + install-shape gating (no fabricated installs; View for discovery/container) → Tasks 1 (shape) + 5 (UI). ✓

**Placeholder scan:** no TBD/TODO; every code step shows real code. Task 2 Step 4 flags the exact `cursor`-unused check to confirm at build. ✓

**Type consistency:** `install_shape`/`installShape` added to Rust+TS+every literal (Task 1 enumerates them); `McpFetcher`/`CURATED_MCP_SOURCES`/`search_mcp_with` (Task 2) consumed by Tasks 3–4's `("glama"/"smithery", …::search)` additions; each source's `parse_servers`/`search` follow the same signature as `registry::search_servers`. `classify_install_shape` used by all three parsers. ✓

**Deferred (noted):** Smithery http entries are `discovery` in this cut (making them installable needs the per-server detail call for `deploymentUrl` — a future enhancement); multi-source search drops deep pagination (`next_cursor: None`), relying on per-source server-side `search` — acceptable for a search-driven UI, and matches the skills aggregator. Docker MCP Catalog + OCI installer = Plan 26.
