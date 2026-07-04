# Ward Plan 01 — Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ward launches as a native macOS window, scans `~/.claude`, and browses your Claude **skills** and **memories** in the Organizer's 3-column layout with a working detail pane — proving the full Rust-core → `invoke` → SolidJS stack end-to-end.

**Architecture:** Tauri 2.0 app. A Rust core exposes typed `#[tauri::command]`s (`scan`, `read_file_content`) over the `invoke` bridge; a SolidJS + TypeScript frontend renders the result. All config logic lives behind a `Harness` trait so more harnesses/categories drop in later. The UI never touches the filesystem — it only calls commands.

**Tech Stack:** Tauri 2.0, Rust (serde, thiserror, dirs), SolidJS + TypeScript + Vite, Vitest + @solidjs/testing-library.

## Global Constraints

- **Framework:** Tauri **2.0** (not v1 — `invoke` imports from `@tauri-apps/api/core`; command registration in `src-tauri/src/lib.rs`).
- **Frontend:** SolidJS + TypeScript + Vite.
- **Platform:** macOS-first (Apple Silicon + Intel). Only the macOS build is tested.
- **Single seam:** the frontend reaches the core **only** via `invoke`; the UI never does filesystem I/O directly.
- **Every command returns `Result<T, WardError>`.**
- **Path safety:** all file access confined under `$HOME`; reject any `..` component; reject paths not under home.
- **Ward's own state dir is `~/.ward/`** (never write into `~/.claude`).
- **Zero telemetry.** No network calls in this plan.
- **No verbatim CCO code.** Independent implementation; `NOTICE` (already in repo) credits CCO (MIT © 2026 mcpware).
- **Normalized data model:** `ScanResult { harnessId, categories, scopes, items, capabilities }`; items are `HarnessItem`.
- **Scope of Plan 01:** Claude adapter, **global scope only**, categories **skill** and **memory**, read-only. Project scopes and the other 10 categories are Plan 02.

**Prerequisites (verify once before Task 1):**
- Rust stable (`rustc --version`), Node ≥ 20 (`node --version`), Xcode Command Line Tools (`xcode-select -p`).
- Repo already exists at `/Users/balakumar/personal/ward` with `docs/`, `README.md`, `NOTICE`, `.gitignore`, git initialized.

---

### Task 1: Scaffold Tauri 2 + SolidJS into the repo

**Files:**
- Create: `package.json`, `index.html`, `vite.config.ts`, `tsconfig.json`, `src/*` (Solid frontend), `src-tauri/*` (Cargo.toml, tauri.conf.json, build.rs, src/main.rs, src/lib.rs, capabilities/default.json, icons/)
- Modify: `.gitignore` (reconcile), `package.json` (name)

**Interfaces:**
- Consumes: nothing (first task).
- Produces: a runnable Tauri app. `npm run tauri dev` opens a window. Rust lib crate is named `ward_lib` with `pub fn run()` in `src-tauri/src/lib.rs`.

- [ ] **Step 1: Scaffold in a temp dir** (create-tauri-app refuses a non-empty target, so generate elsewhere then copy)

```bash
cd /tmp && rm -rf ward-scaffold
npm create tauri-app@latest ward-scaffold -- --template solid-ts --manager npm --yes
```
Expected: `/tmp/ward-scaffold/` created with `src/`, `src-tauri/`, `package.json`, `index.html`, `vite.config.ts`, `tsconfig.json`. (If `--yes`/`--template` are unsupported by your CLI version, run `npm create tauri-app@latest` interactively and choose: project name `ward-scaffold`, **Solid**, **TypeScript**, package manager **npm**.)

- [ ] **Step 2: Copy app files into the repo, preserving docs/git/README/NOTICE**

```bash
rsync -a --exclude '.git' --exclude 'README.md' --exclude '.gitignore' \
  /tmp/ward-scaffold/ /Users/balakumar/personal/ward/
```
Expected: `src/`, `src-tauri/`, `package.json`, etc. now present in the repo; `docs/`, `README.md`, `NOTICE` untouched.

- [ ] **Step 3: Set app name and identifier**

Edit `/Users/balakumar/personal/ward/package.json` — set `"name": "ward"`.
Edit `/Users/balakumar/personal/ward/src-tauri/tauri.conf.json` — set:
```json
{
  "productName": "Ward",
  "identifier": "dev.balakumar.ward",
  "app": { "windows": [{ "title": "Ward", "width": 1200, "height": 800, "minWidth": 900, "minHeight": 600 }] }
}
```
(Leave `build`, `bundle`, and the rest of the generated file as-is.)

- [ ] **Step 4: Install and run the dev build**

```bash
cd /Users/balakumar/personal/ward
npm install
npm run tauri dev
```
Expected: a native window titled **Ward** opens showing the default Solid template. Close it (Cmd-Q) to continue.

- [ ] **Step 5: Commit**

```bash
cd /Users/balakumar/personal/ward
git add -A
git commit -m "chore: scaffold Tauri 2 + SolidJS app"
```

---

### Task 2: Core data model (Rust)

**Files:**
- Create: `src-tauri/src/model.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod model;`)
- Test: inline `#[cfg(test)]` in `model.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `Capabilities`, `Category`, `Scope`, `HarnessItem`, `ScanResult` — all `#[serde(rename_all = "camelCase")]`, `Debug + Clone + Serialize + Deserialize + PartialEq`. Field names below are the contract every later task and the TS layer depend on.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/model.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub context_budget: bool,
    pub mcp_controls: bool,
    pub mcp_policy: bool,
    pub mcp_security: bool,
    pub sessions: bool,
    pub effective: bool,
    pub backup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessItem {
    pub category: String,
    pub scope_id: String,
    pub name: String,
    pub path: String,
    pub movable: bool,
    pub deletable: bool,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub harness_id: String,
    pub categories: Vec<Category>,
    pub scopes: Vec<Scope>,
    pub items: Vec<HarnessItem>,
    pub capabilities: Capabilities,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_item_serializes_camel_case() {
        let item = HarnessItem {
            category: "skill".into(),
            scope_id: "global".into(),
            name: "brainstorming".into(),
            path: "/Users/x/.claude/skills/brainstorming".into(),
            movable: true,
            deletable: true,
            locked: false,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"scopeId\":\"global\""));
        assert!(json.contains("\"category\":\"skill\""));
    }
}
```

- [ ] **Step 2: Add deps and module, run test to verify it fails then builds**

Add to `src-tauri/Cargo.toml` under `[dependencies]` (serde is already there from the template; add serde_json):
```toml
serde_json = "1"
```
Add to the top of `src-tauri/src/lib.rs`: `mod model;`

Run: `cd src-tauri && cargo test model::tests::harness_item_serializes_camel_case`
Expected: compiles and **PASS** (the derives make it pass immediately; this test pins the camelCase contract).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/model.rs src-tauri/src/lib.rs src-tauri/Cargo.toml
git commit -m "feat: core data model (ScanResult, HarnessItem, Capabilities)"
```

