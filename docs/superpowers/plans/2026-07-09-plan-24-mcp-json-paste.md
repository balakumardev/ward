# Plan 24 — MCP servers via pasted `mcpServers` JSON

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users add MCP servers by pasting the standard `{"mcpServers": {…}}` JSON block (what every MCP README publishes), not just the field-by-field form — via a "Paste JSON" tab inside the existing Add-MCP pane.

**Architecture:** A pure `parse_mcp_import` (backend) turns a pasted blob into `(name, config)` pairs; a thin `mcp_import_json` command loops the **existing** `HarnessOps::upsert_mcp_entry` writer per server (never a second writer), returning one `RestoreInfo` each. The Organizer's Add-MCP pane gets a Form/Paste-JSON segmented toggle; the Paste view previews the server names it parsed client-side, then calls the command.

**Tech Stack:** Rust (serde_json), SolidJS + TS (Vite, vitest, @solidjs/testing-library).

## Global Constraints

- **One write engine.** The import path fans out to the existing `upsert_mcp_entry` (Claude → JSON, Codex → TOML). Never a second MCP writer. (Spec §1 / CLAUDE.md.)
- **Errors:** `thiserror` `WardError` + the manual `Serialize` (`#[serde(tag="kind", content="message")]`, `rename_all="camelCase"`). A new variant must be added to the enum, the `ErrorKind` mirror, AND the `Serialize` match — all three (`src-tauri/src/error.rs`).
- Frontend ↔ core ONLY via `invoke` wrappers; JS camelCase ↔ Rust snake_case is automatic. UI never touches the FS.
- New/changed UI uses **classes + tokens** (`src/styles/app.css` / existing `mcp-*` classes), not inline styles. **Preserve every existing `data-testid`.**
- The Organizer MCP add surface is gated on the `mcpEditable()` capability — the Paste JSON tab lives inside the already-gated add pane, so it inherits the gate.
- TDD: failing test → implement → green → commit. Every `cargo test` / `npm test` passes before the commit; the commit compiles.
- One commit per task, conventional prefix. Reuse exact names: `HarnessOps`, `upsert_mcp_entry`, `mcp_upsert_entry`, `ops_for`, `harness_ctx`, `McpConfig`, `OrganizerApi`, `McpForm`, `BLANK_MCP_ITEM`.

---

## Task 1: Backend — `parse_mcp_import` + `mcp_import_json` command

**Files:**
- Modify: `src-tauri/src/error.rs` (add `WardError::InvalidInput`)
- Modify: `src-tauri/src/commands.rs` (add `parse_mcp_import` + `mcp_import_json`)
- Modify: `src-tauri/src/lib.rs` (register the command in `generate_handler!`)
- Test: `src-tauri/src/error.rs`, `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `ops_for(&str) -> Result<&'static dyn HarnessOps, WardError>`, `harness_ctx(&str) -> Result<(Ctx<'static>, Vec<Scope>), WardError>`, `HarnessOps::upsert_mcp_entry(&self, ctx, scope_id, name, config: &serde_json::Value, target_path: Option<&str>, scopes) -> Result<RestoreInfo, WardError>` (all exist).
- Produces: `pub fn parse_mcp_import(json: &str, fallback_name: Option<&str>) -> Result<Vec<(String, serde_json::Value)>, WardError>`; `#[tauri::command] pub fn mcp_import_json(harness: String, scope_id: String, json: String, fallback_name: Option<String>) -> Result<Vec<RestoreInfo>, WardError>`; `WardError::InvalidInput(String)`.

- [ ] **Step 1: Write the failing test for the error variant** — append to `error.rs` `tests`:

```rust
#[test]
fn invalid_input_serializes_camel_case() {
    let e = WardError::InvalidInput("expected a JSON object".into());
    let json = serde_json::to_string(&e).unwrap();
    assert_eq!(json, "{\"kind\":\"invalidInput\",\"message\":\"invalid input: expected a JSON object\"}");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test --lib error::tests::invalid_input_serializes_camel_case`
