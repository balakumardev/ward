import { render, fireEvent, waitFor, cleanup } from '@solidjs/testing-library';
import { afterEach, beforeEach } from 'vitest';
import { Organizer, type OrganizerApi } from './Organizer';
import type { ScanResult } from '../api';

// Aggressive cleanup — vitest's default auto-cleanup sometimes leaves
// stale Solid root components behind, which then show "Select an item"
// alongside the freshly-rendered one. We unmount every test explicitly.
beforeEach(() => { document.body.innerHTML = ''; });
afterEach(() => { cleanup(); document.body.innerHTML = ''; });
// NOTE: brief verbatim uses `getByText('1')` but both categories have count=1,
// which makes the DOM contain two matches. `getByText` throws on multiple
// matches, so the test cannot pass verbatim. Minimal deviation: use
// `getAllByText` to honor the same intent (assert the count badge is visible).

const noopApi: OrganizerApi = {
  listDestinations: async () => [],
  moveItem: async () => ({ kind: 'file', originalPath: '' }),
  deleteItem: async () => ({ kind: 'file', originalPath: '' }),
  restore: async () => undefined,
  bulkRestore: async () => undefined,
  saveFile: async () => undefined,
  bulk: async () => [],
};

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
  const { getByText, getAllByText } = render(() => (
    <Organizer scan={scan} loadFile={async (p) => { loaded.push(p); return 'FILE BODY'; }} api={noopApi} />
  ));
  getByText('Skills');
  getAllByText('1'); // count badges (multiple categories with count=1)
  fireEvent.click(getByText('brainstorming'));
  // detail loads asynchronously
  await Promise.resolve();
  expect(loaded).toEqual(['/p/SKILL.md']);
});

// ── Show-Effective toggle ──

const effectiveScan: ScanResult = {
  harnessId: 'claude',
  categories: [
    { id: 'mcp', label: 'MCP', count: 3 },
    { id: 'command', label: 'Commands', count: 3 },
  ],
  scopes: [
    { id: 'global', kind: 'global', label: 'Global', root: '/Users/x/.claude' },
    { id: 'repo-a', kind: 'project', label: 'repo-a', root: '/work/company/repo-a' },
  ],
  items: [
    // Project active winner
    { category: 'mcp', scopeId: 'repo-a', name: 'github', path: '/p/.mcp.json', movable: true, deletable: true, locked: false },
    // Global shadowed (same name as project github)
    { category: 'mcp', scopeId: 'global', name: 'github', path: '/g/.mcp.json', movable: true, deletable: true, locked: false, effective: 'shadowed' },
    // Global active (no tag, but won't show when toggle is ON because it's global without a tag)
    { category: 'mcp', scopeId: 'global', name: 'slack', path: '/g/.mcp.json', movable: true, deletable: true, locked: false },

    // Command conflict (same name in both)
    { category: 'command', scopeId: 'repo-a', name: 'deploy', path: '/p/cmds/deploy.md', movable: true, deletable: true, locked: false, effective: 'conflict' },
    { category: 'command', scopeId: 'global', name: 'deploy', path: '/g/cmds/deploy.md', movable: true, deletable: true, locked: false, effective: 'conflict' },
    // Project-only command (active)
    { category: 'command', scopeId: 'repo-a', name: 'build', path: '/p/cmds/build.md', movable: true, deletable: true, locked: false },
  ],
  capabilities: { contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true, sessions: true, effective: true, backup: true },
};

test('show-effective toggle hides global items without a tag when ON', async () => {
  const { getByTestId, queryByText, getAllByText } = render(() => (
    <Organizer scan={effectiveScan} loadFile={async () => 'x'} api={noopApi} />
  ));

  // Default category is mcp; all items show.
  expect(getAllByText('github').length).toBeGreaterThan(0);
  expect(getAllByText('slack').length).toBeGreaterThan(0);

  // Activate the toggle.
  const toggle = getByTestId('show-effective-toggle') as HTMLInputElement;
  fireEvent.input(toggle, { target: { checked: true } });

  // 'slack' (global, no tag) is hidden — only project items and tagged items remain.
  expect(queryByText('slack')).toBeNull();
  // 'github' is still visible — project (active) + global (shadowed).
  expect(getAllByText('github').length).toBeGreaterThan(0);
});

