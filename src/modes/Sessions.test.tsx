import { test, expect } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import { Sessions } from './Sessions';
import type { SessionsApi } from './Sessions';
import type { Conversation, ScanResult } from '../api';

// Regression coverage for the "(empty)" bug: real Claude/Codex turns are
// arrays of structured blocks — user turns are `tool_result`, assistant
// turns are `thinking` + `tool_use` (+ optional text). None carry a
// top-level `.text`, so the old renderer collapsed them all to "(empty)".
// These records mirror the real on-disk shapes.
const CONVO: Conversation = {
  sessionId: 'sess-1',
  records: [
    {
      kind: 'user',
      content: 'refactor the scan pipeline',
      blocks: [{ type: 'text', text: 'refactor the scan pipeline' }],
    },
    {
      kind: 'assistant',
      content: 'inspect commands.rs first\nRead: src-tauri/src/commands.rs',
      blocks: [
        { type: 'thinking', text: 'I should inspect commands.rs before editing.' },
        { type: 'toolUse', name: 'Read', inputSummary: 'src-tauri/src/commands.rs' },
      ],
      model: 'claude-opus-4-8',
      usage: { inputTokens: 100, outputTokens: 20 },
    },
    {
      kind: 'user',
      content: 'BUILD SUCCEEDED in 4.2s',
      blocks: [{ type: 'toolResult', content: 'BUILD SUCCEEDED in 4.2s' }],
    },
    // A system record with no summary must NOT render "(empty)".
    { kind: 'system', subtype: 'session_start' },
  ],
};

const SCAN = {
  harness: 'claude',
  items: [
    {
      category: 'session',
      scopeId: 'project',
      name: 'sess-1',
      path: '/x/projects/p/sess-1.jsonl',
      description: 'a session',
      movable: false,
      deletable: true,
      locked: false,
    },
  ],
} as unknown as ScanResult;

function makeApi(convo: Conversation = CONVO): SessionsApi {
  return {
    sessionPreview: () => Promise.resolve(convo),
    sessionCost: () =>
      Promise.resolve({
        totalInputTokens: 0,
        totalOutputTokens: 0,
        totalCacheRead: 0,
        totalCacheWrite: 0,
        perModel: [],
        estimatedCostUsd: 0,
        estimatedRecords: 0,
      }),
    sessionDistill: () =>
      Promise.resolve({
        originalPath: '',
        cleanedPath: '',
        backupPath: '',
        originalBytes: 0,
        cleanedBytes: 0,
        reductionPct: 0,
        indexMd: '',
      }),
    sessionTrim: () => Promise.resolve({} as never),
    restore: () => Promise.resolve(),
  };
}

test('renders tool-call, tool-result, and thinking blocks with their real text', async () => {
  const { getByTestId, findByTestId } = render(() => (
    <Sessions scan={SCAN} api={makeApi()} />
  ));
  fireEvent.click(getByTestId('sessions-btn-open'));
  await findByTestId('sessions-records');

  // 🔧 tool call — shows name + brief input.
  const toolUse = await findByTestId('sessions-block-tooluse');
  expect(toolUse.textContent).toContain('Read');
  expect(toolUse.textContent).toContain('src-tauri/src/commands.rs');

  // ↳ result — shows the tool output text.
  const toolResult = await findByTestId('sessions-block-toolresult');
  expect(toolResult.textContent).toContain('BUILD SUCCEEDED in 4.2s');

  // thinking — foldable <details>, text present in DOM.
  const thinking = await findByTestId('sessions-block-thinking');
  expect(thinking.tagName.toLowerCase()).toBe('details');
  expect(thinking.textContent).toContain('inspect commands.rs before editing');
});

test('no turn collapses to the "(empty)" placeholder', async () => {
  const { getByTestId, findByTestId, queryByText } = render(() => (
    <Sessions scan={SCAN} api={makeApi()} />
  ));
  fireEvent.click(getByTestId('sessions-btn-open'));
  await findByTestId('sessions-records');

  // The literal "(empty)" string is gone from the transcript entirely —
  // tool/thinking turns render their content, meta turns render nothing.
  expect(queryByText('(empty)')).toBeNull();
});