---

### Task 3: Typed error (`WardError`)

**Files:**
- Create: `src-tauri/src/error.rs`
- Modify: `src-tauri/src/lib.rs` (`mod error;`), `src-tauri/Cargo.toml` (add `thiserror`)
- Test: inline in `error.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub enum WardError { NotFound(String), PathEscaped(String), HarnessUnavailable(String), Io(std::io::Error) }` implementing `std::error::Error` and `serde::Serialize` as `{ kind, message }`. All commands return `Result<_, WardError>`.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/error.rs`:
```rust
#[derive(Debug, thiserror::Error)]
pub enum WardError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("path escaped home: {0}")]
    PathEscaped(String),
    #[error("harness unavailable: {0}")]
    HarnessUnavailable(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "message")]
#[serde(rename_all = "camelCase")]
enum ErrorKind {
    NotFound(String),
    PathEscaped(String),
    HarnessUnavailable(String),
    Io(String),
}

impl serde::Serialize for WardError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        let message = self.to_string();
        let kind = match self {
            WardError::NotFound(_) => ErrorKind::NotFound(message),
            WardError::PathEscaped(_) => ErrorKind::PathEscaped(message),
            WardError::HarnessUnavailable(_) => ErrorKind::HarnessUnavailable(message),
            WardError::Io(_) => ErrorKind::Io(message),
        };
        kind.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_kind_and_message() {
        let e = WardError::HarnessUnavailable("codex".into());
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, "{\"kind\":\"harnessUnavailable\",\"message\":\"harness unavailable: codex\"}");
    }
}
```

- [ ] **Step 2: Add dep + module, run the test**

Add to `src-tauri/Cargo.toml` `[dependencies]`: `thiserror = "2"`
Add to `src-tauri/src/lib.rs`: `mod error;`

Run: `cd src-tauri && cargo test error::tests::serializes_kind_and_message`
Expected: **PASS** with the exact JSON asserted.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/error.rs src-tauri/src/lib.rs src-tauri/Cargo.toml
git commit -m "feat: typed WardError with structured serialization"
```

---

### Task 4: Filesystem safety (`fs_utils`)

**Files:**
- Create: `src-tauri/src/fs_utils.rs`
- Modify: `src-tauri/src/lib.rs` (`mod fs_utils;`)
- Test: inline in `fs_utils.rs`

**Interfaces:**
- Consumes: `WardError` (Task 3).
- Produces: `pub fn ensure_under_home(path: &Path, home: &Path) -> Result<PathBuf, WardError>` — rejects `..` components and paths not under `home`; returns the absolute path.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/fs_utils.rs`:
```rust
use std::path::{Component, Path, PathBuf};
use crate::error::WardError;

/// Confine `path` under `home`. Relative paths are joined onto `home`.
/// Any `..` component, or an absolute path not under `home`, is rejected.
pub fn ensure_under_home(path: &Path, home: &Path) -> Result<PathBuf, WardError> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(WardError::PathEscaped(path.display().to_string()));
    }
    let abs = if path.is_absolute() { path.to_path_buf() } else { home.join(path) };
    if !abs.starts_with(home) {
        return Err(WardError::PathEscaped(abs.display().to_string()));
    }
    Ok(abs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_path_under_home() {
        let home = Path::new("/Users/x");
        let p = Path::new("/Users/x/.claude/skills/a/SKILL.md");
        assert_eq!(ensure_under_home(p, home).unwrap(), p.to_path_buf());
    }

    #[test]
    fn rejects_parent_traversal() {
        let home = Path::new("/Users/x");
        let p = Path::new("/Users/x/../etc/passwd");
        assert!(matches!(ensure_under_home(p, home), Err(WardError::PathEscaped(_))));
    }

    #[test]
    fn rejects_outside_home() {
        let home = Path::new("/Users/x");
        let p = Path::new("/etc/passwd");
        assert!(matches!(ensure_under_home(p, home), Err(WardError::PathEscaped(_))));
    }
}
```

- [ ] **Step 2: Register module and run tests**

Add to `src-tauri/src/lib.rs`: `mod fs_utils;`
Run: `cd src-tauri && cargo test fs_utils::tests`
Expected: 3 tests **PASS**.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/fs_utils.rs src-tauri/src/lib.rs
git commit -m "feat: path-safety helper (home confinement, traversal rejection)"
```

---

### Task 5: Harness trait + registry

**Files:**
- Create: `src-tauri/src/harness/mod.rs`
- Modify: `src-tauri/src/lib.rs` (`mod harness;`)
- Test: inline in `harness/mod.rs`

**Interfaces:**
- Consumes: `Scope`, `Capabilities`, `HarnessItem` (Task 2), `WardError` (Task 3).
- Produces:
  - `pub struct Ctx<'a> { pub home: &'a Path, pub cwd: Option<&'a Path> }`
  - `pub trait Harness: Send + Sync { fn id(&self)->&str; fn display_name(&self)->&str; fn short_name(&self)->&str; fn icon(&self)->&str; fn executable(&self)->&str; fn category_ids(&self)->Vec<&'static str>; fn capabilities(&self)->Capabilities; fn discover_scopes(&self,&Ctx)->Result<Vec<Scope>,WardError>; fn scan_category(&self,&Ctx,category:&str,scope:&Scope)->Result<Vec<HarnessItem>,WardError>; }`
  - `pub struct Registry` with `new()`, `register(Box<dyn Harness>)`, `get(&str)->Option<&dyn Harness>`, `list()->Vec<&dyn Harness>`.

