import { test, expect, vi } from 'vitest';
import { render, fireEvent, waitFor } from '@solidjs/testing-library';
import { Marketplace } from './Marketplace';
import type { MarketplaceApi } from './Marketplace';
import type { BuiltConfig, InstallTarget, MarketEntry, McpConfig, PolicyVerdict, ScanResult } from '../api';

const NOTES: MarketEntry = {
  kind: 'mcp',
  name: 'io.github.acme/notes',
  displayName: 'Acme Notes',
  description: 'Read and write notes from your editor.',
  source: 'registry',
  version: '2.1.0',
  verified: true,
  packages: [
    {
      registryType: 'npm',
      identifier: '@acme/notes-mcp',
      version: '2.1.0',
      transport: 'stdio',
      env: [
        { name: 'NOTES_API_KEY', isRequired: true, isSecret: true },
        { name: 'NOTES_REGION', isRequired: false, isSecret: false },
      ],
    },
  ],
  remotes: [],
};

const SCAN: ScanResult = {
  harnessId: 'claude',
  categories: [],
  scopes: [
    { id: 'global', kind: 'global', label: 'Global', root: '/Users/x/.claude' },
    { id: '-proj', kind: 'project', label: 'ward', root: '/Users/x/ward' },
  ],
  items: [],
  capabilities: {
    contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true,
    sessions: true, effective: true, backup: true, mcpEditable: true, skillCreatable: true,
  },
};

/** Realistic mock mirroring the Rust build_mcp_config (npm→npx, secrets omitted). */
function buildConfigMock(entry: MarketEntry, idx: number, env: Record<string, string>): BuiltConfig {
  const pkg = entry.packages[idx];
  const command = pkg.registryType === 'npm' ? 'npx' : 'uvx';
  const args = pkg.registryType === 'npm' ? ['-y', `${pkg.identifier}@${pkg.version}`] : [`${pkg.identifier}==${pkg.version}`];
  const config: McpConfig = { command, args };
  const e: Record<string, string> = {};
  for (const v of pkg.env) if (!v.isSecret && env[v.name]) e[v.name] = env[v.name];
  if (Object.keys(e).length) config.env = e;
  return { name: 'notes', config, commandPreview: [command, ...args], env: pkg.env };
}

function makeApi(over: Partial<MarketplaceApi> = {}): MarketplaceApi {
  return {
    search: vi.fn(async () => ({ entries: [NOTES] })),
    buildConfig: vi.fn(async (entry, idx, env) => buildConfigMock(entry, idx, env)),
    install: vi.fn(async (_e: MarketEntry, _i: number, targets: InstallTarget[]) => targets.map((t) => ({ target: t, ok: true }))),
    getPolicy: vi.fn(async () => ({ allowlist: [], denylist: [] })),
    checkPolicy: vi.fn(async () => 'noPolicy' as PolicyVerdict),
    ...over,
  };
}

test('renders cards from the registry search', async () => {
  const api = makeApi();
  const { findByTestId, getByText } = render(() => <Marketplace scan={SCAN} api={api} />);
  await findByTestId('market-card');
  getByText('Acme Notes');
  expect(api.search).toHaveBeenCalledWith('mcp', '');
});

test('selecting a card shows the exact command preview + policy verdict', async () => {
  const api = makeApi();
  const { findByTestId, getByTestId, findByText } = render(() => <Marketplace scan={SCAN} api={api} />);
  fireEvent.click(await findByTestId('market-card'));
  // The <Show> swaps the preview node when built() resolves, so re-query
  // inside waitFor rather than holding the (soon-detached) fallback node.
  await findByTestId('market-preview');
  await waitFor(() => expect(getByTestId('market-preview').textContent).toContain('npx -y @acme/notes-mcp@2.1.0'));
  // Policy verdict renders and resolves to the mock's "no policy set".
  await findByText('no policy set');
  expect(getByTestId('market-policy-verdict')).toBeTruthy();
});

test('a secret env var renders read-only (no typeable field), non-secret is editable', async () => {
  const api = makeApi();
  const { findByTestId, getAllByTestId, queryAllByTestId } = render(() => <Marketplace scan={SCAN} api={api} />);
  fireEvent.click(await findByTestId('market-card'));
  await findByTestId('market-detail');
  // Exactly one secret note (NOTES_API_KEY), and exactly one editable input (NOTES_REGION).
  await waitFor(() => expect(queryAllByTestId('market-env-secret').length).toBe(1));
  expect(getAllByTestId('market-env-row').length).toBe(2);
  expect(getAllByTestId('market-env-input').length).toBe(1);
  // The secret's row carries no input element.
  const secretNote = getAllByTestId('market-env-secret')[0];
  const secretRow = secretNote.closest('[data-testid="market-env-row"]')!;
  expect(secretRow.querySelector('input')).toBeNull();
});

test('toggling a target + Install calls install with the selected targets', async () => {
  const api = makeApi();
  const { findByTestId, getByTestId } = render(() => <Marketplace scan={SCAN} api={api} />);
  fireEvent.click(await findByTestId('market-card'));
  // Wait for the preview to resolve (re-query — the fallback node is swapped).
  await findByTestId('market-preview');
  await waitFor(() => expect(getByTestId('market-preview').textContent).toContain('npx'));

  // Check the Claude × global cell.
  fireEvent.click(getByTestId('market-cell-claude-global'));

  const installBtn = getByTestId('market-install') as HTMLButtonElement;
  await waitFor(() => expect(installBtn.disabled).toBe(false));
  fireEvent.click(installBtn);

  await waitFor(() => expect(api.install).toHaveBeenCalledTimes(1));
  const call = (api.install as ReturnType<typeof vi.fn>).mock.calls[0];
  expect(call[0].name).toBe('io.github.acme/notes'); // entry
  expect(call[1]).toBe(0); // packageIndex
  expect(call[2]).toEqual([{ harness: 'claude', scopeId: 'global' }]); // targets
});

test('Skills tab shows a coming-soon state, not a broken search', async () => {
  const api = makeApi();
  const { getByTestId, findByTestId } = render(() => <Marketplace scan={SCAN} api={api} />);
  fireEvent.click(getByTestId('market-tab-skills'));
  await findByTestId('market-skills-empty');
});

test('a policy denial disables Install and shows the block note', async () => {
  const api = makeApi({ checkPolicy: vi.fn(async () => 'denied' as PolicyVerdict) });
  const { findByTestId, getByTestId } = render(() => <Marketplace scan={SCAN} api={api} />);
  fireEvent.click(await findByTestId('market-card'));
  await findByTestId('market-deny-note');
  // Even with a target checked, a denied verdict keeps Install disabled.
  fireEvent.click(getByTestId('market-cell-claude-global'));
  const installBtn = getByTestId('market-install') as HTMLButtonElement;
  await waitFor(() => expect(installBtn.disabled).toBe(true));
});
