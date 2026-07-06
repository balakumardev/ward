# Plan 18 — Editable MCP (Claude) + Upsert Engine — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Claude MCP server entries fully add/edit/remove-able from the Organizer, backed by a surgical single-key upsert engine that preserves every other key and captures a byte-exact undo.

**Architecture:** A new `write_mcp_upsert` free fn in `claude_ops.rs` reuses the existing private JSON helpers (`McpParentKey`, `ensure_mcp_parent`, `insert_mcp_entry`, `detect_mcp_parent`, `read_json_or_empty`, `write_json`) to insert-or-overwrite one `mcpServers[<name>]` key and return a whole-file-backup `RestoreInfo { kind: "mcp-upsert" }`. A new `HarnessOps::upsert_mcp_entry` trait method resolves the target file + parent (edit → `item.path` + detected parent; add → `resolve_mcp_json(scope)` + flat `mcpServers`) and calls it. A `mcp_upsert_entry` Tauri command dispatches via `ops_for`. The Organizer detail pane renders a structured stdio/http form (replacing read-only JSON) whose Save/Add call the command; Delete + Undo reuse existing paths.

**Tech Stack:** Rust (serde_json, thiserror, tauri command), SolidJS + TS + Vite, vitest + @solidjs/testing-library.

## Global Constraints

- Reuse Plan 01 names verbatim — do NOT rename `WardError`, `Harness`, `HarnessOps`, `Ctx`, `Registry`, `ClaudeOps`, `RestoreInfo`, `McpParentKey`, `ensure_under_home`, `resolve_mcp_json`, `read_json_or_empty`, `write_json`.
- All Rust structs: `#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]` + `#[serde(rename_all = "camelCase")]`. Errors via `WardError` (`thiserror` + manual serialize already defined).
- Frontend ↔ core ONLY via `invoke`. JS camelCase args → Rust snake_case (automatic).
- New UI is **class-based** via `src/styles/*.css` + tokens (`--surface`, `--accent`, `--border`, `--r-sm`, etc.) — NOT inline styles. Preserve every existing `data-testid`.
- MCP item `path` is the WHOLE shared config file — NEVER full-file-save the pane; ALWAYS surgical upsert of one key.
- TDD: failing test → implement → green → commit. One conventional commit per task. **Commit `Cargo.lock`.** Every `cargo test` (run in `src-tauri/`) and `npm test` must pass before moving on.
- Rust tests run from `src-tauri/`: `cargo test`. JS: `npm test`. Typecheck: `npx tsc --noEmit`.

---

### Task 1: `write_mcp_upsert` free fn + `mcp-upsert` restore arm

**Files:**
- Modify: `src-tauri/src/harness/adapters/claude_ops.rs` (add fn near the other JSON MCP helpers ~line 756; add restore arm at ~line 242)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: existing private `McpParentKey`, `ensure_mcp_parent`, `insert_mcp_entry`, `read_json_or_empty`, `write_json` in this module; `RestoreInfo` from `crate::model`; `claude_mcp::restore_mcp_file`.
- Produces: `pub fn write_mcp_upsert(target: &Path, parent: &McpParentKey, name: &str, config: &serde_json::Value) -> Result<RestoreInfo, WardError>` returning `RestoreInfo { kind: "mcp-upsert", original_path, backup_bytes, mcp_key, mcp_parent_key, mcp_scope }`. A `"mcp-upsert"` arm in `ClaudeOps::restore` routing to `claude_mcp::restore_mcp_file`.

- [ ] **Step 1: Write the failing tests** (append inside `mod tests`)

