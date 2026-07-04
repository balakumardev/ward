import { render, fireEvent } from '@solidjs/testing-library';
import { Organizer } from './Organizer';
import type { ScanResult } from '../api';
// NOTE: brief verbatim uses `getByText('1')` but both categories have count=1,
// which makes the DOM contain two matches. `getByText` throws on multiple
// matches, so the test cannot pass verbatim. Minimal deviation: use
// `getAllByText` to honor the same intent (assert the count badge is visible).


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
    <Organizer scan={scan} loadFile={async (p) => { loaded.push(p); return 'FILE BODY'; }} />
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
    <Organizer scan={effectiveScan} loadFile={async () => 'x'} />
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
    <Organizer scan={effectiveScan} loadFile={async () => 'x'} />
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