test('show-effective toggle renders badges for shadowed/conflict/ancestor', async () => {
  const { getByTestId, getAllByText } = render(() => (
    <Organizer scan={effectiveScan} loadFile={async () => 'x'} api={noopApi} />
  ));

  const toggle = getByTestId('show-effective-toggle') as HTMLInputElement;
  fireEvent.input(toggle, { target: { checked: true } });

  // Default category is mcp; 'github' is shown with the 'shadowed' badge.
  expect(getAllByText('🌫 shadowed').length).toBeGreaterThan(0);

  // Switch to Commands category to surface the conflict badge.
  const commands = getAllByText('Commands');
  expect(commands.length).toBeGreaterThan(0);
  // Click the category in the sidebar (first occurrence).
  fireEvent.click(commands[0]);
  expect(getAllByText('⚠ conflict').length).toBeGreaterThan(0);
});

// ── Plan 03: Move / Delete / Undo / Editor / Bulk ──

/** Wait for all microtasks + a single macrotask to settle. Solid's
 *  reactive updates and the multiple `await`s in our mutation
 *  handlers all need more than one `Promise.resolve()` to drain. */
async function settle() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

const mutableScan: ScanResult = {
  harnessId: 'claude',
  categories: [
    { id: 'skill', label: 'Skills', count: 2 },
  ],
  scopes: [
    { id: 'global', kind: 'global', label: 'Global', root: '/Users/x/.claude' },
    { id: 'repo-a', kind: 'project', label: 'repo-a', root: '/work/repo-a' },
  ],
  items: [
    { category: 'skill', scopeId: 'global', name: 'a', path: '/g/a/SKILL.md', movable: true, deletable: true, locked: false },
    { category: 'skill', scopeId: 'global', name: 'b', path: '/g/b/SKILL.md', movable: true, deletable: true, locked: false },
  ],
  capabilities: { contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true, sessions: true, effective: true, backup: true },
};

test('move menu lists destinations returned by api.listDestinations', async () => {
  const fakeApi: OrganizerApi = {
    ...noopApi,
    listDestinations: async () => [
      { scopeId: 'repo-a', label: 'repo-a', kind: 'project' },
    ],
  };
  const { getAllByTestId, getByTestId } = render(() => (
    <Organizer scan={mutableScan} loadFile={async () => 'body'} api={fakeApi} />
  ));
  const rows = getAllByTestId('item-row');
  fireEvent.click(rows[0]);
  await waitFor(() => expect(getByTestId('move-btn')).toBeTruthy());
  fireEvent.click(getByTestId('move-btn'));
  const menu = getByTestId('move-menu');
  expect(menu.textContent).toContain('repo-a');
});

test('clicking a destination calls api.moveItem', async () => {
  const moved: Array<{ item: any; destScopeId: string }> = [];
  const fakeApi: OrganizerApi = {
    ...noopApi,
    listDestinations: async () => [
      { scopeId: 'repo-a', label: 'repo-a', kind: 'project' },
    ],
    moveItem: async (item, destScopeId) => {
      moved.push({ item, destScopeId });
      return { kind: 'file', originalPath: item.path, currentPath: '/p/a/SKILL.md' };
    },
  };
  const { getAllByTestId, getByTestId } = render(() => (
    <Organizer scan={mutableScan} loadFile={async () => 'body'} api={fakeApi} />
  ));
  const rows = getAllByTestId('item-row');
  fireEvent.click(rows[0]);
  await waitFor(() => expect(getByTestId('move-btn')).toBeTruthy());
  fireEvent.click(getByTestId('move-btn'));
  fireEvent.click(getByTestId('move-dest'));
  await waitFor(() => expect(getByTestId('undo-btn')).toBeTruthy());
  expect(moved.length).toBe(1);
  expect(moved[0].destScopeId).toBe('repo-a');
});