```rust
    #[test]
    fn upsert_inserts_new_entry_into_flat_mcp_servers() {
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        let json = home.join(".claude/.mcp.json");
        fs::create_dir_all(json.parent().unwrap()).unwrap();
        fs::write(&json, r#"{"mcpServers":{"existing":{"command":"e"}}}"#).unwrap();
        let cfg = serde_json::json!({"command":"npx","args":["-y","pkg@1.0.0"]});
        let info = write_mcp_upsert(&json, &McpParentKey::mcp_servers(), "newsrv", &cfg).unwrap();
        let after: serde_json::Value = serde_json::from_str(&fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(after["mcpServers"]["newsrv"]["command"], "npx");
        assert_eq!(after["mcpServers"]["existing"]["command"], "e", "unrelated key preserved");
        assert_eq!(info.kind, "mcp-upsert");
        assert_eq!(info.mcp_key.as_deref(), Some("newsrv"));
        assert!(info.backup_bytes.is_some());
    }

    #[test]
    fn upsert_overwrites_existing_entry() {
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        let json = home.join(".claude/.mcp.json");
        fs::create_dir_all(json.parent().unwrap()).unwrap();
        fs::write(&json, r#"{"mcpServers":{"github":{"command":"old","args":["a"]}}}"#).unwrap();
        let cfg = serde_json::json!({"command":"new","args":["b","c"]});
        write_mcp_upsert(&json, &McpParentKey::mcp_servers(), "github", &cfg).unwrap();
        let after: serde_json::Value = serde_json::from_str(&fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(after["mcpServers"]["github"]["command"], "new");
        assert_eq!(after["mcpServers"]["github"]["args"], serde_json::json!(["b","c"]));
    }

    #[test]
    fn upsert_creates_file_when_missing_and_backup_is_none() {
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        let json = home.join(".claude/.mcp.json");
        assert!(!json.exists());
        let cfg = serde_json::json!({"url":"https://x.com/mcp"});
        let info = write_mcp_upsert(&json, &McpParentKey::mcp_servers(), "remote", &cfg).unwrap();
        assert!(json.exists());
        let after: serde_json::Value = serde_json::from_str(&fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(after["mcpServers"]["remote"]["url"], "https://x.com/mcp");
        assert!(info.backup_bytes.is_none(), "no prior file → no backup");
    }

    #[test]
    fn upsert_undo_restores_byte_identical_and_removes_when_created() {
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        let json = home.join(".claude/.mcp.json");
        fs::create_dir_all(json.parent().unwrap()).unwrap();
        let original = "{\n  \"mcpServers\": {\n    \"a\": {\n      \"command\": \"x\"\n    }\n  }\n}\n";
        fs::write(&json, original).unwrap();
        let ops = ClaudeOps;
        let info = write_mcp_upsert(&json, &McpParentKey::mcp_servers(), "b",
            &serde_json::json!({"command":"y"})).unwrap();
        ops.restore(&ctx_for(home), &info).unwrap();
        assert_eq!(fs::read_to_string(&json).unwrap(), original, "edit undo is byte-identical");

        // create case: undo removes the file
        let json2 = home.join(".claude/fresh.mcp.json");
        let info2 = write_mcp_upsert(&json2, &McpParentKey::mcp_servers(), "c",
            &serde_json::json!({"command":"z"})).unwrap();
        assert!(json2.exists());
        ops.restore(&ctx_for(home), &info2).unwrap();
        assert!(!json2.exists(), "create undo removes the file");
    }
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p ward upsert_ 2>&1 | tail -20` (from `src-tauri/`)
Expected: FAIL — `cannot find function write_mcp_upsert in this scope`.

- [ ] **Step 3: Implement `write_mcp_upsert`** (add after `insert_mcp_entry`, ~line 756)

```rust
/// Surgically insert-or-overwrite one `mcpServers[<name>]` key in `target`,
/// preserving every other key. Whole prior file bytes are captured in the
/// returned `RestoreInfo` (kind `"mcp-upsert"`) so undo is byte-exact for an
/// edit and a clean removal for a create (mirrors `claude_mcp::set_policy`).
pub fn write_mcp_upsert(target: &Path, parent: &McpParentKey, name: &str,
                        config: &serde_json::Value) -> Result<RestoreInfo, WardError> {
    let backup_bytes = std::fs::read(target).unwrap_or_default();
    let mut root = read_json_or_empty(target)?;
    ensure_mcp_parent(&mut root, parent);
    insert_mcp_entry(&mut root, parent, name, config.clone());
    if let Some(dir) = target.parent() { std::fs::create_dir_all(dir)?; }
    write_json(target, &root)?;
    Ok(RestoreInfo {
        kind: "mcp-upsert".into(),
        original_path: target.display().to_string(),
        current_path: None,
        backup_bytes: if backup_bytes.is_empty() { None } else { Some(backup_bytes) },
        mcp_entry: None,
        mcp_key: Some(name.to_string()),
        mcp_parent_key: Some(parent.object_key().to_string()),
        mcp_scope: parent.scope_key().map(|s| s.to_string()),
    })
}
```

Add the restore arm in `ClaudeOps::restore` (the `match info.kind.as_str()` block ~line 239):

```rust
            "mcp-disabled" | "mcp-policy" | "mcp-upsert" =>
                crate::harness::adapters::claude_mcp::restore_mcp_file(ctx.home, info),
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p ward upsert_ 2>&1 | tail -20`
Expected: PASS (4 tests). Then `cargo test -p ward 2>&1 | tail -5` — full suite green.

- [ ] **Step 5: Commit**

```bash
git -C /Users/balakumar/personal/ward add src-tauri/src/harness/adapters/claude_ops.rs
git -C /Users/balakumar/personal/ward commit -m "feat(mcp): surgical write_mcp_upsert + mcp-upsert undo (claude)"
```

---

### Task 2: `HarnessOps::upsert_mcp_entry` trait method + ClaudeOps impl

**Files:**
- Modify: `src-tauri/src/harness/mod.rs` (add trait method ~line 39)
- Modify: `src-tauri/src/harness/adapters/claude_ops.rs` (impl the method in `impl HarnessOps for ClaudeOps`)
- Test: `claude_ops.rs` tests

**Interfaces:**
- Consumes: `write_mcp_upsert` (Task 1), `resolve_mcp_json` (existing pub), `detect_mcp_parent` (existing private), `ensure_under_home`.
- Produces: `HarnessOps::upsert_mcp_entry(&self, ctx: &Ctx, scope_id: &str, name: &str, config: &serde_json::Value, target_path: Option<&str>, scopes: &[Scope]) -> Result<RestoreInfo, WardError>` — the harness-dispatched upsert. `ClaudeOps` implements it.