test('assistant turn that is ONLY a tool call still renders its text', async () => {
  const toolOnly: Conversation = {
    sessionId: 's',
    records: [
      {
        kind: 'assistant',
        content: 'Bash: cargo test',
        blocks: [{ type: 'toolUse', name: 'Bash', inputSummary: 'cargo test' }],
        model: 'claude-opus-4-8',
      },
    ],
  };
  const { getByTestId, findByTestId } = render(() => (
    <Sessions scan={SCAN} api={makeApi(toolOnly)} />
  ));
  fireEvent.click(getByTestId('sessions-btn-open'));
  const toolUse = await findByTestId('sessions-block-tooluse');
  expect(toolUse.textContent).toContain('Bash');
  expect(toolUse.textContent).toContain('cargo test');
});

// Task 4 — the viewer header shows the derived title (falling back to the
// sessionId) plus the selected `.jsonl` path as a copy-to-clipboard control.
test('viewer header shows the derived title and the copyable .jsonl path', async () => {
  const titled: Conversation = {
    sessionId: 'sess-1',
    title: 'Refactor the auth flow',
    records: [],
  };
  const { getByTestId, findByText, findByTestId } = render(() => (
    <Sessions scan={SCAN} api={makeApi(titled)} />
  ));
  fireEvent.click(getByTestId('sessions-btn-open'));
  await findByTestId('sessions-records');

  // Header shows the derived title, not the bare sessionId.
  expect(await findByText('Refactor the auth flow')).toBeTruthy();

  // Header shows the `.jsonl` path as a copy-to-clipboard control.
  const pathEl = await findByTestId('sessions-path');
  expect(pathEl.textContent).toContain('/x/projects/p/sess-1.jsonl');
});

test('viewer header falls back to the sessionId when the conversation has no title', async () => {
  // CONVO carries no `title`; the header must fall back to `sessionId`.
  const { getByTestId, findByTestId, container } = render(() => (
    <Sessions scan={SCAN} api={makeApi()} />
  ));
  fireEvent.click(getByTestId('sessions-btn-open'));
  await findByTestId('sessions-records');

  const titleEl = container.querySelector('.sx-convo-title');
  expect(titleEl?.textContent).toBe('sess-1');
});

// Task 5 — `other` records ("#N · attachment" etc.) render with empty bodies
// and `summary` is redundant with the header title. Both are hidden by
// default behind a "Show N system events" toggle. The `<For>` still
// iterates ALL records, so the preserved `sessions-record-N` indices don't
// renumber — a hidden record's testid is simply absent from the DOM.
test('other/summary noise rows are hidden by default and revealed by the toggle', async () => {
  const convo: Conversation = {
    sessionId: 'abc',
    title: 'T',
    records: [
      { kind: 'user', content: 'hi', blocks: [{ type: 'text', text: 'hi' }] },
      { kind: 'other', recordType: 'attachment' },
      { kind: 'summary', text: 'T' },
    ],
  };
  const { getByTestId, queryByTestId, findByTestId } = render(() => (
    <Sessions scan={SCAN} api={makeApi(convo)} />
  ));
  fireEvent.click(getByTestId('sessions-btn-open'));
  await findByTestId('sessions-records');

  // The user turn (record #0) always renders.
  expect(getByTestId('sessions-record-0')).toBeTruthy();
  // The `other` (record #1) and `summary` (record #2) are hidden by default.
  expect(queryByTestId('sessions-record-1')).toBeNull();
  expect(queryByTestId('sessions-record-2')).toBeNull();

  // The toggle reveals them — indices are preserved (still 1 and 2).
  fireEvent.click(getByTestId('sessions-toggle-system'));
  expect(getByTestId('sessions-record-1')).toBeTruthy();
  expect(getByTestId('sessions-record-2')).toBeTruthy();
});
