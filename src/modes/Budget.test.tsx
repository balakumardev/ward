import { vi, test, expect, beforeEach, afterEach } from 'vitest';
import { render, waitFor, cleanup } from '@solidjs/testing-library';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { Budget } from './Budget';
import type { BudgetBreakdown, ScanResult, Scope } from '../api';

let originalInternals: unknown;
beforeEach(() => {
  invoke.mockReset();
  document.body.innerHTML = '';
  originalInternals = (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
});
afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = originalInternals;
});

const scan: ScanResult = {
  harnessId: 'claude',
  categories: [],
  scopes: [{ id: 'global', kind: 'global', label: 'Global (~/.claude)', root: '/Users/x/.claude' }],
  items: [],
  capabilities: { contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true, sessions: true, effective: true, backup: true },
};
const scope: Scope = { id: 'global', kind: 'global', label: 'Global (~/.claude)', root: '/Users/x/.claude' };

/** A breakdown where skills are metadata always-on (tiny) but their
 *  bodies + MCP schemas are deferred — the exact split the fix produces. */
const breakdown: BudgetBreakdown = {
  systemLoaded: 18000,
  outputStyle: 0,
  systemDeferred: 7000,
  mcpSchemas: 6200,
  mcpToolNames: 120,
  claudemd: 1900,
  claudeMdFiles: [{ path: '/Users/x/.claude/CLAUDE.md', name: 'CLAUDE.md', tokens: 1800, measured: false }],
  skillListing: 2000,
  skillListingRaw: 6238,
  skillBoilerplate: 400,
  agentListing: 180,
  alwaysLoadedItems: [{ category: 'memory', name: 'MEMORY.md', tokens: 900, measured: false }],
  metadataItems: [{ category: 'skill', name: 'brainstorming', tokens: 42, measured: false }],
  deferredItems: [
    { category: 'skill', name: 'brainstorming', tokens: 3800, measured: false },
    { category: 'rule', name: 'python-paths', tokens: 410, measured: false },
  ],
  deferredTotal: 7000 + 6200 + 3800 + 410,
  autocompactBuffer: 13000,
  maxOutput: 32000,
  warningThreshold: 20000,
  measured: false,
  used: 18000 + 120 + 1900 + 2000 + 400 + 180 + 900,
  contextLimit: 200000,
};

test('Budget separates always-on used from on-invoke deferred', async () => {
  invoke.mockResolvedValue(breakdown);
  const { getByTestId } = render(() => <Budget scan={scan} scope={scope} />);

  // Meter fills toward the always-on `used`, NOT the deferred bodies.
  const used = await waitFor(() => getByTestId('budget-used'));
  expect(used.textContent).toContain('23,500'); // 18000+120+1900+2000+400+180+900

  // The on-invoke deferred panel renders with its own total.
  expect(getByTestId('budget-deferred-total').textContent).toContain(
    (breakdown.deferredTotal).toLocaleString(),
  );
  // MCP schemas surface in the DEFERRED panel, not the always-on rows.
  expect(getByTestId('budget-row-deferred-mcp-schemas')).toBeTruthy();
  // A skill appears BOTH as tiny always-on metadata and a large deferred body.
  expect(getByTestId('budget-group-meta-skill')).toBeTruthy();
  expect(getByTestId('budget-group-def-skill')).toBeTruthy();
  // The always-on breakdown carries the skill+command listing row.
  expect(getByTestId('budget-row-system-skill-listing')).toBeTruthy();
});

test('Budget shows the deferred figure in the meter legend', async () => {
  invoke.mockResolvedValue(breakdown);
  const { getByTestId } = render(() => <Budget scan={scan} scope={scope} />);
  const note = await waitFor(() => getByTestId('budget-deferred-note'));
  expect(note.textContent).toContain('deferred');
});

test('Budget flags a capped skill listing', async () => {
  invoke.mockResolvedValue(breakdown);
  const { getByTestId } = render(() => <Budget scan={scan} scope={scope} />);
  const cap = await waitFor(() => getByTestId('budget-listing-capped'));
  expect(cap.textContent).toContain('6,238'); // raw
  expect(cap.textContent).toContain('2,000'); // capped
});