- [ ] **Step 1: Write the failing tests** (append to `claude_ops.rs` tests)

```rust
    #[test]
    fn ops_upsert_edit_existing_writes_back_to_item_path() {
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        let json = home.join(".claude.json");
        fs::write(&json, r#"{"mcpServers":{"github":{"command":"gh"}}}"#).unwrap();
        let ops = ClaudeOps;
        let scopes = scopes_for(home, &home.join("work/project-a"));
        let info = ops.upsert_mcp_entry(&ctx_for(home), "global", "github",
            &serde_json::json!({"command":"gh","args":["api"]}),
            Some(&json.display().to_string()), &scopes).unwrap();
        let after: serde_json::Value = serde_json::from_str(&fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(after["mcpServers"]["github"]["args"][0], "api");
        assert_eq!(info.kind, "mcp-upsert");
    }

    #[test]
    fn ops_upsert_add_new_resolves_global_mcp_json() {
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        let scopes = scopes_for(home, &home.join("work/project-a"));
        let ops = ClaudeOps;
        // No target_path → resolves ~/.claude/.mcp.json (global) + flat mcpServers.
        let info = ops.upsert_mcp_entry(&ctx_for(home), "global", "brandnew",
            &serde_json::json!({"command":"npx","args":["-y","x@1.0.0"]}), None, &scopes).unwrap();
        let target = home.join(".claude/.mcp.json");
        assert!(target.exists(), "global add lands in ~/.claude/.mcp.json");
        let after: serde_json::Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(after["mcpServers"]["brandnew"]["command"], "npx");
        assert_eq!(info.mcp_parent_key.as_deref(), Some("mcpServers"));
    }

    #[test]
    fn ops_upsert_add_new_scan_visible() {
        // Proves the resolved write target is a file ClaudeAdapter scans.
        use crate::harness::adapters::claude::ClaudeAdapter;
        use crate::harness::framework;
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        fs::create_dir_all(home.join(".claude")).unwrap();
        let ctx = Ctx { home, cwd: None };
        let scopes = framework::run_scan(&ClaudeAdapter, &ctx).unwrap().scopes;
        ClaudeOps.upsert_mcp_entry(&ctx, "global", "visible-srv",
            &serde_json::json!({"command":"echo"}), None, &scopes).unwrap();
        let items = framework::run_scan(&ClaudeAdapter, &ctx).unwrap().items;
        assert!(items.iter().any(|i| i.category == "mcp" && i.name == "visible-srv"),
            "upserted server must appear in a fresh scan");
    }

    #[test]
    fn ops_upsert_rejects_target_outside_home() {
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        let scopes = scopes_for(home, &home.join("work/project-a"));
        let bad = home.join("../evil.json");
        let res = ClaudeOps.upsert_mcp_entry(&ctx_for(home), "global", "x",
            &serde_json::json!({"command":"y"}), Some(&bad.display().to_string()), &scopes);
        assert!(res.is_err(), "target outside home must be rejected");
    }
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p ward ops_upsert 2>&1 | tail -20`
Expected: FAIL — `no method named upsert_mcp_entry`.

- [ ] **Step 3: Add the trait method + impl**

In `src-tauri/src/harness/mod.rs`, inside `pub trait HarnessOps`, after `save_file`:

```rust
    /// Insert-or-overwrite one MCP server entry. `target_path == Some` edits
    /// that exact file (parent auto-detected); `None` resolves the write
    /// target from `scope_id` (a new server). Returns an undo payload.
    fn upsert_mcp_entry(&self, ctx: &Ctx, scope_id: &str, name: &str,
        config: &serde_json::Value, target_path: Option<&str>, scopes: &[Scope])
        -> Result<RestoreInfo, WardError>;
```

In `claude_ops.rs`, inside `impl HarnessOps for ClaudeOps` (after `save_file`):

```rust
    fn upsert_mcp_entry(&self, ctx: &Ctx, scope_id: &str, name: &str,
        config: &serde_json::Value, target_path: Option<&str>, scopes: &[Scope])
        -> Result<RestoreInfo, WardError>
    {
        let (target, parent) = match target_path {
            Some(tp) => {
                let p = ensure_under_home(Path::new(tp), ctx.home)?;
                let parent = detect_mcp_parent(&p, name, scopes);
                (p, parent)
            }
            None => {
                let p = resolve_mcp_json(scope_id, scopes)
                    .ok_or_else(|| WardError::NotFound(format!("Cannot resolve .mcp.json for {scope_id}")))?;
                let p = ensure_under_home(&p, ctx.home)?;
                (p, McpParentKey::mcp_servers())
            }
        };
        write_mcp_upsert(&target, &parent, name, config)
    }
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p ward ops_upsert 2>&1 | tail -20` → PASS (4).
Then `cargo test -p ward 2>&1 | tail -5` — full suite green (confirms no other `HarnessOps` impl needs the method yet; only `ClaudeOps` exists).

