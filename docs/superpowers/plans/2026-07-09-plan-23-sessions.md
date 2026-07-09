# Plan 23 — Sessions: real titles + readable transcript

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show Claude Code's auto-generated session title in the Sessions list (instead of the raw UUID), and make the transcript readable — hide the empty "attachment"-style noise rows, render tool results as collapsible pretty-printed cards, and surface the on-disk `.jsonl` path.

**Architecture:** Backend (`sessions/parse.rs`) stops discarding the `{"type":"summary",…}` line and parses it into a real `SessionRecord::Summary`; a `derive_title` helper computes a title (summary → first user text → none); a bounded `session_head_title` gives the *list* a cheap title without full-parsing every transcript; both `scan_sessions` adapters use it. Frontend (`Sessions.tsx`) hides `other`/`summary` records behind a toggle, renders tool-result blocks as collapsible cards with JSON pretty-printed, and shows the title + path in the viewer header.

**Tech Stack:** Rust (serde_json), SolidJS + TS (Vite, vitest, @solidjs/testing-library).

## Global Constraints

- All Rust models derive `Debug, Clone, Serialize, Deserialize, PartialEq` with `#[serde(rename_all = "camelCase")]`. (Spec §1 / CLAUDE.md.)
- Frontend ↔ core ONLY via existing `invoke` wrappers; JS camelCase ↔ Rust snake_case is automatic. UI never touches the filesystem.
- New/changed UI uses **classes + tokens** (`src/styles/sessions.css`), never inline styles. **Preserve every existing `data-testid`** (tests + `tests/e2e/` depend on them).
- TDD: failing test → implement → green → commit. Every `cargo test` and `npm test` must pass before moving on. Never mark a task done with a failing/skipped test.
- One commit per task, conventional prefix (`feat:` / `refactor:` / `test:`). Commit `Cargo.lock` only if it changed (no new deps here, so it won't).
- Solid-testing-library gotchas: `<div>{a} {b}</div>` makes multiple text nodes that confuse strict `getByText` — wrap literal text in `<span>`; when several nodes share text use `getAllByText`.
- Reuse Plan 01/07 names exactly: `Conversation`, `SessionRecord`, `ContentBlock`, `parse_file`, `parse_line`, `scan_sessions`, `Sessions`, `BlockRow`, `headLabel`, `metaBody`.

---

## Task 1: Parse the `summary` line into `SessionRecord::Summary`

Claude Code writes `{"type":"summary","summary":"<title>","leafUuid":"<uuid>"}`. Today it falls through `classify`'s `_` arm to `SessionRecord::Other { record_type: "summary" }` and the title text is discarded. Give it a real record so downstream code (Task 2) can read it.

**Files:**
- Modify: `src-tauri/src/sessions/parse.rs` (`SessionRecord` enum + `classify`)
- Modify: `src/api.ts` (`SessionRecord` union — mirror)
- Modify: `src/modes/Sessions.tsx` (`headLabel` — exhaustiveness)
- Test: `src-tauri/src/sessions/parse.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `SessionRecord::Summary { text: String, leaf_uuid: Option<String> }` (serde tag `kind` → `"summary"`, camelCase fields → `text`, `leafUuid`). Task 2 reads `text`.

- [ ] **Step 1: Write the failing test** — append to the `tests` module in `src-tauri/src/sessions/parse.rs`:

```rust
#[test]
fn parse_line_summary_becomes_summary_record() {
    let line = r#"{"type":"summary","summary":"Refactor the auth flow","leafUuid":"abc-123"}"#;
    let rec = parse_line(line).expect("summary line parses");
    assert_eq!(
        rec,
        SessionRecord::Summary {
            text: "Refactor the auth flow".to_string(),
            leaf_uuid: Some("abc-123".to_string()),
        }
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib sessions::parse::tests::parse_line_summary_becomes_summary_record`
Expected: FAIL — the record parses as `Other { record_type: "summary" }`, not `Summary { … }` (assert_eq mismatch), OR a compile error that `Summary` is not a variant (add the variant next).

- [ ] **Step 3: Add the enum variant** — in `SessionRecord` (after the `AiTitle` variant, before `QueueOperation`):

```rust
    /// `{"type":"summary","summary":"...","leafUuid":"..."}` — Claude Code's
    /// auto-generated conversation title. Previously discarded as `Other`.
    Summary {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        leaf_uuid: Option<String>,
    },
```

- [ ] **Step 4: Classify it** — in `classify`, add an arm **before** the final `_ =>`:

```rust
        ("summary", _) => SessionRecord::Summary {
            text: obj
                .get("summary")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            leaf_uuid: obj
                .get("leafUuid")
                .and_then(|s| s.as_str())
                .map(str::to_string),
        },
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib sessions::parse`
Expected: PASS (all `sessions::parse` tests, including the new one).

- [ ] **Step 6: Mirror the type in TS + keep the frontend exhaustive** — in `src/api.ts`, add to the `SessionRecord` union (after the `aiTitle` line):

```ts
  | { kind: 'summary'; text: string; leafUuid?: string }
```

In `src/modes/Sessions.tsx`, add a case to `headLabel`'s switch (keeps it exhaustive so `tsc` stays green):

```ts
    case 'summary':
      return 'Summary';
```

- [ ] **Step 7: Verify types + tests green**

Run: `npx tsc --noEmit` → clean. `cd src-tauri && cargo test --lib sessions` → PASS.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/sessions/parse.rs src/api.ts src/modes/Sessions.tsx
git commit -m "feat(sessions): parse the auto-summary line into SessionRecord::Summary"
```

---

## Task 2: Derive a title on `Conversation`

**Files:**
- Modify: `src-tauri/src/sessions/parse.rs` (`Conversation` struct, `Conversation::empty`, `parse_file`, new `derive_title`)
- Modify: `src/api.ts` (`Conversation` interface)
- Test: `src-tauri/src/sessions/parse.rs`

**Interfaces:**
- Consumes: `SessionRecord::Summary { text, .. }` (Task 1), `SessionRecord::User { blocks, .. }`, `ContentBlock::Text`, `one_line_truncate` (exists in parse.rs).
- Produces: `Conversation.title: Option<String>` (serialized `title`, omitted when `None`); `fn derive_title(records: &[SessionRecord]) -> Option<String>`.

- [ ] **Step 1: Write the failing tests** — append to the `tests` module:

```rust
#[test]
fn derive_title_prefers_summary_line() {
    let recs = vec![
        SessionRecord::User { content: "hi".into(), blocks: vec![ContentBlock::Text { text: "hi".into() }], ts: None },
        SessionRecord::Summary { text: "Fix the login bug".into(), leaf_uuid: None },
    ];
    assert_eq!(derive_title(&recs), Some("Fix the login bug".to_string()));
}

#[test]
fn derive_title_falls_back_to_first_user_text() {
    let recs = vec![
        // A tool-result-only user record must be skipped in favour of real text.
        SessionRecord::User { content: "tool out".into(), blocks: vec![ContentBlock::ToolResult { content: "tool out".into() }], ts: None },
        SessionRecord::User { content: "Please add a logout button".into(), blocks: vec![ContentBlock::Text { text: "Please add a logout button".into() }], ts: None },
    ];
    assert_eq!(derive_title(&recs), Some("Please add a logout button".to_string()));
}

#[test]
fn derive_title_none_when_no_summary_or_user_text() {
    let recs = vec![SessionRecord::Other { record_type: "attachment".into() }];
    assert_eq!(derive_title(&recs), None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib sessions::parse::tests::derive_title`
Expected: FAIL — `derive_title` is not defined (compile error). Add it next.

- [ ] **Step 3: Implement `derive_title`** — add near the other helpers in `parse.rs`:

```rust
/// A human-readable title for a session: Claude Code's auto-generated summary
/// when present (the last one — later summaries reflect the most recent state
/// after compaction), else the first real user text prompt (first line,
/// truncated), else `None`.
fn derive_title(records: &[SessionRecord]) -> Option<String> {
    if let Some(t) = records.iter().rev().find_map(|r| match r {
        SessionRecord::Summary { text, .. } if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => None,
    }) {
        return Some(t);
    }
    records.iter().find_map(|r| match r {
        SessionRecord::User { blocks, .. } => blocks.iter().find_map(|b| match b {
            ContentBlock::Text { text } if !text.trim().is_empty() => {
                Some(one_line_truncate(text.trim(), 80))
            }
            _ => None,
        }),
        _ => None,
    })
}
```

- [ ] **Step 4: Add the field + wire it in** — change the `Conversation` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub records: Vec<SessionRecord>,
}
```

Update `Conversation::empty`:

```rust
    fn empty(session_id: String) -> Self {
        Self { session_id, title: None, records: Vec::new() }
    }
```

In `parse_file`, after the read loop and before `Ok(conv)`:

```rust
    conv.title = derive_title(&conv.records);
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib sessions`
Expected: PASS. (If any existing `parse` test constructs `Conversation { … }` literally, add `title: None`.)

- [ ] **Step 6: Mirror in TS** — in `src/api.ts`, extend the `Conversation` interface:

```ts
export interface Conversation {
  sessionId: string;
  title?: string;
  records: SessionRecord[];
}
```

Run: `npx tsc --noEmit` → clean.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/sessions/parse.rs src/api.ts
git commit -m "feat(sessions): derive a Conversation.title (summary -> first prompt)"
```

---

## Task 3: Cheap list title via `session_head_title`, wired into both `scan_sessions`

The Sessions *list* must show the title without full-parsing every (possibly large) transcript. Read a bounded head window and derive the title from it.

**Files:**
- Modify: `src-tauri/src/sessions/parse.rs` (new `session_head_title` + `SESSION_HEAD_CAP`)
- Modify: `src-tauri/src/harness/adapters/claude.rs` (`scan_sessions`)
- Modify: `src-tauri/src/harness/adapters/codex.rs` (`scan_sessions`)
- Test: `src-tauri/src/sessions/parse.rs`

**Interfaces:**
- Consumes: `derive_title`, `parse_line` (Tasks 1–2).
- Produces: `pub fn session_head_title(path: &Path, cap: usize) -> Option<String>`; `pub const SESSION_HEAD_CAP: usize = 256 * 1024;`.

- [ ] **Step 1: Write the failing test** — append to the `tests` module (uses a temp file):

```rust
#[test]
fn session_head_title_reads_summary_from_head() {
    use std::io::Write;
    let dir = std::env::temp_dir().join(format!("ward-sess-head-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.jsonl");
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, r#"{{"type":"summary","summary":"Wire the popover","leafUuid":"x"}}"#).unwrap();
    writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"hello"}}}}"#).unwrap();
    drop(f);
    assert_eq!(session_head_title(&path, SESSION_HEAD_CAP), Some("Wire the popover".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib sessions::parse::tests::session_head_title_reads_summary_from_head`
Expected: FAIL — `session_head_title` / `SESSION_HEAD_CAP` not defined (compile error).

- [ ] **Step 3: Implement it** — in `parse.rs` (ensure `use std::io::Read;` is present alongside the existing `BufRead`/`BufReader` imports):

```rust
/// Max bytes read by `session_head_title`. Titles (the summary line or the
/// first user prompt) live near the top of a transcript, so a bounded head
/// read keeps the Sessions *list* cheap even for large files.
pub const SESSION_HEAD_CAP: usize = 256 * 1024;

/// Cheap title for the sessions LIST: read at most `cap` bytes and derive a
/// title without parsing the whole transcript. `None` if nothing usable is in
/// the head window (caller falls back to the file stem).
pub fn session_head_title(path: &Path, cap: usize) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    BufReader::new(file).take(cap as u64).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let records: Vec<SessionRecord> = text.lines().filter_map(parse_line).collect();
    derive_title(&records)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib sessions::parse`
Expected: PASS.

- [ ] **Step 5: Wire into Claude `scan_sessions`** — in `src-tauri/src/harness/adapters/claude.rs`, replace the `let name = …; let stem = …;` block and the `name: stem,` field so the row name is the title (fall back to the stem):

```rust
        let name = entry.file_name().to_string_lossy().to_string();
        let stem = name.trim_end_matches(".jsonl").to_string();
        let title = crate::sessions::parse::session_head_title(
            &p,
            crate::sessions::parse::SESSION_HEAD_CAP,
        )
        .unwrap_or_else(|| stem.clone());
        items.push(HarnessItem {
            category: "session".into(),
            scope_id: scope.id.clone(),
            name: title,
            description: String::new(),
            path: p.display().to_string(),
            movable: false, deletable: false, locked: true,
            effective: None,
            mcp_config: None,
        });
```

- [ ] **Step 6: Wire into Codex `scan_sessions`** — in `src-tauri/src/harness/adapters/codex.rs`, for the per-file loop (the rollout files, NOT the `session_index.jsonl` item), set the name to the title with the session id as fallback:

```rust
    for path in paths {
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let session_id = extract_session_id(&file_name);
        let title = crate::sessions::parse::session_head_title(
            &path,
            crate::sessions::parse::SESSION_HEAD_CAP,
        )
        .unwrap_or_else(|| session_id.clone());
        items.push(HarnessItem {
            category: "session".into(),
            scope_id: scope.id.clone(),
            name: title,
            description: file_name,
            path: path.display().to_string(),
            movable: false, deletable: false, locked: false,
            effective: None, mcp_config: None,
        });
    }
```

- [ ] **Step 7: Run the adapter tests** (an existing test asserts session scanning)

Run: `cd src-tauri && cargo test --lib harness::adapters::claude::tests::scan_sessions`
Expected: PASS. If `scan_sessions_returns_jsonl_in_project_dir` asserts `name == <uuid>`, update it to assert on `path`/`category` (the name is now a title); the fixture line it writes has no summary/user-text so `session_head_title` returns `None` and the name falls back to the stem — the assertion may already hold. Run and adjust only if it fails.

- [ ] **Step 8: Full backend suite green**

Run: `cd src-tauri && cargo test`
Expected: PASS (0 failed).

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/sessions/parse.rs src-tauri/src/harness/adapters/claude.rs src-tauri/src/harness/adapters/codex.rs
git commit -m "feat(sessions): show the derived title in the session list (both harnesses)"
```

---

## Task 4: Show the title + `.jsonl` path in the viewer header

**Files:**
- Modify: `src/modes/Sessions.tsx` (viewer header `sx-convo-head`)
- Modify: `src/styles/sessions.css`
- Test: `src/modes/Sessions.test.tsx` (create if absent; otherwise extend)

**Interfaces:**
- Consumes: `Conversation.title` (Task 2), the existing `selectedPath()` signal (the `.jsonl` path).

- [ ] **Step 1: Write the failing test** — in `src/modes/Sessions.test.tsx`. If the file exists, add these tests reusing its existing render/mock helpers; if not, create it with this self-contained setup:

```tsx
import { render } from '@solidjs/testing-library';
import { Sessions, type SessionsApi } from './Sessions';
import type { Conversation, ScanResult } from '../api';

function scanWith(path: string): ScanResult {
  return {
    harnessId: 'claude',
    items: [{ category: 'session', scopeId: 'proj', name: 'My Title', description: '', path, movable: false, deletable: false, locked: true, effective: null, mcpConfig: null }],
    categories: [], scopes: [], capabilities: {} as never, findings: [],
  } as unknown as ScanResult;
}

function apiWith(convo: Conversation): SessionsApi {
  return {
    sessionPreview: async () => convo,
    sessionCost: async () => ({ totalInputTokens: 0, totalOutputTokens: 0, totalCacheRead: 0, totalCacheWrite: 0, perModel: [], estimatedCostUsd: 0, estimatedRecords: 0 }),
    sessionDistill: async () => ({ originalPath: '', cleanedPath: '', backupPath: '', originalBytes: 0, cleanedBytes: 0, reductionPct: 0, indexMd: '' }),
    sessionTrim: async () => ({ kind: 'trim', originalPath: '', currentPath: null, backupBytes: null } as never),
    restore: async () => {},
  };
}

test('viewer header shows the title and the .jsonl path', async () => {
  const path = '/Users/x/.claude/projects/proj/abc.jsonl';
  const convo: Conversation = { sessionId: 'abc', title: 'Refactor the auth flow', records: [] };
  const { getByTestId, findByText } = render(() => <Sessions scan={scanWith(path)} api={apiWith(convo)} />);
  getByTestId('sessions-row').click();
  expect(await findByText('Refactor the auth flow')).toBeTruthy();
  expect(await findByText(path)).toBeTruthy();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm test -- Sessions`
Expected: FAIL — the header renders `abc · 0 records`, not the title/path.

- [ ] **Step 3: Implement the header** — in `Sessions.tsx`, replace the `<h2 class="sx-convo-head">…</h2>` (currently `{c().sessionId} · {c().records.length} records`) with:

```tsx
                <header class="sx-convo-head">
                  <div class="sx-convo-title">{c().title || c().sessionId}</div>
                  <div class="sx-convo-meta">
                    <span>{c().records.length} records</span>
                    <button
                      type="button"
                      class="sx-convo-path"
                      title="Copy path"
                      onClick={() => navigator.clipboard?.writeText(selectedPath())}
                      data-testid="sessions-path"
                    >
                      {selectedPath()}
                    </button>
                  </div>
                </header>
```

- [ ] **Step 4: Style it** — append to `src/styles/sessions.css`:

```css
.sx-convo-head { display: flex; flex-direction: column; gap: 4px; margin: 0 0 12px; }
.sx-convo-title { font-size: 15px; font-weight: 600; color: var(--text); }
.sx-convo-meta { display: flex; align-items: center; gap: 10px; color: var(--text-dim); font-size: 12px; }
.sx-convo-path {
  background: transparent; border: none; padding: 0; cursor: pointer;
  color: var(--text-dim); font-family: var(--font-mono); font-size: 11px;
  max-width: 60ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  transition: color 120ms ease;
}
.sx-convo-path:hover { color: var(--accent); }
```

- [ ] **Step 5: Run test to verify it passes**

Run: `npm test -- Sessions`
Expected: PASS.

- [ ] **Step 6: Verify types + commit**

```bash
npx tsc --noEmit
git add src/modes/Sessions.tsx src/styles/sessions.css src/modes/Sessions.test.tsx
git commit -m "feat(sessions): show session title + copyable .jsonl path in the viewer header"
```

---

## Task 5: Hide the empty `other`/`summary` noise rows behind a toggle

The `#N · attachment` (and `file-history-snapshot`/`progress`/`pr-link`/`custom-title`) rows render with empty bodies — the user's "empty numbers saying attachment". `summary` is now redundant with the header title. Hide both by default; a toggle reveals them, preserving existing record indices/testids.

**Files:**
- Modify: `src/modes/Sessions.tsx`
- Modify: `src/styles/sessions.css`
- Test: `src/modes/Sessions.test.tsx`

- [ ] **Step 1: Write the failing test** — add to `Sessions.test.tsx`:

```tsx
test('other/summary noise rows are hidden by default and revealed by the toggle', async () => {
  const convo: Conversation = {
    sessionId: 'abc', title: 'T',
    records: [
      { kind: 'user', content: 'hi', blocks: [{ type: 'text', text: 'hi' }] },
      { kind: 'other', recordType: 'attachment' },
      { kind: 'summary', text: 'T' },
    ],
  };
  const { getByTestId, queryByTestId, findByTestId } = render(() => <Sessions scan={scanWith('/p/abc.jsonl')} api={apiWith(convo)} />);
  getByTestId('sessions-row').click();
  await findByTestId('sessions-records');
  // The attachment (record #1) is hidden until the toggle is on.
  expect(queryByTestId('sessions-record-1')).toBeNull();
  getByTestId('sessions-toggle-system').click();
  expect(getByTestId('sessions-record-1')).toBeTruthy();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm test -- Sessions`
Expected: FAIL — `sessions-record-1` is present without a toggle; `sessions-toggle-system` does not exist.

- [ ] **Step 3: Implement** — in `Sessions.tsx`:

Add near the top-level helpers (module scope):

```ts
/** Record kinds/types that render with no body — noise in the transcript.
 *  `other` records are always empty; `summary` is shown as the header title. */
function isNoiseRecord(rec: SessionRecord): boolean {
  return rec.kind === 'other' || rec.kind === 'summary';
}
```

Inside the `Sessions` component, add a signal (next to the other `createSignal`s):

```ts
  const [showSystem, setShowSystem] = createSignal(false);
```

In the records section, add a memo for the hidden count and a toggle above the `<For>`. Replace the opening of the records block (`<div data-testid="sessions-records" …><h2 …>` down to the `<For each={c().records}>`) so it reads:

```tsx
              <div data-testid="sessions-records" class="sx-records">
                {/* header from Task 4 stays here */}
                <Show when={c().records.filter(isNoiseRecord).length > 0}>
                  <button
                    type="button"
                    class="sx-system-toggle"
                    data-testid="sessions-toggle-system"
                    onClick={() => setShowSystem((v) => !v)}
                  >
                    {showSystem() ? 'Hide' : 'Show'} {c().records.filter(isNoiseRecord).length} system events
                  </button>
                </Show>
                <For each={c().records}>
```

Wrap each rendered record row in a `<Show>` gate. The `<For>` callback currently returns `<div data-testid={`sessions-record-${i()}`} …>`; wrap that returned element:

```tsx
                    return (
                      <Show when={showSystem() || !isNoiseRecord(rec)}>
                        {/* the existing <div data-testid={`sessions-record-${i()}`} …> … </div> unchanged */}
                      </Show>
                    );
```

(Keep the existing record `<div>` and its children exactly as-is inside the `<Show>`; only the wrapper is new. Indices/testids are preserved because the `<For>` still iterates all records.)

- [ ] **Step 4: Style the toggle** — append to `src/styles/sessions.css`:

```css
.sx-system-toggle {
  align-self: flex-start; margin: 0 0 8px; padding: 2px 8px;
  background: var(--surface-3); border: 1px solid var(--sh-1); border-radius: var(--r-sm);
  color: var(--text-dim); font-size: 11px; cursor: pointer;
  transition: color 120ms ease, border-color 120ms ease;
}
.sx-system-toggle:hover { color: var(--text); border-color: var(--accent); }
```

- [ ] **Step 5: Run test to verify it passes + no regressions**

Run: `npm test -- Sessions`
Expected: PASS (new test + existing Sessions tests).

- [ ] **Step 6: Commit**

```bash
git add src/modes/Sessions.tsx src/styles/sessions.css src/modes/Sessions.test.tsx
git commit -m "feat(sessions): hide empty system/summary rows behind a toggle"
```

---

## Task 6: Collapsible, pretty-printed tool-result cards

Tool results (`toolResult.content`) are dumped verbatim — usually raw JSON or full file reads (the "lots of json"). Render them as cards: pretty-print detected JSON; collapse long results behind a `<details>`.

**Files:**
- Modify: `src/modes/Sessions.tsx` (`BlockRow` + a `prettyIfJson` helper)
- Modify: `src/styles/sessions.css`
- Test: `src/modes/Sessions.test.tsx`

**Interfaces:**
- Produces: `function prettyIfJson(s: string): { text: string; isJson: boolean }` (module scope).

- [ ] **Step 1: Write the failing tests** — add to `Sessions.test.tsx`:

```tsx
test('a JSON tool result is pretty-printed', async () => {
  const convo: Conversation = {
    sessionId: 'abc', title: 'T',
    records: [{ kind: 'user', content: '', blocks: [{ type: 'toolResult', content: '{"a":1,"b":2}' }] }],
  };
  const { getByTestId, findByTestId } = render(() => <Sessions scan={scanWith('/p/abc.jsonl')} api={apiWith(convo)} />);
  getByTestId('sessions-row').click();
  const body = await findByTestId('sessions-block-toolresult');
  // Pretty-printed JSON spans multiple lines with indentation.
  expect(body.textContent).toContain('"a": 1');
  expect(body.textContent).toContain('\n');
});

test('a long tool result is collapsed in a <details>', async () => {
  const long = 'x'.repeat(600);
  const convo: Conversation = {
    sessionId: 'abc', title: 'T',
    records: [{ kind: 'user', content: '', blocks: [{ type: 'toolResult', content: long }] }],
  };
  const { getByTestId, container } = render(() => <Sessions scan={scanWith('/p/abc.jsonl')} api={apiWith(convo)} />);
  getByTestId('sessions-row').click();
  await Promise.resolve();
  expect(container.querySelector('[data-testid="sessions-block-toolresult"] details')).toBeTruthy();
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npm test -- Sessions`
Expected: FAIL — content renders verbatim (no `"a": 1` pretty spacing) and there is no `<details>`.

- [ ] **Step 3: Implement** — in `Sessions.tsx`, add the helper at module scope:

```ts
/** Pretty-print a tool-result string when it is JSON, else return it as-is. */
function prettyIfJson(s: string): { text: string; isJson: boolean } {
  const t = s.trim();
  if (t.length > 1 && (t[0] === '{' || t[0] === '[')) {
    try {
      return { text: JSON.stringify(JSON.parse(t), null, 2), isJson: true };
    } catch {
      /* not JSON — fall through */
    }
  }
  return { text: s, isJson: false };
}
```

Replace the `case 'toolResult':` block in `BlockRow` with:

```tsx
    case 'toolResult': {
      const { text, isJson } = prettyIfJson(b.content);
      const lines = text.split('\n').length;
      const long = text.length > 400 || lines > 8;
      const body = (
        <pre class="sx-block-result-body" classList={{ 'is-json': isJson }}>{text}</pre>
      );
      return (
        <div data-testid="sessions-block-toolresult" class="sx-block sx-block--toolresult">
          <span class="sx-block-result-arrow" aria-hidden="true">↳</span>
          <Show when={long} fallback={body}>
            <details class="sx-result-details">
              <summary class="sx-result-summary">tool result · {isJson ? 'JSON' : 'text'} · {lines} lines</summary>
              {body}
            </details>
          </Show>
        </div>
      );
    }
```

- [ ] **Step 4: Style the card** — append to / adjust in `src/styles/sessions.css` (the existing `.sx-block-result-body` keeps its scroll cap; add the details + JSON affordances):

```css
.sx-result-details { flex: 1; min-width: 0; }
.sx-result-summary { cursor: pointer; color: var(--text-dim); font-size: 11px; user-select: none; }
.sx-result-summary:hover { color: var(--accent); }
.sx-block-result-body.is-json { color: var(--accent); }
.sx-block-result-body { margin: 4px 0 0; }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `npm test -- Sessions`
Expected: PASS.

- [ ] **Step 6: Verify types + full JS suite + commit**

```bash
npx tsc --noEmit
npm test
git add src/modes/Sessions.tsx src/styles/sessions.css src/modes/Sessions.test.tsx
git commit -m "feat(sessions): collapsible, JSON-pretty-printed tool-result cards"
```

---

## Self-Review (completed by plan author)

**Spec coverage** (against spec §3):
- Parse summary line → Task 1. ✓
- Title precedence summary → first prompt → UUID → Task 2 (`derive_title`) + Task 3 (list fallback to stem) + Task 4 (header falls back to `sessionId`). ✓
- Cheap list title (bounded scan) → Task 3. ✓
- Hide noise rows + toggle → Task 5. ✓
- Collapsible pretty-printed tool results → Task 6. ✓
- Show `.jsonl` path → Task 4. ✓

**Placeholder scan:** no TBD/TODO; every code step shows the actual code. ✓

**Type consistency:** `SessionRecord::Summary { text, leaf_uuid }` (Rust) ↔ `{ kind:'summary'; text; leafUuid? }` (TS); `Conversation.title: Option<String>` ↔ `title?: string`; `derive_title` / `session_head_title` / `SESSION_HEAD_CAP` used consistently across Tasks 2–3; `prettyIfJson` / `isNoiseRecord` defined once (Tasks 5–6). ✓

**Deferred (noted, not in this plan):** per-record message count + relative timestamps in the list sub-line (needs a full read or an extra field); leafUuid→last-message matching for multi-summary files (uses "last summary" heuristic instead).
