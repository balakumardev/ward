import { test, expect, vi } from 'vitest';
import { render, fireEvent, waitFor } from '@solidjs/testing-library';
import { Marketplace } from './Marketplace';
import type { MarketplaceApi } from './Marketplace';
import type { BuiltConfig, InstallTarget, MarketEntry, McpConfig, PolicyVerdict, ScanResult, SkillPreview } from '../api';

const SKILL: MarketEntry = {
  kind: 'skill',
  name: 'brainstorming',
  displayName: 'brainstorming',
  description: 'Explore intent and requirements before building.',
  source: 'marketplace',
  verified: true,
  packages: [],
  remotes: [],
  repoUrl: 'https://raw.githubusercontent.com/acme/agent-skills/main',
  skillPath: 'https://raw.githubusercontent.com/acme/agent-skills/main/skills/brainstorming/SKILL.md',
};

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
    search: vi.fn(async (kind: string) => ({ entries: kind === 'skill' ? [SKILL] : [NOTES] })),
    buildConfig: vi.fn(async (entry, idx, env) => buildConfigMock(entry, idx, env)),
    install: vi.fn(async (_e: MarketEntry, _i: number, targets: InstallTarget[]) => targets.map((t) => ({ target: t, ok: true }))),
    getPolicy: vi.fn(async () => ({ allowlist: [], denylist: [] })),
    checkPolicy: vi.fn(async () => 'noPolicy' as PolicyVerdict),
    previewSkill: vi.fn(async (e: MarketEntry): Promise<SkillPreview> => ({
      name: e.name,
      description: e.description,
      body: `---\nname: ${e.name}\ndescription: ${e.description}\n---\n\n# ${e.name}\n\nSynthetic skill body.\n`,
    })),
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

test('adding a target + Install calls install with the selected targets', async () => {
  const api = makeApi();
  const { findByTestId, getByTestId } = render(() => <Marketplace scan={SCAN} api={api} />);
  fireEvent.click(await findByTestId('market-card'));
  // Wait for the preview to resolve (re-query — the fallback node is swapped).
  await findByTestId('market-preview');
  await waitFor(() => expect(getByTestId('market-preview').textContent).toContain('npx'));

  // Builder defaults to Claude Code + Global (first scope) — just Add it.
  fireEvent.click(getByTestId('market-add-target'));
  await findByTestId('market-target-row');

  const installBtn = getByTestId('market-install') as HTMLButtonElement;
  await waitFor(() => expect(installBtn.disabled).toBe(false));
  fireEvent.click(installBtn);

  await waitFor(() => expect(api.install).toHaveBeenCalledTimes(1));
  const call = (api.install as ReturnType<typeof vi.fn>).mock.calls[0];
  expect(call[0].name).toBe('io.github.acme/notes'); // entry
  expect(call[1]).toBe(0); // packageIndex
  expect(call[2]).toEqual([{ harness: 'claude', scopeId: 'global' }]); // targets
});

test('the target picker adds a chosen harness+project and removes it', async () => {
  const api = makeApi();
  const { findByTestId, getByTestId, queryByTestId } = render(() => <Marketplace scan={SCAN} api={api} />);
  fireEvent.click(await findByTestId('market-card'));
  await findByTestId('market-preview');

  // Pick a non-default pair: Codex CLI + the "ward" project scope, then Add.
  fireEvent.change(getByTestId('market-add-harness'), { target: { value: 'codex' } });
  fireEvent.change(getByTestId('market-add-scope'), { target: { value: '-proj' } });
  fireEvent.click(getByTestId('market-add-target'));

  const row = await findByTestId('market-target-row');
  expect(row.textContent).toContain('Codex CLI');
  expect(row.textContent).toContain('ward');

  // Install passes exactly the chosen target.
  const installBtn = getByTestId('market-install') as HTMLButtonElement;
  await waitFor(() => expect(installBtn.disabled).toBe(false));
  fireEvent.click(installBtn);
  await waitFor(() => expect(api.install).toHaveBeenCalledTimes(1));
  expect((api.install as ReturnType<typeof vi.fn>).mock.calls[0][2]).toEqual([{ harness: 'codex', scopeId: '-proj' }]);

  // Remove it → no rows left, Install disabled again.
  fireEvent.click(getByTestId('market-target-remove'));
  await waitFor(() => expect(queryByTestId('market-target-row')).toBeNull());
  await waitFor(() => expect((getByTestId('market-install') as HTMLButtonElement).disabled).toBe(true));
});