- [ ] **Step 5: Commit**

```bash
git -C /Users/balakumar/personal/ward add src-tauri/src/harness/mod.rs src-tauri/src/harness/adapters/claude_ops.rs
git -C /Users/balakumar/personal/ward commit -m "feat(mcp): HarnessOps::upsert_mcp_entry + ClaudeOps target/parent resolution"
```

---

### Task 3: `mcp_upsert_entry` Tauri command + registration

**Files:**
- Modify: `src-tauri/src/commands.rs` (add command after `mcp_check_policy` ~line 217)
- Modify: `src-tauri/src/lib.rs` (add to `generate_handler!` list ~line 326)
- Test: `commands.rs` tests (exercise via `ClaudeOps` like the existing `bulk` tests)

**Interfaces:**
- Consumes: `ops_for`, `harness_ctx` (existing), `HarnessOps::upsert_mcp_entry` (Task 2).
- Produces: `#[tauri::command] pub fn mcp_upsert_entry(harness: String, scope_id: String, name: String, config: serde_json::Value, target_path: Option<String>) -> Result<RestoreInfo, WardError>`.

- [ ] **Step 1: Write the failing test** (append to `commands.rs` tests)

```rust
    #[test]
    fn mcp_upsert_via_ops_round_trips() {
        // The command itself needs dirs::home_dir(); exercise the ops path it
        // delegates to (same pattern as the bulk tests above).
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let json = home.join(".claude.json");
        fs::write(&json, r#"{"mcpServers":{}}"#).unwrap();
        let ops = crate::harness::adapters::claude_ops::ClaudeOps;
        let ctx = Ctx { home, cwd: None };
        let scopes = vec![Scope { id: "global".into(), kind: "global".into(),
            label: "Global".into(), root: home.join(".claude").display().to_string() }];
        let info = ops.upsert_mcp_entry(&ctx, "global", "srv",
            &serde_json::json!({"command":"c"}), Some(&json.display().to_string()), &scopes).unwrap();
        assert_eq!(info.kind, "mcp-upsert");
        let after: serde_json::Value = serde_json::from_str(&fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(after["mcpServers"]["srv"]["command"], "c");
    }
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p ward mcp_upsert_via_ops 2>&1 | tail -15`
Expected: FAIL (compile error until the ops method exists — it does after Task 2, so this test actually passes; if so, keep it as a regression guard and proceed. The real deliverable of this task is the command wiring, verified by `cargo check` compiling the new command + handler.)

- [ ] **Step 3: Add the command + register it**

`commands.rs` after `mcp_check_policy`:

```rust
/// Insert-or-overwrite one MCP server entry (Add or Edit from the Organizer).
/// `target_path == Some` edits that exact config file; `None` resolves the
/// scope's write target for a new server. Returns a `RestoreInfo` for Undo.
#[tauri::command]
pub fn mcp_upsert_entry(
    harness: String,
    scope_id: String,
    name: String,
    config: serde_json::Value,
    target_path: Option<String>,
) -> Result<RestoreInfo, WardError> {
    let ops = ops_for(&harness)?;
    let (ctx, scopes) = harness_ctx(&harness)?;
    ops.upsert_mcp_entry(&ctx, &scope_id, &name, &config, target_path.as_deref(), &scopes)
}
```

`lib.rs` — add `commands::mcp_upsert_entry,` to the `generate_handler!` list (after `commands::mcp_check_policy,`).

- [ ] **Step 4: Run tests + check**

Run: `cargo test -p ward 2>&1 | tail -5` → all pass.
Run: `cargo check 2>&1 | tail -5` → clean (confirms the command signature is a valid Tauri command and the handler compiles).

- [ ] **Step 5: Commit**

```bash
git -C /Users/balakumar/personal/ward add src-tauri/src/commands.rs src-tauri/src/lib.rs
git -C /Users/balakumar/personal/ward commit -m "feat(mcp): mcp_upsert_entry Tauri command + handler registration"
```

---

### Task 4: `McpConfig` type + `api.mcpUpsertEntry` + `RestoreInfo` kind

**Files:**
- Modify: `src/api.ts` (add `McpConfig` interface ~line 66; extend `RestoreInfo.kind` union ~line 57; add `mcpUpsertEntry` to `api` object ~line 362)
- Test: `src/api.test.ts`

**Interfaces:**
- Produces: `export interface McpConfig { command?: string; args?: string[]; env?: Record<string,string>; url?: string; headers?: Record<string,string>; type?: string; enabled?: boolean }`; `api.mcpUpsertEntry(harness, scopeId, name, config, targetPath?)` → `invoke('mcp_upsert_entry', { harness, scopeId, name, config, targetPath })`.

- [ ] **Step 1: Write the failing test** (append to `src/api.test.ts`)

