import { render, fireEvent, waitFor, cleanup, screen } from '@solidjs/testing-library';
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
  mcpGetDisabled: async () => [],
  mcpSetDisabled: async () => ({ kind: 'mcp-disabled', originalPath: '/Users/x/.claude.json' }),
  mcpGetPolicy: async () => ({ allowlist: [], denylist: [] }),
  upsertMcpEntry: async () => ({ kind: 'mcp-upsert', originalPath: '/Users/x/.claude.json' }),
  mcpImportJson: async () => [],
  skillUpsert: async () => ({ kind: 'skill-create', originalPath: '/Users/x/.claude/skills/x' }),
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
  capabilities: { contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true, sessions: true, effective: true, backup: true, mcpEditable: true, skillCreatable: true, pluginsManageable: true, settingsEditable: true },
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
  capabilities: { contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true, sessions: true, effective: true, backup: true, mcpEditable: true, skillCreatable: true, pluginsManageable: true, settingsEditable: true },
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
  capabilities: { contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true, sessions: true, effective: true, backup: true, mcpEditable: true, skillCreatable: true, pluginsManageable: true, settingsEditable: true },
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

test('delete shows an in-app confirm, then calls api.deleteItem and shows undo', async () => {
  const deleted: any[] = [];
  const fakeApi: OrganizerApi = {
    ...noopApi,
    deleteItem: async (item) => { deleted.push(item); return { kind: 'file', originalPath: item.path }; },
  };
  const { getAllByTestId, getByTestId, queryByTestId } = render(() => (
    <Organizer scan={mutableScan} loadFile={async () => 'body'} api={fakeApi} />
  ));
  const rows = getAllByTestId('item-row');
  fireEvent.click(rows[0]);
  await waitFor(() => expect(getByTestId('delete-btn')).toBeTruthy());
  fireEvent.click(getByTestId('delete-btn'));
  // The confirm modal appears — and NOTHING is deleted until it is confirmed.
  await waitFor(() => expect(getByTestId('confirm-ok')).toBeTruthy());
  expect(deleted.length).toBe(0);
  fireEvent.click(getByTestId('confirm-ok'));
  await waitFor(() => expect(getByTestId('undo-btn')).toBeTruthy());
  expect(deleted.length).toBe(1);
  expect(queryByTestId('confirm-ok')).toBeNull(); // modal closed
});