- [ ] **Step 1: Write the failing test** (uses a tiny fake adapter)

Create `src-tauri/src/harness/mod.rs`:
```rust
pub mod framework;
pub mod adapters;

use std::path::Path;
use crate::model::{Capabilities, HarnessItem, Scope};
use crate::error::WardError;

pub struct Ctx<'a> {
    pub home: &'a Path,
    pub cwd: Option<&'a Path>,
}

pub trait Harness: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn short_name(&self) -> &str;
    fn icon(&self) -> &str;
    fn executable(&self) -> &str;
    fn category_ids(&self) -> Vec<&'static str>;
    fn capabilities(&self) -> Capabilities;
    fn discover_scopes(&self, ctx: &Ctx) -> Result<Vec<Scope>, WardError>;
    fn scan_category(&self, ctx: &Ctx, category: &str, scope: &Scope)
        -> Result<Vec<HarnessItem>, WardError>;
}

pub struct Registry {
    adapters: Vec<Box<dyn Harness>>,
}

impl Registry {
    pub fn new() -> Self {
        Registry { adapters: Vec::new() }
    }
    pub fn register(&mut self, adapter: Box<dyn Harness>) {
        self.adapters.push(adapter);
    }
    pub fn get(&self, id: &str) -> Option<&dyn Harness> {
        self.adapters.iter().map(|b| b.as_ref()).find(|a| a.id() == id)
    }
    pub fn list(&self) -> Vec<&dyn Harness> {
        self.adapters.iter().map(|b| b.as_ref()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;
    impl Harness for Fake {
        fn id(&self) -> &str { "fake" }
        fn display_name(&self) -> &str { "Fake" }
        fn short_name(&self) -> &str { "Fk" }
        fn icon(&self) -> &str { "◆" }
        fn executable(&self) -> &str { "fake" }
        fn category_ids(&self) -> Vec<&'static str> { vec!["skill"] }
        fn capabilities(&self) -> Capabilities {
            Capabilities { context_budget: false, mcp_controls: false, mcp_policy: false,
                mcp_security: false, sessions: false, effective: false, backup: false }
        }
        fn discover_scopes(&self, _ctx: &Ctx) -> Result<Vec<Scope>, WardError> { Ok(vec![]) }
        fn scan_category(&self, _c: &Ctx, _cat: &str, _s: &Scope) -> Result<Vec<HarnessItem>, WardError> { Ok(vec![]) }
    }

    #[test]
    fn registry_registers_and_finds() {
        let mut r = Registry::new();
        r.register(Box::new(Fake));
        assert!(r.get("fake").is_some());
        assert!(r.get("nope").is_none());
        assert_eq!(r.list().len(), 1);
    }
}
```

- [ ] **Step 2: Create empty submodule stubs so it compiles**

Create `src-tauri/src/harness/framework.rs` with a placeholder that Task 8 fills:
```rust
// filled in Task 8
```
Create `src-tauri/src/harness/adapters/mod.rs`:
```rust
// adapters registered here; claude added in Task 6
```
Add to `src-tauri/src/lib.rs`: `mod harness;`

- [ ] **Step 3: Run the test**

Run: `cd src-tauri && cargo test harness::tests::registry_registers_and_finds`
Expected: **PASS**.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/harness src-tauri/src/lib.rs
git commit -m "feat: Harness trait + Registry"
```

---

### Task 6: Claude adapter — descriptor, capabilities, global scope

**Files:**
- Create: `src-tauri/src/harness/adapters/claude.rs`
- Modify: `src-tauri/src/harness/adapters/mod.rs`, `src-tauri/Cargo.toml` (add `dirs`)
- Test: inline in `claude.rs`

**Interfaces:**
- Consumes: `Harness`, `Ctx` (Task 5), `Scope`, `Capabilities`, `HarnessItem` (Task 2), `WardError` (Task 3).
- Produces: `pub struct ClaudeAdapter;` implementing `Harness` with `id()=="claude"`, all 7 capabilities `true`, `category_ids() == ["skill","memory"]` (extended in Plan 02), and `discover_scopes` returning a single **global** scope rooted at `~/.claude`.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/harness/adapters/claude.rs`:
```rust
use std::path::Path;
use crate::error::WardError;
use crate::harness::{Ctx, Harness};
use crate::model::{Capabilities, HarnessItem, Scope};

pub struct ClaudeAdapter;

impl ClaudeAdapter {
    fn claude_root(home: &Path) -> std::path::PathBuf {
        home.join(".claude")
    }
}

impl Harness for ClaudeAdapter {
    fn id(&self) -> &str { "claude" }
    fn display_name(&self) -> &str { "Claude Code" }
    fn short_name(&self) -> &str { "Claude" }
    fn icon(&self) -> &str { "◆" }
    fn executable(&self) -> &str { "claude" }

    fn category_ids(&self) -> Vec<&'static str> { vec!["skill", "memory"] }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            context_budget: true, mcp_controls: true, mcp_policy: true,
            mcp_security: true, sessions: true, effective: true, backup: true,
        }
    }

    fn discover_scopes(&self, ctx: &Ctx) -> Result<Vec<Scope>, WardError> {
        let root = Self::claude_root(ctx.home);
        Ok(vec![Scope {
            id: "global".into(),
            kind: "global".into(),
            label: "Global (~/.claude)".into(),
            root: root.display().to_string(),
        }])
    }

    fn scan_category(&self, _ctx: &Ctx, _category: &str, _scope: &Scope)
        -> Result<Vec<HarnessItem>, WardError> {
        Ok(vec![]) // filled in Task 7
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_global_scope() {
        let home = Path::new("/Users/x");
        let ctx = Ctx { home, cwd: None };
        let scopes = ClaudeAdapter.discover_scopes(&ctx).unwrap();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].id, "global");
        assert_eq!(scopes[0].root, "/Users/x/.claude");
    }

    #[test]
    fn advertises_all_capabilities() {
        let c = ClaudeAdapter.capabilities();
        assert!(c.effective && c.mcp_security && c.backup && c.context_budget);
    }
}
```