```ts
  it('mcpUpsertEntry invokes mcp_upsert_entry with camelCase args', async () => {
    const spy = installInvokeSpy({ kind: 'mcp-upsert', originalPath: '/x' });
    await api.mcpUpsertEntry('claude', 'global', 'srv', { command: 'npx', args: ['-y', 'p@1.0.0'] }, '/Users/x/.claude.json');
    expect(spy).toHaveBeenCalledWith('mcp_upsert_entry', {
      harness: 'claude', scopeId: 'global', name: 'srv',
      config: { command: 'npx', args: ['-y', 'p@1.0.0'] }, targetPath: '/Users/x/.claude.json',
    });
  });
```

(Use the existing `api.test.ts` invoke-spy helper; match its established name/shape — inspect the top of `api.test.ts` and reuse whatever pattern the other tests use to stub `window.__TAURI_INTERNALS__.invoke`.)

- [ ] **Step 2: Run test, verify it fails**

Run: `npm test -- api.test 2>&1 | tail -15`
Expected: FAIL — `api.mcpUpsertEntry is not a function`.

- [ ] **Step 3: Implement the type + method**

In `src/api.ts`, extend the `RestoreInfo` `kind` union to include `'mcp-upsert'`:

```ts
  kind: 'file' | 'mcp-entry' | 'mcp-disabled' | 'mcp-policy' | 'mcp-upsert';
```

Add the `McpConfig` interface near the other MCP types:

```ts
export interface McpConfig {
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  headers?: Record<string, string>;
  type?: string;
  enabled?: boolean;
  [key: string]: unknown; // preserve unknown keys round-trip
}
```

Add to the `api` object (after `mcpCheckPolicy`):

```ts
  mcpUpsertEntry: (harness: string, scopeId: string, name: string, config: McpConfig, targetPath?: string) =>
    invokeOrThrow<RestoreInfo>('mcp_upsert_entry', { harness, scopeId, name, config, targetPath }),
```

- [ ] **Step 4: Run test + typecheck**

Run: `npm test -- api.test 2>&1 | tail -15` → PASS.
Run: `npx tsc --noEmit 2>&1 | tail -5` → clean.

- [ ] **Step 5: Commit**

```bash
git -C /Users/balakumar/personal/ward add src/api.ts src/api.test.ts
git -C /Users/balakumar/personal/ward commit -m "feat(mcp): McpConfig type + api.mcpUpsertEntry + mcp-upsert RestoreInfo kind"
```

---

### Task 5: Mock bridge — `mcp_upsert_entry` dispatch + `MockStore.upsertMcpEntry`

**Files:**
- Modify: `src/mock/dispatch.ts` (add a `case` in the `switch`)
- Modify: `src/mock/store.ts` (add `upsertMcpEntry` method)
- Test: `src/mock/store.test.ts`

**Interfaces:**
- Consumes: existing `MockStore` internals (`scanFor`, `newUndo`, `MockRestore`).
- Produces: `MockStore.upsertMcpEntry(harness, scopeId, name, config, targetPath?)` — patches an existing MCP item's `mcpConfig` in place OR inserts a new MCP item into the harness scan; returns a `MockRestore { kind: 'mcp-upsert', ... , __undoId }`. Dispatch: `case 'mcp_upsert_entry'`.

- [ ] **Step 1: Write the failing tests** (append to `src/mock/store.test.ts`)

```ts
  it('upsertMcpEntry edits an existing MCP item config in place', () => {
    const s = new MockStore();
    const before = s.scan('claude').items.find((i) => i.category === 'mcp')!;
    const r = s.upsertMcpEntry('claude', before.scopeId, before.name, { command: 'edited', args: ['z'] }, before.path);
    expect(r.kind).toBe('mcp-upsert');
    const after = s.scan('claude').items.find((i) => i.category === 'mcp' && i.name === before.name)!;
    expect((after.mcpConfig as { command: string }).command).toBe('edited');
  });

  it('upsertMcpEntry adds a new MCP item and undo removes it', () => {
    const s = new MockStore();
    const n0 = s.scan('claude').items.filter((i) => i.category === 'mcp').length;
    const r = s.upsertMcpEntry('claude', 'global', 'brandnew', { command: 'npx', args: ['-y', 'p@1.0.0'] });
    const n1 = s.scan('claude').items.filter((i) => i.category === 'mcp').length;
    expect(n1).toBe(n0 + 1);
    s.restore({ ...r });
    const n2 = s.scan('claude').items.filter((i) => i.category === 'mcp').length;
    expect(n2).toBe(n0);
  });
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `npm test -- store.test 2>&1 | tail -15`
Expected: FAIL — `s.upsertMcpEntry is not a function`.

- [ ] **Step 3: Implement**

In `src/mock/store.ts`, add (mirroring the existing `moveItem`/`setDisabled` methods and their `newUndo` usage):

```ts
  upsertMcpEntry(harness: string, scopeId: string, name: string, config: unknown, targetPath?: string): MockRestore {
    const s = this.scanFor(harness);
    const idx = s.items.findIndex((i) => i.category === 'mcp' && i.name === name && i.scopeId === scopeId);
    if (idx >= 0) {
      const prev = s.items[idx].mcpConfig;
      s.items[idx] = { ...s.items[idx], mcpConfig: config };
      const undoId = this.newUndo(() => { s.items[idx] = { ...s.items[idx], mcpConfig: prev }; });
      return { kind: 'mcp-upsert', originalPath: s.items[idx].path, __undoId: undoId };
    }
    const newItem = {
      category: 'mcp', scopeId, name,
      path: targetPath ?? `${scopeId}/.mcp.json`,
      movable: false, deletable: true, locked: false, mcpConfig: config,
    } as (typeof s.items)[number];
    s.items.push(newItem);
    const undoId = this.newUndo(() => {
      const j = s.items.indexOf(newItem);
      if (j >= 0) s.items.splice(j, 1);
    });
    return { kind: 'mcp-upsert', originalPath: newItem.path, __undoId: undoId };
  }
