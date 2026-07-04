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