- [ ] **Step 2: Register the module + add `dirs`**

Add to `src-tauri/Cargo.toml` `[dependencies]`: `dirs = "5"`
Set `src-tauri/src/harness/adapters/mod.rs` to:
```rust
pub mod claude;
```

- [ ] **Step 3: Run the tests**

Run: `cd src-tauri && cargo test harness::adapters::claude::tests`
Expected: 2 tests **PASS**.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/harness/adapters src-tauri/Cargo.toml
git commit -m "feat: Claude adapter descriptor + global scope discovery"
```

---

### Task 7: Claude adapter — skill & memory scanners (read-only)

**Files:**
- Modify: `src-tauri/src/harness/adapters/claude.rs` (implement `scan_category`)
- Test: inline in `claude.rs` (temp-dir fixtures)

**Interfaces:**
- Consumes: `ClaudeAdapter` (Task 6), `HarnessItem` (Task 2).
- Produces: `scan_category(ctx, "skill", global)` → one item per `~/.claude/skills/<name>/SKILL.md`; `scan_category(ctx, "memory", global)` → items for `~/.claude/CLAUDE.md` (if present) + each `~/.claude/memory/*.md`. Items carry `movable:true, deletable:true, locked:false`.

- [ ] **Step 1: Write the failing test** (build a fake `~/.claude` in a temp dir)

Add `tempfile` to `src-tauri/Cargo.toml` under `[dev-dependencies]`:
```toml
tempfile = "3"
```
Append these tests to the `tests` module in `claude.rs`:
```rust
    use std::fs;

    fn make_home() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        fs::create_dir_all(claude.join("skills/brainstorming")).unwrap();
        fs::write(claude.join("skills/brainstorming/SKILL.md"), "---\nname: brainstorming\n---\n").unwrap();
        fs::create_dir_all(claude.join("skills/deep-research")).unwrap();
        fs::write(claude.join("skills/deep-research/SKILL.md"), "x").unwrap();
        fs::write(claude.join("CLAUDE.md"), "root memory").unwrap();
        fs::create_dir_all(claude.join("memory")).unwrap();
        fs::write(claude.join("memory/user.md"), "u").unwrap();
        dir
    }

    #[test]
    fn scans_skills() {
        let home = make_home();
        let ctx = Ctx { home: home.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let mut items = ClaudeAdapter.scan_category(&ctx, "skill", &scope).unwrap();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "brainstorming");
        assert_eq!(items[0].category, "skill");
        assert_eq!(items[0].scope_id, "global");
        assert!(items[0].path.ends_with("skills/brainstorming/SKILL.md"));
    }

    #[test]
    fn scans_memories_including_root_claude_md() {
        let home = make_home();
        let ctx = Ctx { home: home.path(), cwd: None };
        let scope = ClaudeAdapter.discover_scopes(&ctx).unwrap().remove(0);
        let items = ClaudeAdapter.scan_category(&ctx, "memory", &scope).unwrap();
        let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"CLAUDE.md"));
        assert!(names.contains(&"user.md"));
        assert_eq!(items.len(), 2);
    }
```

- [ ] **Step 2: Run the tests to verify they FAIL**

Run: `cd src-tauri && cargo test harness::adapters::claude::tests::scans_skills`
Expected: **FAIL** (`scan_category` returns `vec![]`, so `items.len()` is 0, not 2).

- [ ] **Step 3: Implement `scan_category`**

Replace the placeholder `scan_category` in `claude.rs` with:
```rust
    fn scan_category(&self, ctx: &Ctx, category: &str, scope: &Scope)
        -> Result<Vec<HarnessItem>, WardError> {
        let root = Self::claude_root(ctx.home);
        let mut items = Vec::new();
        match category {
            "skill" => {
                let skills = root.join("skills");
                if let Ok(entries) = std::fs::read_dir(&skills) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        let manifest = p.join("SKILL.md");
                        if manifest.is_file() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            items.push(HarnessItem {
                                category: "skill".into(),
                                scope_id: scope.id.clone(),
                                name,
                                path: manifest.display().to_string(),
                                movable: true, deletable: true, locked: false,
                            });
                        }
                    }
                }
            }
            "memory" => {
                let root_md = root.join("CLAUDE.md");
                if root_md.is_file() {
                    items.push(HarnessItem {
                        category: "memory".into(), scope_id: scope.id.clone(),
                        name: "CLAUDE.md".into(), path: root_md.display().to_string(),
                        movable: false, deletable: false, locked: true,
                    });
                }
                let mem = root.join("memory");
                if let Ok(entries) = std::fs::read_dir(&mem) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.extension().and_then(|e| e.to_str()) == Some("md") {
                            let name = p.file_name().unwrap().to_string_lossy().to_string();
                            items.push(HarnessItem {
                                category: "memory".into(), scope_id: scope.id.clone(),
                                name, path: p.display().to_string(),
                                movable: true, deletable: true, locked: false,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(items)
    }
```
(Note: `CLAUDE.md` is marked `locked` — the root memory is not freely movable/deletable. Adjust the earlier `scans_memories` test if you change this; it only checks names + count.)

- [ ] **Step 4: Run the tests to verify they PASS**

Run: `cd src-tauri && cargo test harness::adapters::claude::tests`
Expected: all 4 tests **PASS**.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/harness/adapters/claude.rs src-tauri/Cargo.toml
git commit -m "feat: Claude skill + memory scanners (read-only, global scope)"
```

---

### Task 8: Scan framework — assemble `ScanResult`

**Files:**
- Modify: `src-tauri/src/harness/framework.rs`
- Test: inline in `framework.rs`

**Interfaces:**
- Consumes: `Harness`, `Ctx` (Task 5), `ScanResult`, `Category`, `HarnessItem` (Task 2).
- Produces:
  - `pub fn category_label(id: &str) -> String`
  - `pub fn run_scan(adapter: &dyn Harness, ctx: &Ctx) -> Result<ScanResult, WardError>` — runs every category scanner across every discovered scope, assembles items, computes per-category counts.

- [ ] **Step 1: Write the failing test**

Set `src-tauri/src/harness/framework.rs` to:
```rust
use crate::error::WardError;
use crate::harness::{Ctx, Harness};
use crate::model::{Category, HarnessItem, ScanResult};

pub fn category_label(id: &str) -> String {
    match id {
        "skill" => "Skills",
        "memory" => "Memories",
        "mcp" => "MCP",
        "command" => "Commands",
        "agent" => "Agents",
        "hook" => "Hooks",
        "plan" => "Plans",
        "rule" => "Rules",
        "config" => "Config",
        "plugin" => "Plugins",
        "session" => "Sessions",
        "setting" => "Settings",
        other => other,
    }
    .to_string()
}

pub fn run_scan(adapter: &dyn Harness, ctx: &Ctx) -> Result<ScanResult, WardError> {
    let scopes = adapter.discover_scopes(ctx)?;
    let mut items: Vec<HarnessItem> = Vec::new();
    for scope in &scopes {
        for cat in adapter.category_ids() {
            items.extend(adapter.scan_category(ctx, cat, scope)?);
        }
    }
    let categories = adapter
        .category_ids()
        .iter()
        .map(|id| Category {
            id: (*id).to_string(),
            label: category_label(id),
            count: items.iter().filter(|i| i.category == *id).count(),
        })
        .collect();
    Ok(ScanResult {
        harness_id: adapter.id().to_string(),
        categories,
        scopes,
        items,
        capabilities: adapter.capabilities(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::adapters::claude::ClaudeAdapter;
    use std::fs;

    #[test]
    fn run_scan_counts_items_per_category() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join(".claude");
        fs::create_dir_all(claude.join("skills/a")).unwrap();
        fs::write(claude.join("skills/a/SKILL.md"), "x").unwrap();
        fs::write(claude.join("CLAUDE.md"), "m").unwrap();

        let ctx = Ctx { home: dir.path(), cwd: None };
        let result = run_scan(&ClaudeAdapter, &ctx).unwrap();

        assert_eq!(result.harness_id, "claude");
        let skill_cat = result.categories.iter().find(|c| c.id == "skill").unwrap();
        assert_eq!(skill_cat.count, 1);
        assert_eq!(skill_cat.label, "Skills");
        assert_eq!(result.items.iter().filter(|i| i.category == "skill").count(), 1);
    }
}
```

- [ ] **Step 2: Run the test**

Run: `cd src-tauri && cargo test harness::framework::tests::run_scan_counts_items_per_category`
Expected: **PASS**.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/harness/framework.rs
git commit -m "feat: scan framework assembles normalized ScanResult"
```

---

### Task 9: `scan` command + registry builder

**Files:**
- Create: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs` (`mod commands;`, `build_registry`, register handler)
- Test: inline in `commands.rs`

**Interfaces:**
- Consumes: `Registry` (Task 5), `run_scan` (Task 8), `ClaudeAdapter` (Task 6), `ScanResult` (Task 2), `WardError` (Task 3).
- Produces:
  - `pub fn build_registry() -> Registry` (registers `ClaudeAdapter`)
  - `pub fn scan_impl(registry: &Registry, home: &Path, harness_id: &str) -> Result<ScanResult, WardError>`
  - `#[tauri::command] pub fn scan(harness: String) -> Result<ScanResult, WardError>`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/commands.rs`:
```rust
use std::path::Path;
use crate::error::WardError;
use crate::harness::adapters::claude::ClaudeAdapter;
use crate::harness::{framework, Ctx, Registry};
use crate::model::ScanResult;

pub fn build_registry() -> Registry {
    let mut r = Registry::new();
    r.register(Box::new(ClaudeAdapter));
    r
}

pub fn scan_impl(registry: &Registry, home: &Path, harness_id: &str) -> Result<ScanResult, WardError> {
    let adapter = registry
        .get(harness_id)
        .ok_or_else(|| WardError::HarnessUnavailable(harness_id.to_string()))?;
    let ctx = Ctx { home, cwd: None };
    framework::run_scan(adapter, &ctx)
}

#[tauri::command]
pub fn scan(harness: String) -> Result<ScanResult, WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    let registry = build_registry();
    scan_impl(&registry, &home, &harness)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scan_impl_returns_claude_result() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".claude/skills/a")).unwrap();
        fs::write(dir.path().join(".claude/skills/a/SKILL.md"), "x").unwrap();

        let registry = build_registry();
        let result = scan_impl(&registry, dir.path(), "claude").unwrap();
        assert_eq!(result.harness_id, "claude");
        assert_eq!(result.items.iter().filter(|i| i.category == "skill").count(), 1);
    }

    #[test]
    fn scan_impl_unknown_harness_errors() {
        let registry = build_registry();
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            scan_impl(&registry, dir.path(), "nope"),
            Err(WardError::HarnessUnavailable(_))
        ));
    }
}
```

- [ ] **Step 2: Wire into `lib.rs`**

Edit `src-tauri/src/lib.rs` — ensure the module list and builder look like this (keep the template's `run` shape, add `commands` + handler):
```rust
mod model;
mod error;
mod fs_utils;
mod harness;
mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::scan,
            commands::read_file_content
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```
(`read_file_content` is added in Task 10 — if you run the app between tasks, temporarily drop it from the handler list.)

- [ ] **Step 3: Run the tests**

Run: `cd src-tauri && cargo test commands::tests`
Expected: 2 tests **PASS**.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: scan command + registry builder"
```

---

### Task 10: `read_file_content` command

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Test: inline in `commands.rs`

**Interfaces:**
- Consumes: `ensure_under_home` (Task 4), `WardError` (Task 3).
- Produces:
  - `pub fn read_file_impl(path: &Path, home: &Path) -> Result<String, WardError>`
  - `#[tauri::command] pub fn read_file_content(path: String) -> Result<String, WardError>`

- [ ] **Step 1: Write the failing test**

Append to `commands.rs` (above the `#[cfg(test)]` block for the impl fns; add tests inside the existing `tests` module):
```rust
pub fn read_file_impl(path: &Path, home: &Path) -> Result<String, WardError> {
    let abs = crate::fs_utils::ensure_under_home(path, home)?;
    Ok(std::fs::read_to_string(abs)?)
}

#[tauri::command]
pub fn read_file_content(path: String) -> Result<String, WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    read_file_impl(Path::new(&path), &home)
}
```
Add these tests to the `tests` module:
```rust
    #[test]
    fn reads_allowed_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join(".claude/x.md");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, "hello").unwrap();
        assert_eq!(read_file_impl(&f, dir.path()).unwrap(), "hello");
    }

    #[test]
    fn rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("../etc/passwd");
        assert!(matches!(read_file_impl(&bad, dir.path()), Err(WardError::PathEscaped(_))));
    }
```

- [ ] **Step 2: Run the tests**

Run: `cd src-tauri && cargo test commands::tests`
Expected: 4 tests **PASS** (2 from Task 9 + 2 new).

- [ ] **Step 3: Confirm the full Rust suite is green**

Run: `cd src-tauri && cargo test`
Expected: all tests across `model`, `error`, `fs_utils`, `harness`, `commands` **PASS**.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat: read_file_content command with path safety"
```

---

### Task 11: Frontend — design tokens + app shell (C layout)

**Files:**
- Create: `src/styles/tokens.css`, `src/components/Sidebar.tsx`, `src/components/Shell.tsx`
- Modify: `src/App.tsx`, `src/index.tsx` (import tokens), `vite.config.ts` (add Vitest), `package.json` (test deps + script)
- Test: `src/components/Sidebar.test.tsx`

**Interfaces:**
- Consumes: nothing (static shell).
- Produces: `Shell` component rendering a `Sidebar` (5 modes: Organizer, Security, Context Budget, Sessions, Backups) + a main slot. `MODES: {id:string,label:string,icon:string}[]` exported from `Sidebar.tsx`.

- [ ] **Step 1: Add Vitest tooling**

```bash
cd /Users/balakumar/personal/ward
npm install -D vitest jsdom @solidjs/testing-library @testing-library/jest-dom vite-plugin-solid
```
Add to `package.json` `"scripts"`: `"test": "vitest run"`.
Ensure `vite.config.ts` includes the Solid plugin and a test block:
```ts
import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';

export default defineConfig({
  plugins: [solid()],
  // Tauri expects a fixed port, fail if unavailable
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test-setup.ts'],
  },
});
```
Create `src/test-setup.ts`:
```ts
import '@testing-library/jest-dom';
```

- [ ] **Step 2: Write the design tokens (Security Console)**

Create `src/styles/tokens.css`:
```css
:root {
  --bg: #0e1420;
  --surface: #131c2b;
  --surface-2: #0c121d;
  --border: rgba(120, 160, 220, 0.14);
  --text: #dfe7f2;
  --text-dim: rgba(223, 231, 242, 0.6);
  --accent: #8ff0a8;
  --crit: #ff453a;
  --warn: #ff9f0a;
  --ok: #30d158;
  --font-ui: -apple-system, "SF Pro Text", system-ui, sans-serif;
  --font-mono: ui-monospace, "SF Mono", Menlo, monospace;
  --radius: 8px;
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--text); font-family: var(--font-ui); }
```
Import it at the top of `src/index.tsx`: `import './styles/tokens.css';`

- [ ] **Step 3: Write the failing test**

Create `src/components/Sidebar.test.tsx`:
```tsx
import { render } from '@solidjs/testing-library';
import { Sidebar, MODES } from './Sidebar';

test('renders all five modes', () => {
  const { getByText } = render(() => <Sidebar active="organizer" onSelect={() => {}} />);
  for (const m of MODES) getByText(m.label);
  expect(MODES.map((m) => m.id)).toEqual(['organizer', 'security', 'budget', 'sessions', 'backups']);
});
```

- [ ] **Step 4: Run it to verify it fails**

Run: `npm test -- Sidebar`
Expected: **FAIL** (`Sidebar` not found).

- [ ] **Step 5: Implement `Sidebar` and `Shell`**

Create `src/components/Sidebar.tsx`:
```tsx
import { For } from 'solid-js';

export const MODES = [
  { id: 'organizer', label: 'Organizer', icon: '⌘' },
  { id: 'security', label: 'Security', icon: '⛨' },
  { id: 'budget', label: 'Context Budget', icon: '▣' },
  { id: 'sessions', label: 'Sessions', icon: '⧉' },
  { id: 'backups', label: 'Backups', icon: '↺' },
] as const;

export function Sidebar(props: { active: string; onSelect: (id: string) => void }) {
  return (
    <nav style={{ width: '210px', background: 'var(--surface-2)', 'border-right': '1px solid var(--border)', padding: '10px 8px' }}>
      <div style={{ 'font-size': '11px', color: 'var(--text-dim)', margin: '0 6px 8px' }}>◆ Claude Code</div>
      <For each={MODES}>
        {(m) => (
          <div
            onClick={() => props.onSelect(m.id)}
            style={{
              padding: '5px 8px', margin: '3px 0', 'border-radius': 'var(--radius)', cursor: 'pointer',
              background: props.active === m.id ? 'rgba(48,209,88,0.14)' : 'transparent',
              color: props.active === m.id ? 'var(--accent)' : 'var(--text)',
            }}
          >
            {m.icon} {m.label}
          </div>
        )}
      </For>
    </nav>
  );
}
```
Create `src/components/Shell.tsx`:
```tsx
import { JSX } from 'solid-js';
import { Sidebar } from './Sidebar';

export function Shell(props: { active: string; onSelect: (id: string) => void; children: JSX.Element }) {
  return (
    <div style={{ display: 'flex', height: '100vh' }}>
      <Sidebar active={props.active} onSelect={props.onSelect} />
      <main style={{ flex: 1, overflow: 'auto' }}>{props.children}</main>
    </div>
  );
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `npm test -- Sidebar`
Expected: **PASS**.

- [ ] **Step 7: Commit**

```bash
git add src package.json vite.config.ts package-lock.json
git commit -m "feat: Security Console design tokens + app shell (sidebar modes)"
```

---

### Task 12: Frontend — typed `invoke` wrapper (`api.ts`)

**Files:**
- Create: `src/api.ts`, `src/api.test.ts`

**Interfaces:**
- Consumes: the `scan` / `read_file_content` commands (Tasks 9, 10).
- Produces: TS interfaces `Capabilities`, `Category`, `Scope`, `HarnessItem`, `ScanResult` (camelCase, matching Task 2); `api.scan(harness: string): Promise<ScanResult>`; `api.readFileContent(path: string): Promise<string>`.

- [ ] **Step 1: Write the failing test** (mock the Tauri core module)

Create `src/api.test.ts`:
```ts
import { vi, test, expect, beforeEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { api } from './api';

beforeEach(() => invoke.mockReset());

test('scan calls invoke with harness arg', async () => {
  invoke.mockResolvedValue({ harnessId: 'claude', categories: [], scopes: [], items: [], capabilities: {} });
  const res = await api.scan('claude');
  expect(invoke).toHaveBeenCalledWith('scan', { harness: 'claude' });
  expect(res.harnessId).toBe('claude');
});

test('readFileContent passes path', async () => {
  invoke.mockResolvedValue('file body');
  const body = await api.readFileContent('/Users/x/.claude/CLAUDE.md');
  expect(invoke).toHaveBeenCalledWith('read_file_content', { path: '/Users/x/.claude/CLAUDE.md' });
  expect(body).toBe('file body');
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `npm test -- api`
Expected: **FAIL** (`./api` has no `api` export).

- [ ] **Step 3: Implement `api.ts`**

Create `src/api.ts`:
```ts
import { invoke } from '@tauri-apps/api/core';

export interface Capabilities {
  contextBudget: boolean; mcpControls: boolean; mcpPolicy: boolean;
  mcpSecurity: boolean; sessions: boolean; effective: boolean; backup: boolean;
}
export interface Category { id: string; label: string; count: number; }
export interface Scope { id: string; kind: string; label: string; root: string; }
export interface HarnessItem {
  category: string; scopeId: string; name: string; path: string;
  movable: boolean; deletable: boolean; locked: boolean;
}
export interface ScanResult {
  harnessId: string; categories: Category[]; scopes: Scope[];
  items: HarnessItem[]; capabilities: Capabilities;
}

export const api = {
  scan: (harness: string) => invoke<ScanResult>('scan', { harness }),
  readFileContent: (path: string) => invoke<string>('read_file_content', { path }),
};
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- api`
Expected: 2 tests **PASS**.

- [ ] **Step 5: Commit**

```bash
git add src/api.ts src/api.test.ts
git commit -m "feat: typed invoke wrapper (api.scan, api.readFileContent)"
```

---

### Task 13: Frontend — Organizer (categories → items → detail)

**Files:**
- Create: `src/modes/Organizer.tsx`, `src/modes/Organizer.test.tsx`
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: `api` + types (Task 12), `Shell` (Task 11).
- Produces: an `Organizer` component that takes a `ScanResult` + a `loadFile(path)` callback, renders a category column (with counts), an item column grouped by scope, and a detail pane showing the selected file's content.

- [ ] **Step 1: Write the failing test**

Create `src/modes/Organizer.test.tsx`:
```tsx
import { render, fireEvent } from '@solidjs/testing-library';
import { Organizer } from './Organizer';
import type { ScanResult } from '../api';

const scan: ScanResult = {
  harnessId: 'claude',
  categories: [
    { id: 'skill', label: 'Skills', count: 1 },
    { id: 'memory', label: 'Memories', count: 1 },
  ],
  scopes: [{ id: 'global', kind: 'global', label: 'Global (~/.claude)', root: '/Users/x/.claude' }],
  items: [
    { category: 'skill', scopeId: 'global', name: 'brainstorming', path: '/p/SKILL.md', movable: true, deletable: true, locked: false },
  ],
  capabilities: { contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true, sessions: true, effective: true, backup: true },
};

test('shows category counts and lists items; clicking loads content', async () => {
  const loaded: string[] = [];
  const { getByText } = render(() => (
    <Organizer scan={scan} loadFile={async (p) => { loaded.push(p); return 'FILE BODY'; }} />
  ));
  getByText('Skills');
  getByText('1'); // count badge
  fireEvent.click(getByText('brainstorming'));
  // detail loads asynchronously
  await Promise.resolve();
  expect(loaded).toEqual(['/p/SKILL.md']);
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `npm test -- Organizer`
Expected: **FAIL** (`Organizer` not found).

- [ ] **Step 3: Implement `Organizer`**

Create `src/modes/Organizer.tsx`:
```tsx
import { createSignal, createMemo, For, Show } from 'solid-js';
import type { ScanResult } from '../api';

export function Organizer(props: { scan: ScanResult; loadFile: (path: string) => Promise<string> }) {
  const [activeCat, setActiveCat] = createSignal(props.scan.categories[0]?.id ?? '');
  const [detail, setDetail] = createSignal<string>('');
  const [selected, setSelected] = createSignal<string>('');

  const itemsForCat = createMemo(() =>
    props.scan.items.filter((i) => i.category === activeCat())
  );

  async function open(path: string) {
    setSelected(path);
    setDetail(await props.loadFile(path));
  }

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <div style={{ width: '220px', 'border-right': '1px solid var(--border)', padding: '10px 8px' }}>
        <div style={{ 'font-size': '10px', color: 'var(--text-dim)' }}>Categories</div>
        <For each={props.scan.categories}>
          {(c) => (
            <div onClick={() => setActiveCat(c.id)}
              style={{ display: 'flex', 'justify-content': 'space-between', padding: '5px 8px', margin: '3px 0',
                'border-radius': 'var(--radius)', cursor: 'pointer',
                background: activeCat() === c.id ? 'rgba(48,209,88,0.14)' : 'transparent' }}>
              <span>{c.label}</span><span style={{ color: 'var(--text-dim)' }}>{c.count}</span>
            </div>
          )}
        </For>
      </div>

      <div style={{ width: '300px', 'border-right': '1px solid var(--border)', padding: '10px 8px' }}>
        <For each={props.scan.scopes}>
          {(scope) => (
            <>
              <div style={{ 'font-size': '9px', color: 'var(--text-dim)', margin: '6px 0 3px' }}>{scope.label}</div>
              <For each={itemsForCat().filter((i) => i.scopeId === scope.id)}>
                {(item) => (
                  <div onClick={() => open(item.path)}
                    style={{ padding: '5px 8px', margin: '3px 0', 'border-radius': 'var(--radius)', cursor: 'pointer',
                      background: selected() === item.path ? 'var(--surface)' : 'transparent' }}>
                    {item.name}{item.locked ? ' 🔒' : ''}
                  </div>
                )}
              </For>
            </>
          )}
        </For>
      </div>

      <div style={{ flex: 1, padding: '12px' }}>
        <Show when={selected()} fallback={<div style={{ color: 'var(--text-dim)' }}>Select an item</div>}>
          <pre style={{ 'font-family': 'var(--font-mono)', 'font-size': '12px', 'white-space': 'pre-wrap' }}>{detail()}</pre>
        </Show>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- Organizer`
Expected: **PASS**.

- [ ] **Step 5: Wire `App.tsx` to load a real scan on mount**

Replace `src/App.tsx` with:
```tsx
import { createResource, createSignal, Show } from 'solid-js';
import { Shell } from './components/Shell';
import { Organizer } from './modes/Organizer';
import { api } from './api';

export default function App() {
  const [mode, setMode] = createSignal('organizer');
  const [scan] = createResource(() => api.scan('claude'));

  return (
    <Shell active={mode()} onSelect={setMode}>
      <Show when={scan()} fallback={<div style={{ padding: '16px' }}>Scanning ~/.claude…</div>}>
        {(result) => (
          <Show when={mode() === 'organizer'} fallback={<div style={{ padding: '16px', color: 'var(--text-dim)' }}>Coming in a later plan.</div>}>
            <Organizer scan={result()} loadFile={api.readFileContent} />
          </Show>
        )}
      </Show>
    </Shell>
  );
}
```

- [ ] **Step 6: Run the full frontend suite**

Run: `npm test`
Expected: Sidebar, api, Organizer tests all **PASS**.

- [ ] **Step 7: Commit**

```bash
git add src
git commit -m "feat: Organizer mode (categories, scope-grouped items, detail pane)"
```

---

### Task 14: End-to-end smoke against real `~/.claude`

**Files:**
- None (verification task). Optionally: `docs/superpowers/plans/plan-01-smoke.md` (checklist record).

**Interfaces:**
- Consumes: everything above.
- Produces: a verified running app.

- [ ] **Step 1: Launch the real app**

Run: `cd /Users/balakumar/personal/ward && npm run tauri dev`
Expected: the **Ward** window opens; after a brief "Scanning ~/.claude…" it shows the Organizer.

- [ ] **Step 2: Verify against known config** (this machine has `~/.claude/skills/` and `~/.claude/CLAUDE.md`)

Manual checklist — confirm each:
- [ ] Sidebar shows all 5 modes; **Organizer** is active/highlighted.
- [ ] **Skills** category shows a count > 0; clicking it lists skill names under "Global (~/.claude)".
- [ ] **Memories** category lists `CLAUDE.md` (with 🔒) plus any `~/.claude/memory/*.md`.
- [ ] Clicking an item loads its file content in the detail pane (monospace).
- [ ] Clicking Security/Budget/Sessions/Backups shows the "Coming in a later plan" placeholder (proves mode switching).

- [ ] **Step 3: Verify the safety boundary** (quick sanity)

In the running app's devtools console (right-click → Inspect, or `Cmd+Option+I`), run:
```js
await window.__TAURI__.core.invoke('read_file_content', { path: '/etc/passwd' })
```
Expected: rejects with a `pathEscaped` error object (not the file contents).

- [ ] **Step 4: Final commit / tag**

```bash
cd /Users/balakumar/personal/ward
git add -A
git commit -m "chore: Plan 01 foundation complete — native scan + browse Claude config" --allow-empty
git tag plan-01-foundation
```

---

## Self-Review

**1. Spec coverage (Plan 01 slice):**
- Native Tauri 2 app shell → Tasks 1, 11. ✓
- Rust core with typed commands over `invoke` → Tasks 9, 10, 12. ✓
- Normalized data model (`ScanResult`/`HarnessItem`/`Capabilities`) → Task 2, mirrored in TS Task 12. ✓
- Harness trait/registry (extensibility core) → Tasks 5, 6. ✓
- Claude adapter read-only scan → Tasks 6, 7, 8. ✓
- Organizer 3-column browse + detail → Task 13. ✓
- Security Console visual style → Task 11. ✓
- Path safety / home confinement / zero network → Tasks 4, 10, verified Task 14. ✓
- Deferred (correctly, to later plans, per spec §13): project scopes + 10 more categories (Plan 02), mutations (03), MCP controls (04), security scan (05), budget (06), sessions (07), backups (08), Codex (09), native menu-bar (10), Ward-as-MCP (11), packaging (12). ✓

**2. Placeholder scan:** No "TODO/TBD/handle edge cases" — every code step shows complete code; deferrals name the exact future plan. ✓

**3. Type consistency:** Rust `#[serde(rename_all="camelCase")]` fields (`scopeId`, `harnessId`, etc.) match the TS interfaces in Task 12 exactly. `scan_impl`/`read_file_impl`/`run_scan`/`build_registry`/`ensure_under_home`/`ClaudeAdapter`/`category_ids()==["skill","memory"]` are referenced with identical names/signatures across Tasks 5–13. `MODES` ids in Task 11 match the mode-switch check in Task 13's `App.tsx`. ✓
