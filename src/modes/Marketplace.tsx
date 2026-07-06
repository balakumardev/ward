import { createEffect, createMemo, createResource, createSignal, For, Show } from 'solid-js';
import type {
  BuiltConfig, EnvVar, InstallResult, InstallTarget, MarketEntry, MarketPage,
  McpPolicy, McpConfig, PolicyVerdict, ScanResult,
} from '../api';
import '../styles/marketplace.css';

/** Plan 21 — Marketplace mode (MCP servers).
 *
 * Search the official MCP Registry → preview the exact version-pinned,
 * secret-safe config → run the policy gate → install into any harness × scope
 * via the shared `upsert_mcp_entry` engine (install-once-to-many).
 *
 * The Skills tab is a Plan 22 seam: it renders a clean "coming soon" state,
 * never a broken search. Everything is class-based (`marketplace.css`); no
 * inline styling drives interactive state.
 */

/** The api surface the Marketplace needs. App.tsx wires these to `api.*`
 *  (install also re-scans so the Organizer reflects new servers); tests pass
 *  a lightweight stub. */
export interface MarketplaceApi {
  search: (kind: string, query: string, cursor?: string) => Promise<MarketPage>;
  buildConfig: (entry: MarketEntry, packageIndex: number, envValues: Record<string, string>) => Promise<BuiltConfig>;
  install: (entry: MarketEntry, packageIndex: number, targets: InstallTarget[], envValues: Record<string, string>) => Promise<InstallResult[]>;
  getPolicy: () => Promise<McpPolicy>;
  checkPolicy: (name: string, config: unknown, policy: McpPolicy) => Promise<PolicyVerdict>;
}

/** Static harness axis for the install matrix. Data-driven so a future
 *  `{ id: 'claude-desktop', … }` target slots in without a rewrite (spec §12). */
const INSTALL_HARNESSES = [
  { id: 'claude', label: 'Claude Code' },
  { id: 'codex', label: 'Codex CLI' },
] as const;

const tkey = (harness: string, scopeId: string) => `${harness}::${scopeId}`;

/** The env/header object that will actually be written (stdio `env` or remote
 *  `headers`). Secrets were already omitted upstream by `build_mcp_config`. */
function writtenEnv(config: McpConfig): Record<string, string> {
  return config.env ?? config.headers ?? {};
}