Expected: FAIL to compile — `WardError::InvalidInput` does not exist.

- [ ] **Step 3: Add the variant in all three places** — in `error.rs`:

In `enum WardError` (after `Registry`):
```rust
    #[error("invalid input: {0}")]
    InvalidInput(String),
```
In `enum ErrorKind` (after `Registry(String)`):
```rust
    InvalidInput(String),
```
In the `Serialize` match (after the `Registry` arm):
```rust
            WardError::InvalidInput(_) => ErrorKind::InvalidInput(message),
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cd src-tauri && cargo test --lib error`
Expected: PASS.

- [ ] **Step 5: Write the failing tests for the parser** — append to the `commands.rs` `#[cfg(test)] mod tests` (add `use super::parse_mcp_import;` if the tests module doesn't already `use super::*;`):

```rust
#[test]
fn parse_mcp_import_wrapped_multi() {
    let json = r#"{"mcpServers":{"ctx7":{"command":"npx","args":["-y","@upstash/context7-mcp"]},"fs":{"command":"uvx","args":["mcp-fs"]}}}"#;
    let mut got = parse_mcp_import(json, None).unwrap();
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].0, "ctx7");
    assert_eq!(got[0].1["command"], "npx");
    assert_eq!(got[1].0, "fs");
}

#[test]
fn parse_mcp_import_accepts_mcp_servers_alias() {
    let json = r#"{"mcp_servers":{"a":{"url":"https://x/mcp","type":"http"}}}"#;
    let got = parse_mcp_import(json, None).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "a");
    assert_eq!(got[0].1["url"], "https://x/mcp");
}

#[test]
fn parse_mcp_import_bare_single_uses_fallback_name() {
    let json = r#"{"command":"npx","args":["-y","srv"]}"#;
    let got = parse_mcp_import(json, Some("my-server")).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "my-server");
    assert_eq!(got[0].1["command"], "npx");
}

#[test]
fn parse_mcp_import_bare_single_without_name_errors() {
    let json = r#"{"command":"npx"}"#;
    let err = parse_mcp_import(json, None).unwrap_err();
    assert!(matches!(err, WardError::InvalidInput(_)));
}

#[test]
fn parse_mcp_import_bare_map() {
    let json = r#"{"srv":{"command":"npx"}}"#;
    let got = parse_mcp_import(json, None).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "srv");
}

#[test]
fn parse_mcp_import_invalid_json_errors() {
    let err = parse_mcp_import("{not json", None).unwrap_err();
    assert!(matches!(err, WardError::InvalidInput(_)));
}

#[test]
fn parse_mcp_import_non_object_server_errors() {
    let err = parse_mcp_import(r#"{"mcpServers":{"x":"nope"}}"#, None).unwrap_err();
    assert!(matches!(err, WardError::InvalidInput(_)));
}

#[test]
fn parse_mcp_import_empty_errors() {
    assert!(matches!(parse_mcp_import("{}", None).unwrap_err(), WardError::InvalidInput(_)));
    assert!(matches!(parse_mcp_import(r#"{"mcpServers":{}}"#, None).unwrap_err(), WardError::InvalidInput(_)));
}
```

- [ ] **Step 6: Run them to verify they fail**

Run: `cd src-tauri && cargo test --lib commands::tests::parse_mcp_import`
Expected: FAIL to compile — `parse_mcp_import` is not defined.

- [ ] **Step 7: Implement the parser** — add to `commands.rs` (near the other MCP command helpers; it needs `use serde_json::Value;` — add if not present at the top of the file):

```rust
/// Parse a pasted `mcpServers` JSON blob into `(name, config)` pairs ready for
/// `upsert_mcp_entry`. Validates the whole blob before any caller writes.
/// Accepts, in this precedence:
///   1. `{"mcpServers": {"<name>": {…}}}` (or `mcp_servers`) — the map value.
///   2. A bare single server (`command` or `url` string at top level) — named
///      from `fallback_name` (error if absent/blank).
///   3. A bare map `{"<name>": {…}}` — each value must be a server object.
pub fn parse_mcp_import(
    json: &str,
    fallback_name: Option<&str>,
) -> Result<Vec<(String, serde_json::Value)>, WardError> {
    let root: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| WardError::InvalidInput(format!("invalid JSON: {e}")))?;
    let obj = root
        .as_object()
        .ok_or_else(|| WardError::InvalidInput("expected a JSON object".into()))?;

    if let Some(v) = obj.get("mcpServers").or_else(|| obj.get("mcp_servers")) {
        let map = v
            .as_object()
            .ok_or_else(|| WardError::InvalidInput("\"mcpServers\" must be an object".into()))?;
        return collect_mcp_servers(map);
    }

    let is_single = obj.get("command").is_some_and(|c| c.is_string())
        || obj.get("url").is_some_and(|u| u.is_string());
    if is_single {
        let name = fallback_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| WardError::InvalidInput("a single server object needs a name".into()))?;
        return Ok(vec![(name.to_string(), root.clone())]);
    }

    collect_mcp_servers(obj)
}

fn collect_mcp_servers(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<(String, serde_json::Value)>, WardError> {
    if map.is_empty() {
        return Err(WardError::InvalidInput("no MCP servers found".into()));
    }
    let mut out = Vec::with_capacity(map.len());
    for (name, cfg) in map {
        if !cfg.is_object() {
            return Err(WardError::InvalidInput(format!(
                "server \"{name}\" must be an object"
            )));
        }
        out.push((name.clone(), cfg.clone()));
    }
    Ok(out)
}
```

- [ ] **Step 8: Run them to verify they pass**

Run: `cd src-tauri && cargo test --lib commands::tests::parse_mcp_import`
Expected: PASS (all 8).

- [ ] **Step 9: Add the command** — in `commands.rs`, after `mcp_upsert_entry`:

```rust
/// Import one or more MCP servers from a pasted `mcpServers` JSON blob. Parses +
/// validates the whole blob first, then upserts each server into `scope_id` via
/// the SAME writer the Organizer form and Marketplace use. Returns one
/// `RestoreInfo` per server (for a batch Undo).
#[tauri::command]
pub fn mcp_import_json(
    harness: String,
    scope_id: String,
    json: String,
    fallback_name: Option<String>,
) -> Result<Vec<RestoreInfo>, WardError> {
    let entries = parse_mcp_import(&json, fallback_name.as_deref())?;
    let ops = ops_for(&harness)?;
    let (ctx, scopes) = harness_ctx(&harness)?;
    let mut infos = Vec::with_capacity(entries.len());
    for (name, config) in entries {
        infos.push(ops.upsert_mcp_entry(&ctx, &scope_id, &name, &config, None, &scopes)?);
    }
    Ok(infos)
}
```

- [ ] **Step 10: Register the command** — in `src-tauri/src/lib.rs`, add `commands::mcp_import_json,` to the `tauri::generate_handler![…]` list, right after `commands::mcp_upsert_entry,`.

- [ ] **Step 11: Full backend suite green**

Run: `cd src-tauri && cargo test`
Expected: PASS (0 failed).

- [ ] **Step 12: Commit**

```bash
git add src-tauri/src/error.rs src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(mcp): parse + import pasted mcpServers JSON (fans out to upsert_mcp_entry)"
```

---

## Task 2: Frontend — "Paste JSON" tab in the Add-MCP pane

**Files:**
- Modify: `src/api.ts` (`mcpImportJson` wrapper)
- Modify: `src/App.tsx` (bridge method)
- Modify: `src/modes/Organizer.tsx` (`OrganizerApi` interface + a `previewMcpImport` helper + the Paste JSON tab in the add pane)
- Modify: `src/styles/app.css` (tab + textarea styling)
- Test: `src/modes/Organizer.test.tsx`

**Interfaces:**
- Consumes: `api.mcpImportJson` (Task 1 command); the add-pane state `addingMcp`/`newName`/`chosenScope`; `props.api.mcpImportJson` (new on `OrganizerApi`).
- Produces: `previewMcpImport(json: string): { servers: string[]; single: boolean; error?: string }` (module-scope in Organizer.tsx, mirrors the backend precedence for the live preview).

- [ ] **Step 1: Add the api.ts wrapper** — in `src/api.ts`, after `mcpUpsertEntry`:

```ts
  // Plan 24 — import one or more MCP servers from a pasted mcpServers JSON blob.
  mcpImportJson: (harness: string, scopeId: string, json: string, fallbackName?: string) =>
    invokeOrThrow<RestoreInfo[]>('mcp_import_json', { harness, scopeId, json, fallbackName }),
```

- [ ] **Step 2: Add the `OrganizerApi` method + App bridge** — in `src/modes/Organizer.tsx`, in the `OrganizerApi` interface (after `upsertMcpEntry`):

```ts
  mcpImportJson: (scopeId: string, json: string, fallbackName?: string) => Promise<RestoreInfo[]>;
```

In `src/App.tsx`, after the `upsertMcpEntry` bridge method:

```tsx
    mcpImportJson: async (scopeId: string, json: string, fallbackName?: string) => {
      const r = await api.mcpImportJson(harness(), scopeId, json, fallbackName);
      await refetch();
      return r;
    },
```

- [ ] **Step 3: Write the failing frontend test** — in `src/modes/Organizer.test.tsx`, reusing the file's existing render/mock helpers (read the file first; do NOT invent a new harness). The test switches the Add-MCP pane to the Paste JSON tab, types a blob, and asserts (a) the preview lists the parsed server name and (b) Import calls `mcpImportJson`. Sketch (adapt selectors/mocks to the existing harness):

```tsx
test('Add-MCP Paste JSON tab previews server names and imports', async () => {
  const calls: string[] = [];
  // reuse the file's existing scan/api factory; stub mcpImportJson to record the blob
  const api = makeOrganizerApi({ mcpImportJson: async (_s: string, json: string) => { calls.push(json); return []; } });
  const scan = scanWithMcpCategory(); // existing helper producing an mcp category + mcpEditable caps
  const { getByTestId, findByTestId, findByText } = render(() => <Organizer scan={scan} api={api} />);
  getByTestId('mcp-add-button').click();
  (await findByTestId('mcp-paste-tab')).click();
  const ta = await findByTestId('mcp-paste-json') as HTMLTextAreaElement;
  ta.value = '{"mcpServers":{"ctx7":{"command":"npx"}}}';
  ta.dispatchEvent(new Event('input', { bubbles: true }));
  expect(await findByText(/ctx7/)).toBeTruthy();      // preview shows the name
  getByTestId('mcp-paste-import').click();
  expect(calls[0]).toContain('ctx7');                  // import sent the blob
});
```

- [ ] **Step 4: Run it to verify it fails**

Run: `npm test -- Organizer`
Expected: FAIL — `mcp-paste-tab` / `mcp-paste-json` / `mcp-paste-import` don't exist yet.

- [ ] **Step 5: Add the preview helper** — in `src/modes/Organizer.tsx` at module scope:

```ts
/** Mirror of the backend `parse_mcp_import` precedence, for the live paste
 *  preview: returns the server names found (or a parse error). */
function previewMcpImport(json: string): { servers: string[]; single: boolean; error?: string } {
  const t = json.trim();
  if (!t) return { servers: [], single: false };
  let root: unknown;
  try { root = JSON.parse(t); } catch (e) { return { servers: [], single: false, error: `invalid JSON: ${(e as Error).message}` }; }
  if (typeof root !== 'object' || root === null || Array.isArray(root)) return { servers: [], single: false, error: 'expected a JSON object' };
  const obj = root as Record<string, unknown>;
  const wrap = (obj.mcpServers ?? obj.mcp_servers) as Record<string, unknown> | undefined;
  if (wrap && typeof wrap === 'object') {
    const names = Object.keys(wrap);
    return names.length ? { servers: names, single: false } : { servers: [], single: false, error: 'no MCP servers found' };
  }
  if (typeof obj.command === 'string' || typeof obj.url === 'string') return { servers: [], single: true };
  const names = Object.keys(obj);
  return names.length ? { servers: names, single: false } : { servers: [], single: false, error: 'no MCP servers found' };
}
```

- [ ] **Step 6: Add the tab + Paste view to the Add-MCP pane** — in the add-mode render block (the `<div class="rise">` for "Add MCP Server", around the `<McpForm mode="add" …/>`), introduce a `const [addTab, setAddTab] = createSignal<'form' | 'paste'>('form')` signal (next to the other Organizer signals) and a `const [pasteJson, setPasteJson] = createSignal('')`. Add a segmented toggle above the form, and render the McpForm OR the paste view by `addTab()`:

```tsx
            <div class="seg mcp-add-tabs">
              <button classList={{ 'seg-btn': true, active: addTab() === 'form' }}
                data-testid="mcp-form-tab" onClick={() => setAddTab('form')}>Form</button>
              <button classList={{ 'seg-btn': true, active: addTab() === 'paste' }}
                data-testid="mcp-paste-tab" onClick={() => setAddTab('paste')}>Paste JSON</button>
            </div>
            <Show when={addTab() === 'paste'} fallback={
              <McpForm mode="add" item={BLANK_MCP_ITEM} scopes={props.scan.scopes}
                name={newName()} onName={setNewName} scopeId={chosenScope()} onScope={setChosenScope}
                onSave={addMcp} harness={props.scan.harnessId} />
            }>
              <div class="mcp-paste" data-testid="mcp-paste">
                <label class="mcp-label">Scope</label>
                <select class="mcp-input mcp-scope-select" data-testid="mcp-paste-scope"
                  value={chosenScope()} onChange={(e) => setChosenScope(e.currentTarget.value)}>
                  <For each={props.scan.scopes}>{(s) => <option value={s.id}>{s.label}</option>}</For>
                </select>
                <label class="mcp-label">Paste an mcpServers JSON block (or a single server object)</label>
                <textarea class="mcp-input mcp-paste-area" data-testid="mcp-paste-json" spellcheck={false}
                  placeholder={'{\n  "mcpServers": {\n    "context7": { "command": "npx", "args": ["-y", "@upstash/context7-mcp"] }\n  }\n}'}
                  value={pasteJson()} onInput={(e) => setPasteJson(e.currentTarget.value)} />
                {/* single-server paste needs a name */}
                <Show when={previewMcpImport(pasteJson()).single}>
                  <label class="mcp-label">Name (single server)</label>
                  <input class="mcp-input" data-testid="mcp-paste-name" placeholder="server-name" spellcheck={false}
                    value={newName()} onInput={(e) => setNewName(e.currentTarget.value)} />
                </Show>
                <div class="mcp-paste-preview" data-testid="mcp-paste-preview">
                  <Show when={previewMcpImport(pasteJson()).error}>
                    <span class="mcp-paste-err">{previewMcpImport(pasteJson()).error}</span>
                  </Show>
                  <Show when={previewMcpImport(pasteJson()).servers.length > 0}>
                    <span>Will add: {previewMcpImport(pasteJson()).servers.join(', ')}</span>
                  </Show>
                </div>
                <div class="editor-foot">
                  <button class="btn btn-primary" data-testid="mcp-paste-import"
                    disabled={previewMcpImport(pasteJson()).servers.length === 0 && !previewMcpImport(pasteJson()).single}
                    onClick={() => void doImportMcp()}>Import</button>
                </div>
              </div>
            </Show>
```

Add the handler near `addMcp`:

```tsx
  async function doImportMcp(): Promise<void> {
    const prev = previewMcpImport(pasteJson());
    if (prev.error) { setStatusMsg(`MCP import failed: ${prev.error}`); return; }
    if (prev.single && !newName().trim()) { setStatusMsg('MCP import failed: a single server needs a name.'); return; }
    try {
      const infos = await props.api.mcpImportJson(chosenScope(), pasteJson(), prev.single ? newName().trim() : undefined);
      setAddingMcp(false);
      setPasteJson('');
      setAddTab('form');
      setLastUndo(infos);
      setStatusMsg(`Imported ${infos.length} server(s). Click Undo to reverse.`);
    } catch (e) {
      setStatusMsg(`MCP import failed: ${String((e as { message?: string })?.message ?? e)}`);
    }
  }
```

(If `setLastUndo` only accepts a single `RestoreInfo`, check its signature — the bulk path `doBulk` stores `RestoreInfo[]` in `lastUndo`, so `setLastUndo(infos)` matches that array form; reuse whichever the bulk undo uses.)

- [ ] **Step 7: Style it** — append to `src/styles/app.css`:

```css
.mcp-add-tabs { margin-bottom: 10px; }
.mcp-paste { display: flex; flex-direction: column; gap: 8px; }
.mcp-paste-area { min-height: 160px; font-family: var(--font-mono); font-size: 12px; resize: vertical; white-space: pre; }
.mcp-paste-preview { min-height: 18px; font-size: 12px; color: var(--text-dim); }
.mcp-paste-err { color: var(--danger); }
```

- [ ] **Step 8: Run the test to verify it passes**

Run: `npm test -- Organizer`
Expected: PASS (new test + existing Organizer tests).

- [ ] **Step 9: Verify types + full JS suite + commit**

```bash
npx tsc --noEmit
npm test
git add src/api.ts src/App.tsx src/modes/Organizer.tsx src/styles/app.css src/modes/Organizer.test.tsx
git commit -m "feat(mcp): Paste JSON tab in the Add-MCP pane (import mcpServers blobs)"
```

---

## Self-Review (completed by plan author)

**Spec coverage** (spec §2):
- Thin `mcp_import_json` fanning out to `upsert_mcp_entry` (one writer) → Task 1. ✓
- Accept wrapped / bare-single / bare-map, disambiguated by the explicit rule → `parse_mcp_import` (Task 1). ✓
- Validation errors (invalid JSON, non-object server, empty, single-without-name) → Task 1 tests. ✓
- "Paste JSON" tab in the Add-MCP pane with a live preview of names → Task 2. ✓
- Works for both harnesses (Claude JSON / Codex TOML) — inherited from `upsert_mcp_entry`; no per-harness code here. ✓

**Placeholder scan:** no TBD/TODO; every code step shows real code. (Step 6's `setLastUndo(infos)` note names the exact fallback to verify — the existing bulk-undo array form.) ✓

**Type consistency:** `parse_mcp_import(&str, Option<&str>) -> Vec<(String, Value)>` and `mcp_import_json(harness, scope_id, json, fallback_name) -> Vec<RestoreInfo>` (Rust) ↔ `mcpImportJson(harness, scopeId, json, fallbackName?) -> RestoreInfo[]` (api.ts) ↔ `mcpImportJson(scopeId, json, fallbackName?)` (OrganizerApi, harness injected by the App bridge). `previewMcpImport` mirrors the backend precedence. `WardError::InvalidInput` added in all three error.rs sites. ✓

**Deferred/noted:** a mid-batch `upsert` I/O failure (rare — parse validates the whole blob first, so the common failures are pre-write) aborts with earlier writes applied but their `RestoreInfo`s not returned; acceptable for an FS-write batch. Name-collision with an existing server is an intentional overwrite (upsert contract) — the preview shows the names so the user sees what they're adding.