test('Skills tab lists skill cards from the catalog search', async () => {
  const api = makeApi();
  const { getByTestId, findByTestId, getByText } = render(() => <Marketplace scan={SCAN} api={api} />);
  fireEvent.click(getByTestId('market-tab-skills'));
  await findByTestId('market-card');
  getByText('brainstorming', { selector: '.mkt-card-name' });
  getByText('Explore intent and requirements before building.');
  expect(api.search).toHaveBeenCalledWith('skill', '');
});

test('selecting a skill shows its SKILL.md preview (approval bound to content)', async () => {
  const api = makeApi();
  const { getByTestId, findByTestId } = render(() => <Marketplace scan={SCAN} api={api} />);
  fireEvent.click(getByTestId('market-tab-skills'));
  fireEvent.click(await findByTestId('market-card'));
  await findByTestId('market-detail');
  // The real SKILL.md body is fetched + rendered before install.
  await findByTestId('market-preview');
  await waitFor(() => expect(getByTestId('market-preview').textContent).toContain('# brainstorming'));
  expect(api.previewSkill).toHaveBeenCalled();
  // No MCP-only affordances for a skill entry.
  expect(() => getByTestId('market-pkg-pick')).toThrow();
  expect(() => getByTestId('market-policy-verdict')).toThrow();
});

test('installing a skill calls install with the skill entry + selected targets', async () => {
  const api = makeApi();
  const { getByTestId, findByTestId } = render(() => <Marketplace scan={SCAN} api={api} />);
  fireEvent.click(getByTestId('market-tab-skills'));
  fireEvent.click(await findByTestId('market-card'));
  await findByTestId('market-preview');

  // Builder defaults to Claude Code + Global — add that target.
  fireEvent.click(getByTestId('market-add-target'));
  await findByTestId('market-target-row');
  const installBtn = getByTestId('market-install') as HTMLButtonElement;
  await waitFor(() => expect(installBtn.disabled).toBe(false));
  fireEvent.click(installBtn);

  await waitFor(() => expect(api.install).toHaveBeenCalledTimes(1));
  const call = (api.install as ReturnType<typeof vi.fn>).mock.calls[0];
  expect(call[0].kind).toBe('skill');
  expect(call[0].name).toBe('brainstorming'); // entry
  expect(call[2]).toEqual([{ harness: 'claude', scopeId: 'global' }]); // targets
});

test('a policy denial disables Install and shows the block note', async () => {
  const api = makeApi({ checkPolicy: vi.fn(async () => 'denied' as PolicyVerdict) });
  const { findByTestId, getByTestId } = render(() => <Marketplace scan={SCAN} api={api} />);
  fireEvent.click(await findByTestId('market-card'));
  await findByTestId('market-deny-note');
  // Even with a target added, a denied verdict keeps Install disabled.
  fireEvent.click(getByTestId('market-add-target'));
  await findByTestId('market-target-row');
  const installBtn = getByTestId('market-install') as HTMLButtonElement;
  await waitFor(() => expect(installBtn.disabled).toBe(true));
});

test('duplicate-named entries (different versions) select independently, not together', async () => {
  // The registry lists the SAME server once per published version. The backend
  // dedupes, but the UI must ALSO key selection uniquely so two same-named rows
  // never light up together ("click one → all selected"). This guards against
  // duplicate-named entries reaching the list from any source.
  const V1: MarketEntry = { ...NOTES, version: '1.0.0' };
  const V2: MarketEntry = { ...NOTES, version: '2.1.0' };
  const api = makeApi({ search: vi.fn(async () => ({ entries: [V1, V2] })) });
  const { findAllByTestId } = render(() => <Marketplace scan={SCAN} api={api} />);
  const cards = await findAllByTestId('market-card');
  expect(cards.length).toBe(2);

  // Click the first card → exactly ONE card is active.
  fireEvent.click(cards[0]);
  await waitFor(() => expect(cards.filter((c) => c.classList.contains('active')).length).toBe(1));
  expect(cards[0].classList.contains('active')).toBe(true);
  expect(cards[1].classList.contains('active')).toBe(false);

  // Click the second → selection moves; still exactly one active.
  fireEvent.click(cards[1]);
  await waitFor(() => expect(cards[1].classList.contains('active')).toBe(true));
  expect(cards[0].classList.contains('active')).toBe(false);
  expect(cards.filter((c) => c.classList.contains('active')).length).toBe(1);
});