export function Marketplace(props: { scan: ScanResult; api: MarketplaceApi }) {
  const [tab, setTab] = createSignal<'mcp' | 'skills'>('mcp');
  const [query, setQuery] = createSignal('');
  const [submitted, setSubmitted] = createSignal('');
  const [selectedName, setSelectedName] = createSignal<string | null>(null);
  const [pkgIndex, setPkgIndex] = createSignal(0);
  const [envValues, setEnvValues] = createSignal<Record<string, string>>({});
  const [targets, setTargets] = createSignal<Set<string>>(new Set());
  const [results, setResults] = createSignal<InstallResult[] | null>(null);
  const [installing, setInstalling] = createSignal(false);

  // Search resource — re-runs ONLY on submit (Enter / Search button) or tab
  // change, never per keystroke (network is user-triggered, never a poll).
  const [page] = createResource(
    () => ({ t: tab(), q: submitted() }),
    ({ t, q }) => (t === 'mcp' ? props.api.search('mcp', q) : Promise.resolve<MarketPage>({ entries: [] })),
  );

  const entries = createMemo(() => page()?.entries ?? []);
  const selected = createMemo(() => entries().find((e) => e.name === selectedName()) ?? null);

  // Reset per-selection state whenever the chosen server changes.
  createEffect(() => {
    selectedName();
    setPkgIndex(0);
    setEnvValues({});
    setResults(null);
  });

  // Current MCP policy (fetched once, re-checked against each built config).
  const [policy] = createResource(() => props.api.getPolicy());

  // The exact config that will land on disk — powers both the preview and the
  // policy gate. Rebuilds when the selection, package, or a filled env changes.
  const [built] = createResource(
    () => {
      const e = selected();
      return e ? { entry: e, idx: pkgIndex(), env: { ...envValues() } } : null;
    },
    ({ entry, idx, env }) => props.api.buildConfig(entry, idx, env),
  );

  const [verdict] = createResource(
    () => {
      const b = built();
      const p = policy();
      return b && p ? { b, p } : null;
    },
    ({ b, p }) => props.api.checkPolicy(b.name, b.config, p),
  );

  // Env / header vars for the currently-picked package or remote.
  const activeVars = createMemo<EnvVar[]>(() => {
    const e = selected();
    if (!e) return [];
    if (e.packages.length) return e.packages[pkgIndex()]?.env ?? [];
    if (e.remotes.length) return e.remotes[pkgIndex()]?.headers ?? [];
    return [];
  });

  // Options for the package / transport picker.
  const pkgOptions = createMemo(() => {
    const e = selected();
    if (!e) return [];
    if (e.packages.length) {
      return e.packages.map((p, i) => ({ i, label: `${p.registryType} · ${p.identifier}@${p.version} · ${p.transport}` }));
    }
    return e.remotes.map((r, i) => ({ i, label: `remote · ${r.transport} · ${r.url}` }));
  });

  const selectedTargets = createMemo<InstallTarget[]>(() =>
    [...targets()].map((k) => {
      const [harness, scopeId] = k.split('::');
      return { harness, scopeId };
    }),
  );

  const canInstall = createMemo(
    () => !installing() && selectedTargets().length > 0 && !!built() && !built.error && verdict() !== 'denied',
  );

  function doSearch(e?: Event) {
    e?.preventDefault();
    setSubmitted(query());
  }

  function setEnv(name: string, value: string) {
    setEnvValues({ ...envValues(), [name]: value });
  }

  function toggleTarget(harness: string, scopeId: string) {
    const k = tkey(harness, scopeId);
    const next = new Set(targets());
    if (next.has(k)) next.delete(k);
    else next.add(k);
    setTargets(next);
  }

  async function doInstall() {
    const e = selected();
    const ts = selectedTargets();
    if (!e || ts.length === 0) return;
    setInstalling(true);
    try {
      const r = await props.api.install(e, pkgIndex(), ts, { ...envValues() });
      setResults(r);
    } catch (err) {
      // The command returns a per-target vector; a thrown error is an
      // unexpected transport/task failure — surface it as one failed toast.
      setResults(ts.map((t) => ({ target: t, ok: false, error: err instanceof Error ? err.message : String(err) })));
    } finally {
      setInstalling(false);
    }
  }

  return (
    <div class="mkt-shell" data-testid="marketplace-mode">
      <header class="mkt-head">
        <div class="mkt-tabs">
          <button
            data-testid="market-tab-mcp"
            classList={{ 'mkt-tab': true, active: tab() === 'mcp' }}
            onClick={() => setTab('mcp')}
          >
            MCP Servers
          </button>
          <button
            data-testid="market-tab-skills"
            classList={{ 'mkt-tab': true, active: tab() === 'skills' }}
            onClick={() => setTab('skills')}
          >
            Skills
          </button>
        </div>
        <Show when={tab() === 'mcp'}>
          <form class="mkt-searchbar" onSubmit={doSearch}>
            <input
              data-testid="market-search"
              class="mkt-search"
              type="search"
              placeholder="Search the MCP registry…"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
            />
            <button type="submit" class="mkt-search-btn">Search</button>
          </form>
        </Show>
      </header>

      {/* Skills tab — Plan 22 seam. Clean empty state, never a broken search. */}
      <Show when={tab() === 'skills'}>
        <div class="mkt-skills-empty" data-testid="market-skills-empty">
          <div class="mkt-skills-glyph">✦</div>
          <div class="mkt-skills-title">Skills marketplace — coming soon</div>
          <p class="mkt-skills-lede">
            Searching &amp; installing Skills across your harnesses lands in a later update.
            MCP servers are fully available now — switch to the <strong>MCP Servers</strong> tab.
          </p>
        </div>
      </Show>

      <Show when={tab() === 'mcp'}>
        <div class="mkt-body">
          {/* Results list */}
          <aside class="mkt-list">
            <Show
              when={!page.loading}
              fallback={<div class="mkt-loading" data-testid="market-loading">Searching the registry…</div>}
            >
              <Show when={!page.error} fallback={
                <div class="mkt-error" data-testid="market-error">Registry search failed: {String(page.error)}</div>
              }>
                <Show
                  when={entries().length > 0}
                  fallback={<div class="mkt-empty" data-testid="market-no-results">No servers found. Try a different search.</div>}
                >
                  <For each={entries()}>
                    {(e) => (
                      <div
                        data-testid="market-card"
                        classList={{ 'mkt-card': true, active: selectedName() === e.name }}
                        onClick={() => setSelectedName(e.name)}
                      >
                        <div class="mkt-card-top">
                          <span class="mkt-card-name">{e.displayName}</span>
                          <Show when={e.verified}>
                            <span class="mkt-badge ok" data-testid="market-verified" title="Registry-verified">✓ verified</span>
                          </Show>
                        </div>
                        <div class="mkt-card-id">
                          <code>{e.name}</code>
                          <Show when={e.version}><span class="mkt-card-ver">v{e.version}</span></Show>
                        </div>
                        <div class="mkt-card-desc">{e.description}</div>
                      </div>
                    )}
                  </For>
                </Show>
              </Show>
            </Show>
          </aside>

          {/* Detail sheet */}
          <main class="mkt-detail-pane">
            <Show
              when={selected()}
              fallback={<div class="mkt-detail-empty" data-testid="market-detail-empty">Select a server to preview &amp; install it.</div>}
            >
              {(e) => (
                <section class="mkt-detail" data-testid="market-detail">
                  <div class="mkt-detail-head">
                    <div class="mkt-detail-heading">
                      <h2 class="mkt-detail-title">{e().displayName}</h2>
                      <div class="mkt-detail-id">
                        <code>{e().name}</code>
                        <Show when={e().version}><span class="mkt-card-ver">v{e().version}</span></Show>
                      </div>
                    </div>
                    <Show when={e().verified}>
                      <span class="mkt-badge ok">✓ verified</span>
                    </Show>
                  </div>
                  <p class="mkt-detail-desc">{e().description}</p>

                  {/* Package / transport picker */}
                  <div class="mkt-field">
                    <label class="mkt-label">Package</label>
                    <select
                      data-testid="market-pkg-pick"
                      class="mkt-select"
                      value={String(pkgIndex())}
                      onChange={(ev) => setPkgIndex(Number(ev.currentTarget.value))}
                    >
                      <For each={pkgOptions()}>
                        {(o) => <option value={String(o.i)}>{o.label}</option>}
                      </For>
                    </select>
                  </div>

                  {/* Env / header vars */}
                  <Show when={activeVars().length > 0}>
                    <div class="mkt-field">
                      <label class="mkt-label">Environment</label>
                      <div class="mkt-env-list">
                        <For each={activeVars()}>
                          {(v) => (
                            <div data-testid="market-env-row" classList={{ 'mkt-env-row': true, secret: v.isSecret }}>
                              <code class="mkt-env-name">
                                {v.name}
                                <Show when={v.isRequired}><span class="mkt-env-req" title="required">*</span></Show>
                              </code>
                              <Show
                                when={v.isSecret}
                                fallback={
                                  <input
                                    data-testid="market-env-input"
                                    class="mkt-env-input"
                                    type="text"
                                    placeholder={v.isRequired ? 'required value' : 'optional value'}
                                    value={envValues()[v.name] ?? ''}
                                    onInput={(ev) => setEnv(v.name, ev.currentTarget.value)}
                                  />
                                }
                              >
                                <span data-testid="market-env-secret" class="mkt-env-secret">
                                  secret — set this in your environment; Ward never stores it
                                </span>
                              </Show>
                            </div>
                          )}
                        </For>
                      </div>
                    </div>
                  </Show>

                  {/* Exact preview of what lands on disk */}
                  <div class="mkt-field">
                    <label class="mkt-label">Will install</label>
                    <Show
                      when={!built.error}
                      fallback={<div class="mkt-preview error" data-testid="market-preview">{String(built.error)}</div>}
                    >
                      <Show when={built()} fallback={<div class="mkt-preview" data-testid="market-preview">Building preview…</div>}>
                        {(b) => (
                          <div class="mkt-preview" data-testid="market-preview">
                            <div class="mkt-preview-cmd"><code>{b().commandPreview.join(' ')}</code></div>
                            <Show when={Object.keys(writtenEnv(b().config)).length > 0}>
                              <div class="mkt-preview-env">
                                <For each={Object.entries(writtenEnv(b().config))}>
                                  {([k, val]) => <div><code>{k}={val}</code></div>}
                                </For>
                              </div>
                            </Show>
                          </div>
                        )}
                      </Show>
                    </Show>
                  </div>

                  {/* Policy gate */}
                  <div class="mkt-field mkt-policy-field">
                    <label class="mkt-label">MCP policy</label>
                    <Show when={verdict()} fallback={<span class="mkt-badge dim" data-testid="market-policy-verdict">checking…</span>}>
                      {(v) => (
                        <span
                          data-testid="market-policy-verdict"
                          classList={{ 'mkt-badge': true, ok: v() === 'allowed', crit: v() === 'denied', dim: v() === 'noPolicy' }}
                        >
                          {v() === 'allowed' ? 'allowed by policy' : v() === 'denied' ? 'blocked by policy' : 'no policy set'}
                        </span>
                      )}
                    </Show>
                  </div>

                  {/* Install target matrix — harness × scope */}
                  <div class="mkt-field">
                    <label class="mkt-label">Install to</label>
                    <div class="mkt-matrix" data-testid="market-target-matrix">
                      <div class="mkt-matrix-row mkt-matrix-head">
                        <span class="mkt-matrix-corner">Harness \ Scope</span>
                        <For each={props.scan.scopes}>
                          {(s) => <span class="mkt-matrix-scope" data-testid="market-target-scope">{s.label}</span>}
                        </For>
                      </div>
                      <For each={INSTALL_HARNESSES}>
                        {(h) => (
                          <div class="mkt-matrix-row" data-testid={`market-target-${h.id}`}>
                            <span class="mkt-matrix-harness">{h.label}</span>
                            <For each={props.scan.scopes}>
                              {(s) => (
                                <label class="mkt-matrix-cell">
                                  <input
                                    type="checkbox"
                                    data-testid={`market-cell-${h.id}-${s.id}`}
                                    checked={targets().has(tkey(h.id, s.id))}
                                    onChange={() => toggleTarget(h.id, s.id)}
                                  />
                                </label>
                              )}
                            </For>
                          </div>
                        )}
                      </For>
                    </div>
                  </div>

                  {/* Install action + per-target results */}
                  <div class="mkt-actions">
                    <button
                      data-testid="market-install"
                      class="mkt-install-btn"
                      disabled={!canInstall()}
                      onClick={doInstall}
                    >
                      {installing()
                        ? 'Installing…'
                        : `Install to ${selectedTargets().length} target${selectedTargets().length === 1 ? '' : 's'}`}
                    </button>
                    <Show when={verdict() === 'denied'}>
                      <span class="mkt-deny-note" data-testid="market-deny-note">
                        Blocked by your MCP policy — adjust it in the Organizer to install this server.
                      </span>
                    </Show>
                  </div>

                  <Show when={results()}>
                    {(rs) => (
                      <div class="mkt-results" data-testid="market-results">
                        <For each={rs()}>
                          {(r) => (
                            <div data-testid="market-toast" classList={{ 'mkt-toast': true, ok: r.ok, err: !r.ok }}>
                              <span class="mkt-toast-target">{r.target.harness} · {r.target.scopeId}</span>
                              <span class="mkt-toast-status">{r.ok ? '✓ installed' : `✕ ${r.error ?? 'failed'}`}</span>
                            </div>
                          )}
                        </For>
                      </div>
                    )}
                  </Show>
                </section>
              )}
            </Show>
          </main>
        </div>
      </Show>
    </div>
  );
}