test('delete calls api.deleteItem and shows undo', async () => {
  window.confirm = () => true;
  const deleted: any[] = [];
  const fakeApi: OrganizerApi = {
    ...noopApi,
    deleteItem: async (item) => { deleted.push(item); return { kind: 'file', originalPath: item.path }; },
  };
  const { getAllByTestId, getByTestId } = render(() => (
    <Organizer scan={mutableScan} loadFile={async () => 'body'} api={fakeApi} />
  ));
  const rows = getAllByTestId('item-row');
  fireEvent.click(rows[0]);
  await waitFor(() => expect(getByTestId('delete-btn')).toBeTruthy());
  fireEvent.click(getByTestId('delete-btn'));
  await waitFor(() => expect(getByTestId('undo-btn')).toBeTruthy());
  expect(deleted.length).toBe(1);
});

test('editor textarea edits and saves via api.saveFile', async () => {
  const saved: Array<{ path: string; content: string }> = [];
  const fakeApi: OrganizerApi = {
    ...noopApi,
    saveFile: async (path, content) => { saved.push({ path, content }); },
  };
  const { getAllByTestId, getByTestId } = render(() => (
    <Organizer scan={mutableScan} loadFile={async () => 'original'} api={fakeApi} />
  ));
  const rows = getAllByTestId('item-row');
  fireEvent.click(rows[0]);
  await waitFor(() => expect(getByTestId('detail-editor')).toBeTruthy());
  const editor = getByTestId('detail-editor') as HTMLTextAreaElement;
  expect(editor.value).toBe('original');
  fireEvent.input(editor, { target: { value: 'edited body' } });
  fireEvent.click(getByTestId('save-btn'));
  await waitFor(() => expect(saved.length).toBe(1));
  expect(saved[0]).toEqual({ path: '/g/a/SKILL.md', content: 'edited body' });
});

test('shift-click extends selection; bulk bar appears', async () => {
  const fakeApi: OrganizerApi = {
    ...noopApi,
    listDestinations: async () => [{ scopeId: 'repo-a', label: 'repo-a', kind: 'project' }],
  };
  const { getAllByTestId, getByTestId } = render(() => (
    <Organizer scan={mutableScan} loadFile={async () => 'body'} api={fakeApi} />
  ));
  const rows = getAllByTestId('item-row');
  expect(rows.length).toBe(2);
  fireEvent.click(rows[0]);
  await waitFor(() => expect(getByTestId('move-btn')).toBeTruthy());
  fireEvent.click(rows[1], { shiftKey: true });
  expect(getByTestId('bulk-bar')).toBeTruthy();
});

test('bulk move calls api.bulk with dest and stores combined undo', async () => {
  window.confirm = () => true;
  const bulked: Array<{ op: string; dest?: string; count: number }> = [];
  const fakeApi: OrganizerApi = {
    ...noopApi,
    listDestinations: async () => [{ scopeId: 'repo-a', label: 'repo-a', kind: 'project' }],
    bulk: async (items, op, destScopeId) => {
      bulked.push({ op, dest: destScopeId, count: items.length });
      return items.map((i) => ({ kind: 'file', originalPath: i.path }));
    },
  };
  const { getAllByTestId, getByTestId } = render(() => (
    <Organizer scan={mutableScan} loadFile={async () => 'body'} api={fakeApi} />
  ));
  const rows = getAllByTestId('item-row');
  fireEvent.click(rows[0]);
  await waitFor(() => expect(getByTestId('move-btn')).toBeTruthy());
  fireEvent.click(rows[1], { shiftKey: true });
  await waitFor(() => expect(getByTestId('bulk-bar')).toBeTruthy());
  const sel = getByTestId('bulk-dest') as HTMLSelectElement;
  sel.value = 'repo-a';
  fireEvent.change(sel, { target: { value: 'repo-a' } });
  fireEvent.click(getByTestId('bulk-move'));
  await waitFor(() => expect(bulked.length).toBe(1));
  expect(bulked[0].op).toBe('move');
  expect(bulked[0].count).toBe(2);
  expect(getByTestId('undo-btn')).toBeTruthy();
});