```

In `src/mock/dispatch.ts`, add to the `switch`:

```ts
    case 'mcp_upsert_entry':
      await delay(60);
      return store.upsertMcpEntry(args.harness ?? 'claude', args.scopeId, args.name, args.config, args.targetPath);
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `npm test -- store.test 2>&1 | tail -15` → PASS.
Run: `npx tsc --noEmit 2>&1 | tail -5` → clean.

- [ ] **Step 5: Commit**

```bash
git -C /Users/balakumar/personal/ward add src/mock/dispatch.ts src/mock/store.ts src/mock/store.test.ts
git -C /Users/balakumar/personal/ward commit -m "feat(mcp): mock upsertMcpEntry (edit-in-place + add + undo)"
```

---

### Task 6: Organizer MCP structured form (Edit) + OrganizerApi/App bridge

**Files:**
- Create: `src/styles/organizer.css` (MCP form styles; import it at the top of `Organizer.tsx`)
- Modify: `src/modes/Organizer.tsx` (replace the read-only MCP block with a structured form; extend `OrganizerApi`)
- Modify: `src/App.tsx` (add `upsertMcpEntry` to the `organizerApi` bridge)
- Test: `src/modes/Organizer.test.tsx`

**Interfaces:**
- Consumes: `props.api.upsertMcpEntry(item, config)` and `item.mcpConfig`.
- Produces: `OrganizerApi.upsertMcpEntry(item: HarnessItem, config: McpConfig) -> Promise<RestoreInfo>`; a rendered MCP form (`data-testid="mcp-form"`) with transport toggle + stdio/http fields; Save persists via the bridge.