test('delete Cancel aborts without calling api.deleteItem', async () => {
  const deleted: any[] = [];
  const fakeApi: OrganizerApi = {
    ...noopApi,
    deleteItem: async (item) => { deleted.push(item); return { kind: 'file', originalPath: item.path }; },
  };
  const { getAllByTestId, getByTestId, queryByTestId } = render(() => (
    <Organizer scan={mutableScan} loadFile={async () => 'body'} api={fakeApi} />
  ));
  const rows = getAllByTestId('item-row');
  fireEvent.click(rows[0]);
  await waitFor(() => expect(getByTestId('delete-btn')).toBeTruthy());
  fireEvent.click(getByTestId('delete-btn'));
  await waitFor(() => expect(getByTestId('confirm-cancel')).toBeTruthy());
  fireEvent.click(getByTestId('confirm-cancel'));
  await waitFor(() => expect(queryByTestId('confirm-ok')).toBeNull());
  expect(deleted.length).toBe(0);
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
  // Markdown items now open in Preview; click into the preview to reach the editor.
  await waitFor(() => expect(getByTestId('detail-preview')).toBeTruthy());
  fireEvent.click(getByTestId('detail-preview'));
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

// ── Plan 04: MCP controls ──

const mcpScan: ScanResult = {
  harnessId: 'claude',
  categories: [{ id: 'mcp', label: 'MCP', count: 2 }],
  scopes: [
    { id: 'global', kind: 'global', label: 'Global', root: '/Users/x/.claude' },
    { id: 'repo-a', kind: 'project', label: 'repo-a', root: '/work/repo-a' },
  ],
  items: [
    { category: 'mcp', scopeId: 'global', name: 'github', path: '/g/.mcp.json',
      movable: true, deletable: true, locked: false,
      mcpConfig: { command: 'gh', args: ['api'] } },
    { category: 'mcp', scopeId: 'repo-a', name: 'evil', path: '/r/.mcp.json',
      movable: true, deletable: true, locked: false,
      mcpConfig: { command: 'python', args: ['evil.py'] } },
    { category: 'mcp', scopeId: 'repo-a', name: 'good', path: '/r/.mcp.json',
      movable: true, deletable: true, locked: false,
      mcpConfig: { command: 'node', args: ['approved.js'] } },
  ],
  capabilities: { contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true, sessions: true, effective: true, backup: true, mcpEditable: true, skillCreatable: true, pluginsManageable: true, settingsEditable: true },
};

test('disable toggle only appears for non-global MCP items', async () => {
  const { getAllByTestId, queryByTestId } = render(() => (
    <Organizer scan={mcpScan} loadFile={async () => '{}'} api={noopApi} />
  ));
  // Switch to MCP category (it's the default).
  // 2 project-scoped MCP rows should have toggles, 1 global row should not.
  const rows = getAllByTestId('item-row');
  expect(rows.length).toBe(3);
  const allToggles = document.querySelectorAll('[data-testid="mcp-disable-toggle"]');
  expect(allToggles.length).toBe(2);
  // Global row should NOT have a toggle.
  void queryByTestId; // ensure no leftover usage
});

test('policy badge shows allowed when allowlist matches by command', async () => {
  const fakeApi: OrganizerApi = {
    ...noopApi,
    mcpGetPolicy: async () => ({
      allowlist: [{ serverCommand: ['node', 'approved.js'] }],
      denylist: [],
    }),
  };
  const { getAllByTestId } = render(() => (
    <Organizer scan={mcpScan} loadFile={async () => '{}'} api={fakeApi} />
  ));
  await waitFor(() => {
    const rows = getAllByTestId('item-row');
    const goodRow = Array.from(rows).find((r) => r.getAttribute('data-item-name') === 'good')!;
    expect(goodRow.textContent).toContain('✓ allowed');
  });
});

test('policy badge shows denied when denylist matches by command', async () => {
  const fakeApi: OrganizerApi = {
    ...noopApi,
    mcpGetPolicy: async () => ({
      allowlist: [],
      denylist: [{ serverCommand: ['python', 'evil.py'] }],
    }),
  };
  const { getAllByTestId } = render(() => (
    <Organizer scan={mcpScan} loadFile={async () => '{}'} api={fakeApi} />
  ));
  await waitFor(() => {
    const rows = getAllByTestId('item-row');
    const evilRow = Array.from(rows).find((r) => r.getAttribute('data-item-name') === 'evil')!;
    expect(evilRow.textContent).toContain('🚫 denied');
  });
});

test('clicking toggle calls mcpSetDisabled and updates label', async () => {
  const setCalls: Array<{ projectPath: string; list: string[] }> = [];
  const fakeApi: OrganizerApi = {
    ...noopApi,
    mcpGetDisabled: async () => ['evil'], // start disabled
    mcpSetDisabled: async (projectPath, list) => {
      setCalls.push({ projectPath, list });
      return { kind: 'mcp-disabled', originalPath: '/Users/x/.claude.json' };
    },
  };
  const { getAllByTestId } = render(() => (
    <Organizer scan={mcpScan} loadFile={async () => '{}'} api={fakeApi} />
  ));
  // Click the row to trigger open() → mcpGetDisabled → toggle updates.
  const evilRow = Array.from(getAllByTestId('item-row'))
    .find((r) => r.getAttribute('data-item-name') === 'evil')!;
  fireEvent.click(evilRow);
  await waitFor(() => {
    const rows = getAllByTestId('item-row');
    const r = Array.from(rows).find((r) => r.getAttribute('data-item-name') === 'evil')!;
    expect(r.textContent).toContain('✗ Disabled');
  });
  const toggle = document.querySelector('[data-testid="mcp-disable-toggle"][data-disabled="true"]') as HTMLButtonElement;
  expect(toggle).toBeTruthy();
  fireEvent.click(toggle);
  await settle();
  expect(setCalls.length).toBe(1);
  expect(setCalls[0].projectPath).toBe('/work/repo-a');
  expect(setCalls[0].list).toEqual([]); // 'evil' removed → enabled
});

test('undo captures the disabled toggle as a RestoreInfo', async () => {
  const fakeApi: OrganizerApi = {
    ...noopApi,
    mcpGetDisabled: async () => [],
    mcpSetDisabled: async (_p, _list) => ({
      kind: 'mcp-disabled',
      originalPath: '/Users/x/.claude.json',
      mcpKey: '/work/repo-a',
      mcpParentKey: 'projects',
    }),
  };
  const { getAllByTestId, queryAllByTestId } = render(() => (
    <Organizer scan={mcpScan} loadFile={async () => '{}'} api={fakeApi} />
  ));
  // Click the row to trigger open() → mcpGetDisabled → toggle renders.
  const evilRow = Array.from(getAllByTestId('item-row'))
    .find((r) => r.getAttribute('data-item-name') === 'evil')!;
  fireEvent.click(evilRow);
  await waitFor(() => {
    const rows = getAllByTestId('item-row');
    const r = Array.from(rows).find((r) => r.getAttribute('data-item-name') === 'evil')!;
    expect(r.textContent).toContain('✓ Enabled');
  });
  const toggle = document.querySelector('[data-testid="mcp-disable-toggle"][data-disabled="false"]') as HTMLButtonElement;
  fireEvent.click(toggle);
  await waitFor(() => expect(queryAllByTestId('undo-btn').length).toBeGreaterThan(0));
});

// ── Plan 18: structured MCP edit form ──

/** Build a minimal single-MCP-item scan for exercising the structured
 *  MCP edit form. Mirrors the real ScanResult fixture shape used above. */
function makeScanWithMcp(opts: {
  name: string; scopeId: string; path: string; mcpConfig: Record<string, unknown>;
}): ScanResult {
  const scopes: ScanResult['scopes'] = [
    { id: 'global', kind: 'global', label: 'Global (~/.claude)', root: '/Users/x/.claude' },
  ];
  if (opts.scopeId !== 'global') {
    scopes.push({ id: opts.scopeId, kind: 'project', label: opts.scopeId, root: `/work/${opts.scopeId}` });
  }
  return {
    harnessId: 'claude',
    categories: [{ id: 'mcp', label: 'MCP', count: 1 }],
    scopes,
    items: [
      { category: 'mcp', scopeId: opts.scopeId, name: opts.name, path: opts.path,
        movable: true, deletable: true, locked: false, mcpConfig: opts.mcpConfig },
    ],
    capabilities: { contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true, sessions: true, effective: true, backup: true, mcpEditable: true, skillCreatable: true, pluginsManageable: true, settingsEditable: true },
  };
}

/** Reuse the file's noop api as the base fake for MCP-form tests. */
const fakeApi = noopApi;

function renderOrganizer(opts: { scan: ScanResult; api: OrganizerApi }) {
  return render(() => <Organizer scan={opts.scan} loadFile={async () => '{}'} api={opts.api} />);
}

/** Flexible ScanResult builder: pass arbitrary `items` and a partial
 *  `capabilities` override. Categories are derived from the items (so
 *  `category-<id>` testids render); scopes default to a single global. */
function makeScan(opts: {
  capabilities?: Partial<ScanResult['capabilities']>;
  items?: ScanResult['items'];
  scopes?: ScanResult['scopes'];
  categories?: ScanResult['categories'];
} = {}): ScanResult {
  const items = opts.items ?? [];
  const scopes = opts.scopes ?? [
    { id: 'global', kind: 'global', label: 'Global (~/.claude)', root: '/Users/x/.claude' },
  ];
  const categories = opts.categories ?? Array.from(new Set(items.map((i) => i.category)))
    .map((id) => ({ id, label: id, count: items.filter((it) => it.category === id).length }));
  const baseCaps: ScanResult['capabilities'] = {
    contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true,
    sessions: true, effective: true, backup: true, mcpEditable: true, skillCreatable: true,
    pluginsManageable: true, settingsEditable: true,
  };
  return {
    harnessId: 'claude',
    categories,
    scopes,
    items,
    capabilities: { ...baseCaps, ...(opts.capabilities ?? {}) },
  };
}

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

it('edits an arg row, adds an env var, and persists both (unknown keys survive)', async () => {
  const upsertSpy = vi.fn().mockResolvedValue({ kind: 'mcp-upsert', originalPath: '/x' });
  const scan = makeScanWithMcp({ name: 'ctx', scopeId: 'global',
    path: '/Users/x/.claude.json', mcpConfig: { command: 'npx', args: ['-y', 'pkg@1'], type: 'stdio' } });
  renderOrganizer({ scan, api: { ...fakeApi, upsertMcpEntry: upsertSpy } });
  fireEvent.click(screen.getByText('ctx'));
  await screen.findByTestId('mcp-form');
  // Edit the second existing arg row in place (exercises <Index> reactivity).
  const argInputs = screen.getAllByTestId('mcp-arg-input') as HTMLInputElement[];
  expect(argInputs.length).toBe(2);
  fireEvent.input(argInputs[1], { target: { value: 'pkg@2' } });
  // Add a new env var and fill both key + value.
  fireEvent.click(screen.getByTestId('mcp-env-add'));
  fireEvent.input(screen.getByTestId('mcp-env-key'), { target: { value: 'TOKEN' } });
  fireEvent.input(screen.getByTestId('mcp-env-value'), { target: { value: 'abc' } });
  fireEvent.click(screen.getByTestId('mcp-save'));
  await waitFor(() => expect(upsertSpy).toHaveBeenCalled());
  const [, config] = upsertSpy.mock.calls[0];
  expect(config.command).toBe('npx');
  expect(config.args).toEqual(['-y', 'pkg@2']);
  expect(config.env).toEqual({ TOKEN: 'abc' });
  expect(config.type).toBe('stdio'); // form-unmanaged key preserved via clone
});

it('switching transport to http persists url and drops stdio fields', async () => {
  const upsertSpy = vi.fn().mockResolvedValue({ kind: 'mcp-upsert', originalPath: '/x' });
  const scan = makeScanWithMcp({ name: 'ctx', scopeId: 'global',
    path: '/Users/x/.claude.json', mcpConfig: { command: 'npx', args: ['-y', 'pkg@1'] } });
  renderOrganizer({ scan, api: { ...fakeApi, upsertMcpEntry: upsertSpy } });
  fireEvent.click(screen.getByText('ctx'));
  await screen.findByTestId('mcp-form');
  expect(screen.getByTestId('mcp-command')).toBeInTheDocument();
  // Toggle to the http transport — stdio fields disappear, url appears.
  fireEvent.click(screen.getByTestId('mcp-transport-http'));
  const url = await screen.findByTestId('mcp-url') as HTMLInputElement;
  fireEvent.input(url, { target: { value: 'https://mcp.example.com/sse' } });
  expect(screen.queryByTestId('mcp-command')).not.toBeInTheDocument();
  fireEvent.click(screen.getByTestId('mcp-save'));
  await waitFor(() => expect(upsertSpy).toHaveBeenCalled());
  const [, config] = upsertSpy.mock.calls[0];
  expect(config.url).toBe('https://mcp.example.com/sse');
  expect(config.command).toBeUndefined();
  expect(config.args).toBeUndefined();
  expect(config.env).toBeUndefined();
});

it('re-seeds the MCP form when switching directly between two MCP servers', async () => {
  // Two MCP servers, both truthy + both category 'mcp'. Selecting A then B
  // directly (no non-MCP item in between) is the case a NON-keyed <Show>
  // fails to remount: McpForm's createSignal seeds would keep A's values,
  // so a subsequent Save would write B's entry with A's stale config.
  const scan: ScanResult = {
    harnessId: 'claude',
    categories: [{ id: 'mcp', label: 'MCP', count: 2 }],
    scopes: [{ id: 'global', kind: 'global', label: 'Global (~/.claude)', root: '/Users/x/.claude' }],
    items: [
      { category: 'mcp', scopeId: 'global', name: 'context7', path: '/Users/x/.claude.json',
        movable: true, deletable: true, locked: false, mcpConfig: { command: 'npx', args: ['-y', 'c7'] } },
      { category: 'mcp', scopeId: 'global', name: 'playwright', path: '/Users/x/.claude.json',
        movable: true, deletable: true, locked: false, mcpConfig: { command: 'pw', args: ['--headed'] } },
    ],
    capabilities: { contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true, sessions: true, effective: true, backup: true, mcpEditable: true, skillCreatable: true, pluginsManageable: true, settingsEditable: true },
  };
  renderOrganizer({ scan, api: fakeApi });

  // Select the first server — form seeds with its command.
  fireEvent.click(screen.getByText('context7'));
  expect((await screen.findByTestId('mcp-command') as HTMLInputElement).value).toBe('npx');

  // Directly select the second server. The form must re-seed to the SECOND
  // server's command ('pw'), NOT keep the first server's stale value ('npx').
  fireEvent.click(screen.getByText('playwright'));
  await waitFor(() => {
    expect((screen.getByTestId('mcp-command') as HTMLInputElement).value).toBe('pw');
  });
});

// ── Plan 18 Task 7: Add MCP Server flow ──

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

// ── Regression: the Add pane must dismiss on any selection ──
// The detail <Show> chain gates the Add-MCP / Add-Skill views AHEAD of the
// current selection, so unless the add flags are reset when the user selects
// something else, the add pane stays pinned over the newly-selected item.

it('dismisses the Add MCP pane when a different item is selected', async () => {
  const scan = makeScan({
    items: [
      { category: 'mcp', scopeId: 'global', name: 'ctx', path: '/Users/x/.claude.json',
        movable: true, deletable: true, locked: false, mcpConfig: { command: 'npx' } },
      { category: 'mcp', scopeId: 'global', name: 'other', path: '/Users/x/.claude.json',
        movable: true, deletable: true, locked: false, mcpConfig: { command: 'pw' } },
    ],
  });
  renderOrganizer({ scan, api: fakeApi });
  fireEvent.click(screen.getByTestId('category-mcp'));
  fireEvent.click(screen.getByTestId('mcp-add-button'));
  // The Add-MCP pane is open (its Cancel + tabs are unique to the add view).
  expect(await screen.findByTestId('mcp-add-cancel')).toBeInTheDocument();
  // Select an existing MCP item — the add pane must give way to its detail.
  fireEvent.click(screen.getByText('ctx'));
  await waitFor(() => expect(screen.queryByTestId('mcp-add-cancel')).toBeNull());
  expect(screen.queryByText('Add MCP Server')).toBeNull();
});

it('dismisses the Add MCP pane when a different category is selected', async () => {
  const scan = makeScan({
    items: [
      { category: 'mcp', scopeId: 'global', name: 'ctx', path: '/Users/x/.claude.json',
        movable: true, deletable: true, locked: false, mcpConfig: { command: 'npx' } },
      { category: 'skill', scopeId: 'global', name: 'brainstorming', path: '/g/SKILL.md',
        movable: true, deletable: true, locked: false },
    ],
  });
  renderOrganizer({ scan, api: fakeApi });
  fireEvent.click(screen.getByTestId('category-mcp'));
  fireEvent.click(screen.getByTestId('mcp-add-button'));
  expect(await screen.findByTestId('mcp-add-cancel')).toBeInTheDocument();
  // Switch to another category — the add pane must dismiss (empty state shows).
  fireEvent.click(screen.getByTestId('category-skill'));
  await waitFor(() => expect(screen.queryByTestId('mcp-add-cancel')).toBeNull());
  expect(screen.getByText('Select an item to view or edit')).toBeInTheDocument();
});

it('dismisses the Add Skill pane when a different item is selected', async () => {
  const scan = makeScan({
    items: [
      { category: 'skill', scopeId: 'global', name: 'alpha', path: '/g/a/SKILL.md',
        movable: true, deletable: true, locked: false },
      { category: 'skill', scopeId: 'global', name: 'beta', path: '/g/b/SKILL.md',
        movable: true, deletable: true, locked: false },
    ],
  });
  renderOrganizer({ scan, api: fakeApi });
  fireEvent.click(screen.getByTestId('category-skill'));
  fireEvent.click(screen.getByTestId('skill-add-button'));
  expect(await screen.findByTestId('skill-add-form')).toBeInTheDocument();
  fireEvent.click(screen.getByText('alpha'));
  await waitFor(() => expect(screen.queryByTestId('skill-add-form')).toBeNull());
  expect(screen.queryByText('Add Skill')).toBeNull();
});

// ── Markdown items: rendered Preview by default, click-to-edit ──

it('opens markdown items in rendered Preview by default', async () => {
  const scan = makeScan({
    items: [
      { category: 'skill', scopeId: 'global', name: 'brainstorming', path: '/g/SKILL.md',
        movable: true, deletable: true, locked: false },
    ],
  });
  render(() => (
    <Organizer scan={scan} loadFile={async () => '# Heading\n\nbody text'} api={fakeApi} />
  ));
  fireEvent.click(screen.getByText('brainstorming'));
  const preview = await screen.findByTestId('detail-preview');
  expect(preview.querySelector('h1')?.textContent).toBe('Heading');
  // The raw editor is not mounted until the user asks to edit.
  expect(screen.queryByTestId('detail-editor')).not.toBeInTheDocument();
});

it('clicking the markdown preview switches to Edit mode', async () => {
  const scan = makeScan({
    items: [
      { category: 'skill', scopeId: 'global', name: 'brainstorming', path: '/g/SKILL.md',
        movable: true, deletable: true, locked: false },
    ],
  });
  render(() => (
    <Organizer scan={scan} loadFile={async () => 'hello world'} api={fakeApi} />
  ));
  fireEvent.click(screen.getByText('brainstorming'));
  fireEvent.click(await screen.findByTestId('detail-preview'));
  const editor = await screen.findByTestId('detail-editor') as HTMLTextAreaElement;
  expect(editor.value).toBe('hello world');
  expect(screen.queryByTestId('detail-preview')).not.toBeInTheDocument();
});

it('keeps a locked markdown item in Preview (no click-to-edit)', async () => {
  const scan = makeScan({
    items: [
      { category: 'memory', scopeId: 'global', name: 'CLAUDE.md', path: '/Users/x/.claude/CLAUDE.md',
        movable: false, deletable: false, locked: true },
    ],
  });
  render(() => (
    <Organizer scan={scan} loadFile={async () => '# Root memory'} api={fakeApi} />
  ));
  fireEvent.click(screen.getByText('CLAUDE.md'));
  const preview = await screen.findByTestId('detail-preview');
  fireEvent.click(preview);
  await Promise.resolve();
  // Locked → the click must NOT drop into the editor.
  expect(screen.queryByTestId('detail-editor')).not.toBeInTheDocument();
  expect(screen.getByTestId('detail-preview')).toBeInTheDocument();
});

// ── Plan 18 review-fix 1: gate the editable form on capabilities.mcpEditable ──

it('renders a READ-ONLY MCP pane (no form, no add button) when mcpEditable is false', async () => {
  // A harness without an MCP upsert backend (e.g. Codex) must NOT show the
  // editable structured form or "+ Add" — it falls back to a read-only view.
  const scan: ScanResult = {
    harnessId: 'codex',
    categories: [{ id: 'mcp', label: 'MCP', count: 1 }],
    scopes: [{ id: 'global', kind: 'global', label: 'Global (~/.codex)', root: '/Users/x/.codex' }],
    items: [
      { category: 'mcp', scopeId: 'global', name: 'context7', path: '/Users/x/.codex/config.toml#context7',
        movable: false, deletable: true, locked: false, mcpConfig: { command: 'npx', args: ['-y', 'c7'] } },
    ],
    capabilities: { contextBudget: false, mcpControls: true, mcpPolicy: false, mcpSecurity: true, sessions: false, effective: false, backup: true, mcpEditable: false, skillCreatable: false, pluginsManageable: false, settingsEditable: false },
  };
  renderOrganizer({ scan, api: fakeApi });
  // "+ Add" is hidden when the harness can't edit MCP.
  expect(screen.queryByTestId('mcp-add-button')).not.toBeInTheDocument();
  // Selecting the MCP item shows the read-only editor, NOT the structured form.
  fireEvent.click(screen.getByText('context7'));
  const editor = await screen.findByTestId('detail-editor') as HTMLTextAreaElement;
  expect(editor).toBeInTheDocument();
  expect(editor.value).toContain('"command": "npx"'); // pretty-printed server config
  expect(screen.queryByTestId('mcp-form')).not.toBeInTheDocument();
  expect(screen.queryByTestId('mcp-save')).not.toBeInTheDocument();
});

// ── Plan 20: Codex Enabled toggle in the MCP form ──

/** An editable Codex scan carrying one stdio MCP server (mcpEditable on). */
function makeCodexMcpScan(mcpConfig: Record<string, unknown>): ScanResult {
  return {
    harnessId: 'codex',
    categories: [{ id: 'mcp', label: 'MCP', count: 1 }],
    scopes: [{ id: 'global', kind: 'global', label: 'Global (~/.codex)', root: '/Users/x/.codex' }],
    items: [
      { category: 'mcp', scopeId: 'global', name: 'context7', path: '/Users/x/.codex/config.toml',
        movable: false, deletable: true, locked: false, mcpConfig },
    ],
    capabilities: { contextBudget: false, mcpControls: false, mcpPolicy: false, mcpSecurity: true, sessions: false, effective: false, backup: true, mcpEditable: true, skillCreatable: true, pluginsManageable: false, settingsEditable: false },
  };
}

it('shows an Enabled checkbox (default true) for a Codex MCP item and persists the toggle on Save', async () => {
  const upsertSpy = vi.fn().mockResolvedValue({ kind: 'mcp-upsert', originalPath: '/x' });
  const scan = makeCodexMcpScan({ command: 'npx', args: ['-y', 'c7'] }); // no `enabled` key
  renderOrganizer({ scan, api: { ...fakeApi, upsertMcpEntry: upsertSpy } });
  fireEvent.click(screen.getByText('context7'));
  await screen.findByTestId('mcp-form');
  const check = screen.getByTestId('mcp-enabled') as HTMLInputElement;
  expect(check).toBeInTheDocument();
  expect(check.checked).toBe(true); // absent `enabled` defaults to true
  // Toggle it off, then Save — `enabled: false` must land in the config.
  fireEvent.click(check);
  fireEvent.click(screen.getByTestId('mcp-save'));
  await waitFor(() => expect(upsertSpy).toHaveBeenCalled());
  const [, config] = upsertSpy.mock.calls[0];
  expect(config.enabled).toBe(false);
  expect(config.command).toBe('npx'); // stdio fields preserved alongside enabled
});

it('reflects an existing enabled:false and keeps it disabled through Save', async () => {
  const upsertSpy = vi.fn().mockResolvedValue({ kind: 'mcp-upsert', originalPath: '/x' });
  const scan = makeCodexMcpScan({ command: 'npx', args: ['-y', 'c7'], enabled: false });
  renderOrganizer({ scan, api: { ...fakeApi, upsertMcpEntry: upsertSpy } });
  fireEvent.click(screen.getByText('context7'));
  await screen.findByTestId('mcp-form');
  const check = screen.getByTestId('mcp-enabled') as HTMLInputElement;
  expect(check.checked).toBe(false); // seeded from config.enabled
  fireEvent.click(screen.getByTestId('mcp-save'));
  await waitFor(() => expect(upsertSpy).toHaveBeenCalled());
  const [, config] = upsertSpy.mock.calls[0];
  expect(config.enabled).toBe(false);
});

it('does NOT show the Enabled checkbox for a Claude MCP item', async () => {
  const scan = makeScanWithMcp({ name: 'context7', scopeId: 'global',
    path: '/Users/x/.claude.json', mcpConfig: { command: 'npx', args: ['-y', 'c7'] } });
  renderOrganizer({ scan, api: fakeApi });
  fireEvent.click(screen.getByText('context7'));
  await screen.findByTestId('mcp-form');
  expect(screen.queryByTestId('mcp-enabled')).not.toBeInTheDocument();
});

// ── Plan 18 review-fix 3: align a contradictory `type` on transport switch ──

it('switching a type:stdio entry to http realigns the contradictory type and keeps url', async () => {
  const upsertSpy = vi.fn().mockResolvedValue({ kind: 'mcp-upsert', originalPath: '/x' });
  const scan = makeScanWithMcp({ name: 'ctx', scopeId: 'global',
    path: '/Users/x/.claude.json', mcpConfig: { command: 'npx', args: ['-y', 'pkg@1'], type: 'stdio' } });
  renderOrganizer({ scan, api: { ...fakeApi, upsertMcpEntry: upsertSpy } });
  fireEvent.click(screen.getByText('ctx'));
  await screen.findByTestId('mcp-form');
  fireEvent.click(screen.getByTestId('mcp-transport-http'));
  const url = await screen.findByTestId('mcp-url') as HTMLInputElement;
  fireEvent.input(url, { target: { value: 'https://mcp.example.com/sse' } });
  fireEvent.click(screen.getByTestId('mcp-save'));
  await waitFor(() => expect(upsertSpy).toHaveBeenCalled());
  const [, config] = upsertSpy.mock.calls[0];
  expect(config.url).toBe('https://mcp.example.com/sse');
  expect(config.type).not.toBe('stdio'); // contradictory stdio type must not survive next to a url
  expect(config.command).toBeUndefined();
});

// ── Plan 19: Add Skill flow ──

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

// ── Plan 24: Paste JSON tab in the Add-MCP pane ──

it('Add-MCP Paste JSON tab previews server names and imports the blob', async () => {
  const calls: string[] = [];
  const importSpy = vi.fn(async (_scopeId: string, json: string) => { calls.push(json); return []; });
  const scan = makeScanWithMcp({ name: 'context7', scopeId: 'global',
    path: '/Users/x/.claude.json', mcpConfig: { command: 'npx' } });
  renderOrganizer({ scan, api: { ...fakeApi, mcpImportJson: importSpy } });
  fireEvent.click(screen.getByTestId('category-mcp'));
  fireEvent.click(screen.getByTestId('mcp-add-button'));
  // Switch the Add pane from the Form tab to the Paste JSON tab.
  fireEvent.click(await screen.findByTestId('mcp-paste-tab'));
  const ta = await screen.findByTestId('mcp-paste-json') as HTMLTextAreaElement;
  fireEvent.input(ta, { target: { value: '{"mcpServers":{"ctx7":{"command":"npx"}}}' } });
  // (a) the live preview lists the parsed server name.
  expect(await screen.findByText(/ctx7/)).toBeInTheDocument();
  // (b) Import calls mcpImportJson with the pasted blob.
  fireEvent.click(screen.getByTestId('mcp-paste-import'));
  await waitFor(() => expect(importSpy).toHaveBeenCalled());
  expect(calls[0]).toContain('ctx7');
});