# Plan 19 — Creatable Skills (Add Skill) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Let users create a new Skill from the Organizer (scaffold `<dir>/<name>/SKILL.md`, then edit it in the existing markdown editor). Editing existing skills already works via `save_file`; this plan adds only the create path, gated on a `skillCreatable` capability (Claude true; Codex false until its write path lands in Plan 20).

**Architecture:** A new `skill_upsert` core fn (name validation + per-harness skills-dir resolution + create-only write + whole-dir undo) in `claude_ops.rs`, exposed as a `skill_upsert` Tauri command. A new `skillCreatable` capability gates the Organizer's `+ Add Skill` control (mirrors Plan 18's `mcpEditable`). The Add flow scaffolds a starter SKILL.md, re-scans, and selects the new item so the user fills it in with the existing editor and Saves via the existing `save_file` path.

**Tech Stack:** Rust (serde_json, thiserror, tauri command), SolidJS + TS + Vite, vitest + @solidjs/testing-library.

## Global Constraints

- Reuse existing names verbatim — do NOT rename `WardError`, `HarnessOps`, `Ctx`, `ClaudeOps`, `RestoreInfo`, `Capabilities`, `ensure_under_home`, `resolve_skill_dir`, `Scope`, `HarnessItem`.
- All Rust structs: `#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]` + `#[serde(rename_all = "camelCase")]`. Errors via `WardError`.
- Frontend ↔ core ONLY via `invoke`; JS camelCase → Rust snake_case (automatic).
- New UI class-based via `src/styles/*.css` + tokens — NOT inline styles (layout-only inline consistent with existing file patterns is tolerated, as in the Plan 18 Add view). Preserve every existing `data-testid`.
- A new skill's on-disk layout is `<skills_dir>/<name>/SKILL.md`. Create is **create-only** — refuse to overwrite an existing skill dir. Editing an existing skill continues through the existing markdown editor + `save_file` (unchanged).
- Skill `name` validation: non-empty; only lowercase letters, digits, and hyphens (`^[a-z0-9][a-z0-9-]*$`); reject anything containing `/`, `\`, `..`, whitespace, or uppercase. Reject with a clear `WardError::NotFound`.
- `skillCreatable`: Claude `true`, Codex `false` (this plan). Plan 20 flips Codex to `true` and adds the Codex `skill-create` restore arm.
- TDD: failing test → implement → green → commit. One conventional commit per task. **Commit `Cargo.lock` only if a dependency changed (none here).** `cargo test` (from `src-tauri/`), `npm test`, `npx tsc --noEmit`, `npm run build` all green before moving on.

---

### Task 1: Rust `skill_upsert` core + `skill-create` restore arm

**Files:**
- Modify: `src-tauri/src/harness/adapters/claude_ops.rs` (add `validate_skill_name`, `skill_upsert`, and a `"skill-create"` arm in `ClaudeOps::restore`)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `resolve_skill_dir(scope_id, scopes)` (existing pub), `ensure_under_home`, `RestoreInfo`, `Scope`.
- Produces:
  - `pub fn validate_skill_name(name: &str) -> Result<(), WardError>`
  - `pub fn skill_upsert(home: &Path, harness: &str, scope_id: &str, name: &str, content: &str, scopes: &[Scope]) -> Result<RestoreInfo, WardError>` → writes `<skills_dir>/<name>/SKILL.md`, create-only, returns `RestoreInfo { kind: "skill-create", original_path: <skill_dir>, ... }`.
  - A `"skill-create"` arm in `ClaudeOps::restore` that removes the created skill dir.

- [ ] **Step 1: Write the failing tests** (append to `mod tests`)

```rust
    #[test]
    fn validate_skill_name_accepts_kebab() {
        assert!(validate_skill_name("my-skill").is_ok());
        assert!(validate_skill_name("skill1").is_ok());
    }

    #[test]
    fn validate_skill_name_rejects_bad() {
        for bad in ["", "Foo", "a/b", "../evil", "a b", "a.b", "-lead", "UPPER"] {
            assert!(validate_skill_name(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn skill_upsert_creates_skill_md_in_claude_global() {
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        let scopes = scopes_for(home, &home.join("work/project-a"));
        let info = skill_upsert(home, "claude", "global", "new-skill",
            "---\nname: new-skill\n---\nbody", &scopes).unwrap();
        let target = home.join(".claude/skills/new-skill/SKILL.md");
        assert!(target.is_file());
        assert_eq!(fs::read_to_string(&target).unwrap(), "---\nname: new-skill\n---\nbody");
        assert_eq!(info.kind, "skill-create");
        assert_eq!(info.original_path, home.join(".claude/skills/new-skill").display().to_string());
        assert!(info.backup_bytes.is_none(), "fresh create → no backup");
    }

    #[test]
    fn skill_upsert_refuses_to_clobber_existing() {
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        let scopes = scopes_for(home, &home.join("work/project-a"));
        let existing = home.join(".claude/skills/dup/SKILL.md");
        fs::create_dir_all(existing.parent().unwrap()).unwrap();
        fs::write(&existing, "old").unwrap();
        let res = skill_upsert(home, "claude", "global", "dup", "new", &scopes);
        assert!(res.is_err(), "must refuse to overwrite an existing skill dir");
        assert_eq!(fs::read_to_string(&existing).unwrap(), "old", "existing content untouched");
    }

    #[test]
    fn skill_upsert_rejects_invalid_name_before_write() {
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        let scopes = scopes_for(home, &home.join("work/project-a"));
        assert!(skill_upsert(home, "claude", "global", "../evil", "x", &scopes).is_err());
        assert!(!home.join(".claude/skills").exists(), "no dir created on invalid name");
    }

    #[test]
    fn skill_create_undo_removes_the_created_dir() {
        let (dir, _repo) = make_home_with_repo();
        let home = dir.path();
        let scopes = scopes_for(home, &home.join("work/project-a"));
        let ops = ClaudeOps;
        let info = skill_upsert(home, "claude", "global", "temp", "body", &scopes).unwrap();
        let skill_dir = home.join(".claude/skills/temp");
        assert!(skill_dir.is_dir());
        ops.restore(&ctx_for(home), &info).unwrap();
        assert!(!skill_dir.exists(), "undo removes the created skill dir");
    }
```

- [ ] **Step 2: Run tests, verify they fail**

Run (from `src-tauri/`): `cargo test -p ward skill_upsert 2>&1 | tail -20` and `cargo test -p ward validate_skill_name 2>&1 | tail -10`
Expected: FAIL — `cannot find function skill_upsert` / `validate_skill_name`.

- [ ] **Step 3: Implement**

Add near the other helpers in `claude_ops.rs`:

```rust
/// Validate a skill directory name: kebab-case, no path separators / traversal.
pub fn validate_skill_name(name: &str) -> Result<(), WardError> {
    let ok = !name.is_empty()
        && name.bytes().enumerate().all(|(i, b)| {
            let c = b as char;
            if i == 0 { c.is_ascii_lowercase() || c.is_ascii_digit() }
            else { c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' }
        });
    if !ok {
        return Err(WardError::NotFound(format!(
            "invalid skill name '{name}' (use lowercase letters, digits, hyphens)"
        )));
    }
    Ok(())
}

/// Resolve the skills directory for `(harness, scope_id)`.
fn resolve_skills_dir_for(home: &Path, harness: &str, scope_id: &str, scopes: &[Scope])
    -> Option<PathBuf>
{
    match harness {
        "claude" => resolve_skill_dir(scope_id, scopes),
        "codex" => {
            if scope_id == "global" {
                Some(home.join(".codex").join("skills"))
            } else {
                let scope = scopes.iter().find(|s| s.id == scope_id)?;
                if scope.kind != "project" { return None; }
                Some(PathBuf::from(&scope.root).join(".codex").join("skills"))
            }
        }
        _ => None,
    }
}

/// Create a new skill: write `<skills_dir>/<name>/SKILL.md`. Create-only —
/// errors if the skill dir already exists. Returns a `skill-create`
/// RestoreInfo whose undo removes the created dir.
pub fn skill_upsert(home: &Path, harness: &str, scope_id: &str, name: &str,
                    content: &str, scopes: &[Scope]) -> Result<RestoreInfo, WardError> {
    validate_skill_name(name)?;
    let dir = resolve_skills_dir_for(home, harness, scope_id, scopes)
        .ok_or_else(|| WardError::NotFound(format!("Cannot resolve skills dir for {harness}/{scope_id}")))?;
    let skill_dir = dir.join(name);
    let skill_dir = ensure_under_home(&skill_dir, home)?;
    if skill_dir.exists() {
        return Err(WardError::NotFound(format!("Skill '{name}' already exists")));
    }
    let target = skill_dir.join("SKILL.md");
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(&target, content)?;
    Ok(RestoreInfo {
        kind: "skill-create".into(),
        original_path: skill_dir.display().to_string(),
        current_path: None,
        backup_bytes: None,
        mcp_entry: None,
        mcp_key: None,
        mcp_parent_key: None,
        mcp_scope: None,
    })
}
```

Add the restore arm in `ClaudeOps::restore` (the `match info.kind.as_str()` block):

```rust
            "skill-create" => {
                let dir = ensure_under_home(Path::new(&info.original_path), ctx.home)?;
                if dir.exists() { std::fs::remove_dir_all(&dir)?; }
                Ok(())
            }
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p ward skill 2>&1 | tail -20` → PASS. Then `cargo test -p ward 2>&1 | tail -5` — full suite green.

- [ ] **Step 5: Commit**

```bash
git -C /Users/balakumar/personal/ward add src-tauri/src/harness/adapters/claude_ops.rs
git -C /Users/balakumar/personal/ward commit -m "feat(skill): skill_upsert create + name validation + skill-create undo"
```

---

### Task 2: `skillCreatable` capability plumbing

**Files:**
- Modify: `src-tauri/src/model.rs` (`Capabilities` struct + its unit test if present)
- Modify: `src-tauri/src/harness/adapters/claude.rs` (capabilities: `skill_creatable: true`)
- Modify: `src-tauri/src/harness/adapters/codex.rs` (capabilities: `skill_creatable: false`)
- Modify: `src-tauri/src/harness/mod.rs` (test `Fake` capabilities: `skill_creatable: false`)
- Modify: `src/api.ts` (`Capabilities` interface: `skillCreatable: boolean`)
- Modify: `src/mock/fixtures/scan-claude.json` (`capabilities.skillCreatable: true`) and `src/mock/fixtures.ts` (Codex scan capabilities: `skillCreatable: false`)
- Modify: `src/modes/Budget.test.tsx` and any other TS `Capabilities` literal (add `skillCreatable`) so `tsc` stays green
- Test: capability assertions in `claude.rs` / `codex.rs` tests

**Interfaces:**
- Produces: `Capabilities.skill_creatable: bool` (serializes `skillCreatable`). Claude `true`, Codex `false`, Fake `false`.

- [ ] **Step 1: Write/adjust the failing tests**

In `claude.rs` capabilities test add `assert!(c.skill_creatable);`. In `codex.rs` capabilities test add `assert!(!c.skill_creatable);`. (Locate the existing `capabilities()` test asserting other fields; extend it.)

- [ ] **Step 2: Run, verify fail**

Run: `cargo test -p ward capabilit 2>&1 | tail -20` → FAIL (no field `skill_creatable`).

- [ ] **Step 3: Implement**

Add `pub skill_creatable: bool,` to `Capabilities` in `model.rs`. Grep every `Capabilities {` construction site (`grep -rn 'Capabilities {' src-tauri/src`) and add the field: `claude.rs` → `skill_creatable: true`; `codex.rs` → `skill_creatable: false`; `mod.rs` Fake → `skill_creatable: false`; any model.rs test literal → a value.
Add `skillCreatable: boolean;` to the `Capabilities` interface in `src/api.ts`. Add `"skillCreatable": true` to `src/mock/fixtures/scan-claude.json`'s `capabilities` object; `skillCreatable: false` to the Codex fixture's capabilities in `src/mock/fixtures.ts`. Add `skillCreatable: true` to the `Capabilities` literal in `src/modes/Budget.test.tsx` (and any other TS literal grep finds: `grep -rn 'mcpEditable' src` locates the Capabilities literals touched in Plan 18).

- [ ] **Step 4: Run tests + typecheck**

Run: `cargo test -p ward 2>&1 | tail -5` (all pass), `npm test 2>&1 | tail -6` (all pass), `npx tsc --noEmit 2>&1 | tail -5` (clean).

- [ ] **Step 5: Commit**

```bash
git -C /Users/balakumar/personal/ward add src-tauri/src/model.rs src-tauri/src/harness/adapters/claude.rs src-tauri/src/harness/adapters/codex.rs src-tauri/src/harness/mod.rs src/api.ts src/mock/fixtures/scan-claude.json src/mock/fixtures.ts src/modes/Budget.test.tsx
git -C /Users/balakumar/personal/ward commit -m "feat(skill): skillCreatable capability (claude true, codex false)"
```

---

### Task 3: `skill_upsert` command + api + mock wiring

**Files:**
- Modify: `src-tauri/src/commands.rs` (`skill_upsert` command)
- Modify: `src-tauri/src/lib.rs` (register in `generate_handler!`)
- Modify: `src/api.ts` (`skillUpsert` wrapper + `'skill-create'` on `RestoreInfo.kind` union)
- Modify: `src/mock/dispatch.ts` (`case 'skill_upsert'`) + `src/mock/store.ts` (`skillUpsert` method)
- Test: `src/mock/store.test.ts`, `src/api.test.ts`

**Interfaces:**
- Consumes: `skill_upsert` core fn (Task 1), `harness_ctx`.
- Produces: `#[tauri::command] pub fn skill_upsert(harness, scope_id, name, content) -> Result<RestoreInfo, WardError>`; `api.skillUpsert(harness, scopeId, name, content)`; `MockStore.skillUpsert(harness, scopeId, name, content)` inserting a new `skill` item and returning a `MockRestore { kind: 'skill-create', __undoId }` whose undo splices it out.

- [ ] **Step 1: Write the failing tests**

`src/api.test.ts` (reuse the file's real invoke-spy pattern):
```ts
  test('skillUpsert invokes skill_upsert with camelCase args', async () => {
    invoke.mockResolvedValue({ kind: 'skill-create', originalPath: '/x' });
    await api.skillUpsert('claude', 'global', 'my-skill', '---\nname: my-skill\n---\n');
    expect(invoke).toHaveBeenCalledWith('skill_upsert',
      { harness: 'claude', scopeId: 'global', name: 'my-skill', content: '---\nname: my-skill\n---\n' });
  });
```

`src/mock/store.test.ts`:
```ts
  it('skillUpsert adds a new skill item and undo removes it', () => {
    const s = new MockStore();
    const n0 = s.scan('claude').items.filter((i) => i.category === 'skill').length;
    const r = s.skillUpsert('claude', 'global', 'brand-skill', '---\nname: brand-skill\n---\n');
    expect(r.kind).toBe('skill-create');
    expect(s.scan('claude').items.filter((i) => i.category === 'skill').length).toBe(n0 + 1);
    s.restore({ ...r });
    expect(s.scan('claude').items.filter((i) => i.category === 'skill').length).toBe(n0);
  });
```

- [ ] **Step 2: Run, verify fail**

Run: `npm test -- api.test store.test 2>&1 | tail -15` → FAIL (`api.skillUpsert`/`s.skillUpsert` not a function).

- [ ] **Step 3: Implement**

`commands.rs` (after the MCP commands):
```rust
/// Create a new skill (scaffold `<skills_dir>/<name>/SKILL.md`). Create-only.
#[tauri::command]
pub fn skill_upsert(harness: String, scope_id: String, name: String, content: String)
    -> Result<RestoreInfo, WardError>
{
    let (ctx, scopes) = harness_ctx(&harness)?;
    crate::harness::adapters::claude_ops::skill_upsert(ctx.home, &harness, &scope_id, &name, &content, &scopes)
}
```
Register `commands::skill_upsert,` in `lib.rs` `generate_handler!`.

`src/api.ts`: add `'skill-create'` to the `RestoreInfo.kind` union; add wrapper:
```ts
  skillUpsert: (harness: string, scopeId: string, name: string, content: string) =>
    invokeOrThrow<RestoreInfo>('skill_upsert', { harness, scopeId, name, content }),
```

`src/mock/store.ts` — add (mirror `upsertMcpEntry`'s add branch):
```ts
  skillUpsert(harness: string, scopeId: string, name: string, _content: string): MockRestore {
    const s = this.scanFor(harness);
    const newItem = { category: 'skill', scopeId, name,
      path: `${scopeId}/skills/${name}/SKILL.md`,
      movable: true, deletable: true, locked: false } as (typeof s.items)[number];
    s.items.push(newItem);
    const undoId = this.newUndo(() => { const j = s.items.indexOf(newItem); if (j >= 0) s.items.splice(j, 1); });
    return { kind: 'skill-create', originalPath: newItem.path, __undoId: undoId };
  }
```
`src/mock/dispatch.ts`:
```ts
    case 'skill_upsert':
      await delay(60);
      return store.skillUpsert(args.harness ?? 'claude', args.scopeId, args.name, args.content);
```

- [ ] **Step 4: Run tests + check**

Run: `npm test -- api.test store.test 2>&1 | tail -15` → PASS. `cargo test -p ward 2>&1 | tail -5` (all pass), `cargo check 2>&1 | tail -3` (clean), `npx tsc --noEmit 2>&1 | tail -3` (clean).

- [ ] **Step 5: Commit**

```bash
git -C /Users/balakumar/personal/ward add src-tauri/src/commands.rs src-tauri/src/lib.rs src/api.ts src/mock/dispatch.ts src/mock/store.ts src/api.test.ts src/mock/store.test.ts
git -C /Users/balakumar/personal/ward commit -m "feat(skill): skill_upsert command + api.skillUpsert + mock"
```

---

### Task 4: Organizer "Add Skill" flow + App bridge

**Files:**
- Modify: `src/modes/Organizer.tsx` (`+ Add Skill` control gated on `skillCreatable` + skill category; name+scope dialog; scaffold → create → select new item)
- Modify: `src/App.tsx` (`organizerApi.skillUpsert` bridge)
- Modify: `src/styles/organizer.css` (dialog styles)
- Test: `src/modes/Organizer.test.tsx`

**Interfaces:**
- Consumes: `props.api.skillUpsert(scopeId, name, content) => Promise<RestoreInfo>` and `props.scan.capabilities.skillCreatable`.
- Produces: `OrganizerApi.skillUpsert(scopeId: string, name: string, content: string) => Promise<RestoreInfo>`; `+ Add Skill` UI (`data-testid="skill-add-button"`) → dialog (`skill-add-name`, `skill-add-scope`, `skill-add-create`).

- [ ] **Step 1: Write the failing test** (append to `Organizer.test.tsx`, reusing its render harness)

```tsx
  it('creates a new skill via Add Skill (scaffold content sent to skillUpsert)', async () => {
    const skillSpy = vi.fn().mockResolvedValue({ kind: 'skill-create', originalPath: '/x' });
    const scan = makeScan({ capabilities: { skillCreatable: true }, items: [
      { category: 'skill', scopeId: 'global', name: 'existing', path: '/Users/x/.claude/skills/existing/SKILL.md', movable: true, deletable: true, locked: false },
    ]});
    renderOrganizer({ scan, api: { ...fakeApi, skillUpsert: skillSpy } });
    fireEvent.click(screen.getByTestId('category-skill'));
    fireEvent.click(screen.getByTestId('skill-add-button'));
    fireEvent.input(screen.getByTestId('skill-add-name'), { target: { value: 'fresh-skill' } });
    fireEvent.click(screen.getByTestId('skill-add-create'));
    await waitFor(() => expect(skillSpy).toHaveBeenCalled());
    const [scopeId, name, content] = skillSpy.mock.calls[0];
    expect(scopeId).toBe('global');
    expect(name).toBe('fresh-skill');
    expect(content).toContain('name: fresh-skill'); // scaffold frontmatter
  });

  it('hides Add Skill when skillCreatable is false', () => {
    const scan = makeScan({ capabilities: { skillCreatable: false }, items: [
      { category: 'skill', scopeId: 'global', name: 'existing', path: '/Users/x/.codex/skills/existing/SKILL.md', movable: true, deletable: true, locked: false },
    ]});
    renderOrganizer({ scan, api: fakeApi });
    fireEvent.click(screen.getByTestId('category-skill'));
    expect(screen.queryByTestId('skill-add-button')).not.toBeInTheDocument();
  });
```

(Adjust `makeScan`/`makeScanWithMcp`/`renderOrganizer`/`fakeApi` to match the real helpers in the file; extend the fixture builder to accept a `capabilities` override + arbitrary `items` if it doesn't already. Add `skillUpsert` to `fakeApi`.)

- [ ] **Step 2: Run, verify fail**

Run: `npm test -- Organizer.test 2>&1 | tail -20` → FAIL (no `skill-add-button`).

- [ ] **Step 3: Implement**

- A `+ Add Skill` button, `data-testid="skill-add-button"`, in the items column when `activeCat() === 'skill' && props.scan.capabilities.skillCreatable`. Clicking sets an `addingSkill` signal.
- When `addingSkill()`, render (in the detail pane, gated first like the MCP add view) a small form: `skill-add-name` (text input), `skill-add-scope` (`<select>` from `props.scan.scopes`), and `skill-add-create` button. On create:
  ```ts
  const name = skillName().trim();
  if (!name) return;
  const content = `---\nname: ${name}\ndescription: TODO one-line description\n---\n\n# ${name}\n\n<!-- Describe when this skill applies and what it does. -->\n`;
  const info = await props.api.skillUpsert(chosenSkillScope(), name, content);
  setAddingSkill(false);
  setLastUndo(info);
  setStatusMsg(`Created skill ${name}`);
  // App's refetch surfaces the new item; optionally select it by key after refetch.
  ```
- `src/App.tsx` bridge:
  ```ts
  skillUpsert: async (scopeId, name, content) => {
    const r = await api.skillUpsert(harness(), scopeId, name, content);
    await refetch();
    return r;
  },
  ```
- Extend the `OrganizerApi` interface with `skillUpsert: (scopeId: string, name: string, content: string) => Promise<RestoreInfo>;`.
- Add dialog styles to `organizer.css` (class-based).

- [ ] **Step 4: Run tests + full green bar**

Run: `npm test -- Organizer.test 2>&1 | tail -20` → PASS. `npm test 2>&1 | tail -6` (all JS pass). `npx tsc --noEmit 2>&1 | tail -3` (clean). `npm run build 2>&1 | tail -6` (succeeds). `cd src-tauri && cargo test 2>&1 | tail -3` (all Rust pass).

- [ ] **Step 5: Commit**

```bash
git -C /Users/balakumar/personal/ward add src/modes/Organizer.tsx src/App.tsx src/styles/organizer.css src/modes/Organizer.test.tsx
git -C /Users/balakumar/personal/ward commit -m "feat(skill): Add Skill flow in Organizer (gated on skillCreatable)"
```

---

## Self-Review

- **Spec coverage:** §7 (editable/creatable skills) → Tasks 1–4. `skill_upsert` command + `api.skillUpsert` + mock (§10) → Tasks 1/3. Capability gate (mirrors Plan 18 `mcpEditable`) → Task 2.
- **Type consistency:** `skill_upsert(home, harness, scope_id, name, content, scopes)` (Task 1) ← command `skill_upsert(harness, scope_id, name, content)` (Task 3) ← `api.skillUpsert(harness, scopeId, name, content)` (Task 3) ← bridge `skillUpsert(scopeId, name, content)` (Task 4, harness injected by App). `RestoreInfo.kind` gains `'skill-create'` in Rust (Task 1) + TS (Task 3). `skill_creatable`/`skillCreatable` defined Task 2, consumed Task 4.
- **Undo:** create returns `RestoreInfo { kind: 'skill-create' }`; ClaudeOps::restore removes the created dir (Task 1 Step 1 test). Codex `skill-create` restore is deferred to Plan 20 (gated off via `skillCreatable: false`).
- **No placeholders:** every step ships real code.
```