- [ ] **Step 1: Write the failing test** (append to `Organizer.test.tsx`, following the file's existing render/harness helpers)

```tsx
  it('renders a structured MCP form and saves an edited arg via upsertMcpEntry', async () => {
    const upsertSpy = vi.fn().mockResolvedValue({ kind: 'mcp-upsert', originalPath: '/x' });
    const scan = makeScanWithMcp({ name: 'context7', scopeId: 'global',
      path: '/Users/x/.claude.json', mcpConfig: { command: 'npx', args: ['-y', 'a@1.0.0'] } });
    renderOrganizer({ scan, api: { ...fakeApi, upsertMcpEntry: upsertSpy } });
    // select the MCP item
    fireEvent.click(screen.getByText('context7'));
    // form is present, not the read-only notice
    expect(await screen.findByTestId('mcp-form')).toBeInTheDocument();
    expect(screen.queryByTestId('detail-editor')).not.toBeInTheDocument();
    // edit the command field
    const cmd = screen.getByTestId('mcp-command') as HTMLInputElement;
    fireEvent.input(cmd, { target: { value: 'uvx' } });
    fireEvent.click(screen.getByTestId('mcp-save'));
    await waitFor(() => expect(upsertSpy).toHaveBeenCalled());
    const [item, config] = upsertSpy.mock.calls[0];
    expect(item.name).toBe('context7');
    expect(config.command).toBe('uvx');
    expect(config.args).toEqual(['-y', 'a@1.0.0']); // preserved
  });
```

(If `makeScanWithMcp` / `renderOrganizer` / `fakeApi` helpers don't already exist in `Organizer.test.tsx`, add small local helpers modeled on the file's existing setup — reuse its real `ScanResult` fixture shape and `render` wrapper.)

- [ ] **Step 2: Run test, verify it fails**

Run: `npm test -- Organizer.test 2>&1 | tail -20`
Expected: FAIL — no `mcp-form` testid (read-only pane still renders).

- [ ] **Step 3: Implement the form + bridge**

In `src/modes/Organizer.tsx`:
1. Add `import '../styles/organizer.css';` at the top.
2. Extend the `OrganizerApi` interface with:
   `upsertMcpEntry: (item: HarnessItem, config: McpConfig) => Promise<RestoreInfo>;`
   (import `McpConfig` from `../api`).
3. Replace the MCP read-only branch in the detail pane. Where it currently renders the disabled `detail-editor` + read-only notice for `isMcp()`, render an `<McpForm>` instead. Add this component in the same file:

```tsx
function McpForm(props: { item: HarnessItem; onSave: (config: McpConfig) => Promise<void> }) {
  const seed = () => (props.item.mcpConfig ?? {}) as McpConfig;
  const [transport, setTransport] = createSignal<'stdio' | 'http'>(seed().url ? 'http' : 'stdio');
  const [command, setCommand] = createSignal(seed().command ?? '');
  const [args, setArgs] = createSignal<string[]>([...(seed().args ?? [])]);
  const [env, setEnv] = createSignal<[string, string][]>(Object.entries(seed().env ?? {}));
  const [url, setUrl] = createSignal(seed().url ?? '');
  const [headers, setHeaders] = createSignal<[string, string][]>(Object.entries(seed().headers ?? {}));
  const [busy, setBusy] = createSignal(false);

  async function save() {
    setBusy(true);
    // Patch a CLONE of the original so unknown keys survive.
    const next: McpConfig = { ...(props.item.mcpConfig as McpConfig ?? {}) };
    if (transport() === 'stdio') {
      next.command = command();
      next.args = args();
      next.env = Object.fromEntries(env().filter(([k]) => k));
      delete next.url; delete next.headers;
    } else {
      next.url = url();
      next.headers = Object.fromEntries(headers().filter(([k]) => k));
      delete next.command; delete next.args; delete next.env;
    }
    try { await props.onSave(next); } finally { setBusy(false); }
  }

  return (
    <div class="mcp-form" data-testid="mcp-form">
      <div class="seg mcp-transport">
        <button classList={{ 'seg-btn': true, active: transport() === 'stdio' }}
          data-testid="mcp-transport-stdio" onClick={() => setTransport('stdio')}>stdio</button>
        <button classList={{ 'seg-btn': true, active: transport() === 'http' }}
          data-testid="mcp-transport-http" onClick={() => setTransport('http')}>http</button>
      </div>
      <Show when={transport() === 'stdio'} fallback={
        <>
          <label class="mcp-label">URL</label>
          <input class="mcp-input" data-testid="mcp-url" value={url()} onInput={(e) => setUrl(e.currentTarget.value)} />
          <KeyValRows label="Headers" rows={headers()} setRows={setHeaders} testid="mcp-header" />
        </>
      }>
        <label class="mcp-label">Command</label>
        <input class="mcp-input" data-testid="mcp-command" value={command()} onInput={(e) => setCommand(e.currentTarget.value)} />
        <ListRows label="Args" rows={args()} setRows={setArgs} testid="mcp-arg" />
        <KeyValRows label="Env" rows={env()} setRows={setEnv} testid="mcp-env" />
      </Show>
      <div class="editor-foot">
        <button class="btn btn-primary" data-testid="mcp-save" disabled={busy()} onClick={() => void save()}>Save</button>
      </div>
    </div>
  );
}
```

Add small `ListRows` (single-value add/remove) and `KeyValRows` (key/value add/remove) helper components in the same file, each rendering `${testid}-add`, `${testid}-row`, `${testid}-remove` controls (single-value: `${testid}-input`; key/value: `${testid}-key` / `${testid}-value`). Wire the MCP branch to render `<McpForm item={item()} onSave={saveMcp} />` where `saveMcp` calls `props.api.upsertMcpEntry(item(), config)` then shows a toast/undo (reuse the existing `lastUndo` + toast wiring used by `doToggleMcpDisabled`).

In `src/App.tsx`, add to the `organizerApi` object:

```ts
  upsertMcpEntry: async (item, config) => {
    const r = await api.mcpUpsertEntry(harness(), item.scopeId, item.name, config, item.path);
    await refetch();
    return r;
  },
```

- [ ] **Step 4: Run tests + typecheck**

Run: `npm test -- Organizer.test 2>&1 | tail -20` → PASS.
Run: `npx tsc --noEmit 2>&1 | tail -5` → clean.
Run: `npm test 2>&1 | tail -6` → full JS suite green (confirms no existing Organizer test regressed on the removed read-only notice; update any test that asserted the old `Read-only — MCP server entry` string).

- [ ] **Step 5: Commit**

```bash
git -C /Users/balakumar/personal/ward add src/modes/Organizer.tsx src/App.tsx src/styles/organizer.css src/modes/Organizer.test.tsx
git -C /Users/balakumar/personal/ward commit -m "feat(mcp): structured MCP edit form in Organizer + upsert bridge"
```

---

### Task 7: Organizer "Add MCP Server" flow + Undo + verification

**Files:**
- Modify: `src/modes/Organizer.tsx` (Add control + blank-form dialog with name + scope)
- Modify: `src/styles/organizer.css` (add-dialog styles)
- Test: `src/modes/Organizer.test.tsx`

**Interfaces:**
- Consumes: `props.api.upsertMcpEntry` (Task 6) with an "add" call shape (a new item stub whose `path` is empty so App passes `targetPath: undefined` → Rust resolves the scope target).
- Produces: `+ Add MCP` affordance (`data-testid="mcp-add-button"`) → form with `mcp-name` + `mcp-scope-pick` → `mcp-save` creates the server.

- [ ] **Step 1: Write the failing test** (append to `Organizer.test.tsx`)

```tsx
  it('adds a new MCP server via the Add flow (no targetPath → scope resolves)', async () => {
    const upsertSpy = vi.fn().mockResolvedValue({ kind: 'mcp-upsert', originalPath: '/x' });
    const scan = makeScanWithMcp({ name: 'context7', scopeId: 'global',
      path: '/Users/x/.claude.json', mcpConfig: { command: 'npx' } });
    renderOrganizer({ scan, api: { ...fakeApi, upsertMcpEntry: upsertSpy } });
    fireEvent.click(screen.getByTestId('category-mcp'));
    fireEvent.click(screen.getByTestId('mcp-add-button'));
    fireEvent.input(screen.getByTestId('mcp-name'), { target: { value: 'newsrv' } });
    fireEvent.input(screen.getByTestId('mcp-command'), { target: { value: 'npx' } });
    fireEvent.click(screen.getByTestId('mcp-save'));
    await waitFor(() => expect(upsertSpy).toHaveBeenCalled());
    const [item, config] = upsertSpy.mock.calls[0];
    expect(item.name).toBe('newsrv');
    expect(item.path).toBe(''); // App forwards undefined targetPath for a create
    expect(config.command).toBe('npx');
  });
```

- [ ] **Step 2: Run test, verify it fails**

Run: `npm test -- Organizer.test 2>&1 | tail -20`
Expected: FAIL — no `mcp-add-button`.

- [ ] **Step 3: Implement the Add flow**

Add a `+ Add` button in the items-column header when the selected category is `mcp` (near the `mcp-policy-button`), `data-testid="mcp-add-button"`. Clicking sets an `addingMcp` signal that makes the detail pane render `<McpForm>` in "add" mode: a `name` input (`mcp-name`) and a scope `<select>`/rail (`mcp-scope-pick`, options from `props.scan.scopes`) become visible, and `mcp-save` calls:

```ts
await props.api.upsertMcpEntry(
  { category: 'mcp', scopeId: chosenScope(), name: newName(), path: '', movable: false, deletable: true, locked: false },
  config,
);
```

Extend `McpForm` with an optional `mode: 'add' | 'edit'` prop: in `add` mode render the name input + scope picker and start from an empty config; in `edit` mode keep name read-only (as Task 6). After a successful add, clear `addingMcp`, show the toast/undo, and rely on App's `refetch()` to surface the new row.

Ensure the App bridge forwards `item.path` (empty string) as `targetPath` — update the bridge to pass `item.path || undefined`:

```ts
  upsertMcpEntry: async (item, config) => {
    const r = await api.mcpUpsertEntry(harness(), item.scopeId, item.name, config, item.path || undefined);
    await refetch();
    return r;
  },
```

- [ ] **Step 4: Run tests + full green bar**

Run: `npm test -- Organizer.test 2>&1 | tail -20` → PASS.
Run: `npm test 2>&1 | tail -6` → all JS pass.
Run: `npx tsc --noEmit 2>&1 | tail -5` → clean.
Run: `npm run build 2>&1 | tail -8` → build succeeds.
Run (from `src-tauri/`): `cargo test 2>&1 | tail -5` → all Rust pass.

- [ ] **Step 5: dev:mock UI smoke (Chrome DevTools MCP)**

Start `npm run dev:mock` (Vite :1430). In Chrome, open `http://localhost:1430/`, go to Organizer → MCP category. Verify: (a) selecting a server shows the structured form (not raw JSON); (b) editing `command`/`args`/`env` + Save updates the row and shows Undo; (c) `+ Add MCP` creates a new server that appears in the list; (d) Delete + Undo still work. Capture the state; report any console errors.

- [ ] **Step 6: Commit**

```bash
git -C /Users/balakumar/personal/ward add src/modes/Organizer.tsx src/App.tsx src/styles/organizer.css src/modes/Organizer.test.tsx
git -C /Users/balakumar/personal/ward commit -m "feat(mcp): Add MCP Server flow in Organizer + dev:mock verification"
```

---

## Self-Review

- **Spec coverage:** §5.1 (Claude upsert engine) → Tasks 1–3; §6 (editable MCP Organizer) → Tasks 6–7; §10 command/api additions (`mcp_upsert_entry`, `mcpUpsertEntry`, `McpConfig`, mock) → Tasks 3–5. Codex (§5.2/§8), Skills (§7), Marketplace (§9) are Plans 20/19/21–22 — out of scope here.
- **Type consistency:** `write_mcp_upsert(target, parent, name, config)` (Task 1) is called by `ClaudeOps::upsert_mcp_entry` (Task 2); `mcp_upsert_entry` command args (`harness, scopeId, name, config, targetPath`) match `api.mcpUpsertEntry` (Task 4) and the mock dispatch (Task 5). `RestoreInfo.kind` gains `'mcp-upsert'` in both Rust (Task 1) and TS (Task 4). `McpConfig` defined in Task 4, consumed in Tasks 6–7.
- **Undo:** every write returns `RestoreInfo { kind: 'mcp-upsert' }` routed through `restore_mcp_file` (whole-file), exercised in Task 1 Step 1.
- **No placeholders:** every step ships real code; no TODO/subset.
```
