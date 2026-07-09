import { createEffect, createMemo, createResource, createSignal, For, Show } from 'solid-js';
import type {
  BuiltConfig, EnvVar, InstallResult, InstallTarget, MarketEntry, MarketPage,
  McpPolicy, McpConfig, PolicyVerdict, ScanResult, SkillPreview,
} from '../api';
import '../styles/marketplace.css';

/** Plan 21 + 22 — Marketplace mode (MCP servers + Skills).
 *
 * Two data sources through ONE set of components. **MCP Servers**: search the
 * official MCP Registry → preview the exact version-pinned, secret-safe config →
 * policy gate → install into any harness × scope. **Skills**: search the curated
 * Claude skill marketplaces → preview the real `SKILL.md` (approval bound to
 * content) → install into any harness × scope. Both fan out through the shared
 * upsert engines (install-once-to-many). Class-based (`marketplace.css`); no
 * inline styling drives interactive state.
 */

/** The api surface the Marketplace needs. App.tsx wires these to `api.*`
 *  (install also re-scans so the Organizer reflects new servers/skills); tests
 *  pass a lightweight stub. */
export interface MarketplaceApi {
  search: (kind: string, query: string, cursor?: string) => Promise<MarketPage>;
  buildConfig: (entry: MarketEntry, packageIndex: number, envValues: Record<string, string>) => Promise<BuiltConfig>;
  install: (entry: MarketEntry, packageIndex: number, targets: InstallTarget[], envValues: Record<string, string>) => Promise<InstallResult[]>;
  getPolicy: () => Promise<McpPolicy>;
  checkPolicy: (name: string, config: unknown, policy: McpPolicy) => Promise<PolicyVerdict>;
  previewSkill: (entry: MarketEntry) => Promise<SkillPreview>;
}

/** Static harness list for the install target picker. Data-driven so a future
 *  `{ id: 'claude-desktop', … }` target slots in without a rewrite (spec §12). */
const INSTALL_HARNESSES = [
  { id: 'claude', label: 'Claude Code' },
  { id: 'codex', label: 'Codex CLI' },
] as const;

const tkey = (harness: string, scopeId: string) => `${harness}::${scopeId}`;

/** Unique row identity for a market entry. The registry lists the SAME server
 *  once per published version, so `name` alone is NOT unique — keying selection
 *  on it makes every same-named row light up together ("click one → all
 *  selected"). The backend now dedupes by name, but we key on
 *  `source::name::version` so the UI is robust even if duplicate-named entries
 *  ever reach it (a second source, a paginated re-append, a dedupe regression). */
const ekey = (e: MarketEntry) => `${e.source}::${e.name}::${e.version ?? ''}`;

/** The env/header object that will actually be written (stdio `env` or remote
 *  `headers`). Secrets were already omitted upstream by `build_mcp_config`. */
function writtenEnv(config: McpConfig): Record<string, string> {
  return config.env ?? config.headers ?? {};
}

/** Guard the "View source" href. `repoUrl` comes from untrusted community
 *  sources (Glama `repository.url`, Smithery `homepage`), so a `javascript:` /
 *  `data:` URL would execute in Ward's IPC-capable webview. Only http(s) URLs
 *  become a link; anything else falls through to the shape note. */
function safeHttpUrl(u: string | undefined): string | undefined {
  return u && /^https?:\/\//i.test(u.trim()) ? u.trim() : undefined;
}

export function Marketplace(props: { scan: ScanResult; api: MarketplaceApi }) {
  const [tab, setTab] = createSignal<'mcp' | 'skills'>('mcp');
  const [query, setQuery] = createSignal('');
  const [submitted, setSubmitted] = createSignal('');
  const [selectedKey, setSelectedKey] = createSignal<string | null>(null);
  const [pkgIndex, setPkgIndex] = createSignal(0);
  const [envValues, setEnvValues] = createSignal<Record<string, string>>({});
  const [targets, setTargets] = createSignal<Set<string>>(new Set());
  const [results, setResults] = createSignal<InstallResult[] | null>(null);
  const [installing, setInstalling] = createSignal(false);
  // Pending harness/scope for the "add target" builder (defaults: Claude Code + Global).
  const [pendingHarness, setPendingHarness] = createSignal<string>(INSTALL_HARNESSES[0].id);
  const [pendingScope, setPendingScope] = createSignal<string>(props.scan.scopes[0]?.id ?? '');

  const searchKind = () => (tab() === 'skills' ? 'skill' : 'mcp');

  // Search resource — re-runs ONLY on submit (Enter / Search button) or tab
  // change, never per keystroke (network is user-triggered, never a poll).
  const [page] = createResource(
    () => ({ kind: searchKind(), q: submitted() }),
    ({ kind, q }) => props.api.search(kind, q),
  );

  const entries = createMemo(() => page()?.entries ?? []);
  const selected = createMemo(() => entries().find((e) => ekey(e) === selectedKey()) ?? null);
  const isSkill = createMemo(() => selected()?.kind === 'skill');
  // Only `installable` MCP entries carry packages/remotes Ward can build + install.
  // `discovery`/`container` entries have nothing to build, so the whole install
  // sub-UI (picker/preview/policy) and the `built` resource are gated on this.
  const isInstallable = createMemo(() => selected()?.installShape === 'installable');

  // Reset per-selection state whenever the chosen entry changes.
  createEffect(() => {
    selectedKey();
    setPkgIndex(0);
    setEnvValues({});
    setResults(null);
  });

  // Current MCP policy (fetched once, re-checked against each built config).
  const [policy] = createResource(() => props.api.getPolicy());

  // The exact MCP config that will land on disk — powers both the preview and
  // the policy gate. Only for MCP entries (skills have no packages to build).
  const [built] = createResource(
    () => {
      const e = selected();
      // Build ONLY for an installable MCP entry. Skills have no packages;
      // discovery/container entries have nothing to build — `build_mcp_config`
      // returns Err for them, so we must not call it at all (a pointless
      // erroring IPC that would paint a broken preview + stuck policy gate).
      return e && e.kind !== 'skill' && e.installShape === 'installable'
        ? { entry: e, idx: pkgIndex(), env: { ...envValues() } }
        : null;
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

  // The fetched SKILL.md preview — approval is bound to this content. Only for
  // skill entries; fetched on select.
  const [skillPreview] = createResource(
    () => {
      const e = selected();
      return e && e.kind === 'skill' ? e : null;
    },
    (e) => props.api.previewSkill(e),
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

  const canInstall = createMemo(() => {
    if (installing() || selectedTargets().length === 0) return false;
    const e = selected();
    if (!e) return false;
    // Skills: approval is bound to the fetched SKILL.md — require it loaded.
    if (e.kind === 'skill') return !!skillPreview() && !skillPreview.error;
    // MCP: require a built config that isn't policy-denied.
    return !!built() && !built.error && verdict() !== 'denied';
  });

  function doSearch(e?: Event) {
    e?.preventDefault();
    setSubmitted(query());
  }

  /** Switch tabs with a clean slate — a selection/query/targets from one data
   *  source must not bleed into the other. */
  function changeTab(t: 'mcp' | 'skills') {
    if (t === tab()) return;
    setTab(t);
    setSelectedKey(null);
    setQuery('');
    setSubmitted('');
    setResults(null);
    setTargets(new Set<string>());
  }

  function setEnv(name: string, value: string) {
    setEnvValues({ ...envValues(), [name]: value });
  }

  /** Add one harness×scope target (idempotent — the Set dedupes). */
  function addTarget(harness: string, scopeId: string) {
    if (!scopeId) return;
    const next = new Set(targets());
    next.add(tkey(harness, scopeId));
    setTargets(next);
  }

  /** Remove one target by its `harness::scope` key. */
  function removeTargetKey(k: string) {
    const next = new Set(targets());
    next.delete(k);
    setTargets(next);
  }

  const harnessLabel = (id: string) => INSTALL_HARNESSES.find((h) => h.id === id)?.label ?? id;
  const scopeLabel = (id: string) => props.scan.scopes.find((s) => s.id === id)?.label ?? id;

  async function doInstall() {
    const e = selected();
    const ts = selectedTargets();
    if (!e || ts.length === 0) return;
    setInstalling(true);
    try {
      // Skills ignore package index / env (0 / {}); MCP uses the picked ones.
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
            onClick={() => changeTab('mcp')}
          >
            MCP Servers
          </button>
          <button
            data-testid="market-tab-skills"
            classList={{ 'mkt-tab': true, active: tab() === 'skills' }}
            onClick={() => changeTab('skills')}
          >
            Skills
          </button>
        </div>
        <form class="mkt-searchbar" onSubmit={doSearch}>
          <input
            data-testid="market-search"
            class="mkt-search"
            type="search"
            placeholder={tab() === 'skills' ? 'Search skills…' : 'Search the MCP registry…'}
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
          />
          <button type="submit" class="mkt-search-btn">Search</button>
        </form>
      </header>

      <div class="mkt-body">
        {/* Results list — shared by both tabs. */}
        <aside class="mkt-list">
          <Show
            when={!page.loading}
            fallback={<div class="mkt-loading" data-testid="market-loading">Searching…</div>}
          >
            <Show when={!page.error} fallback={
              <div class="mkt-error" data-testid="market-error">Search failed: {String(page.error)}</div>
            }>
              <Show
                when={entries().length > 0}
                fallback={<div class="mkt-empty" data-testid="market-no-results">
                  {tab() === 'skills' ? 'No skills found. Try a different search.' : 'No servers found. Try a different search.'}
                </div>}
              >
                <For each={entries()}>
                  {(e) => (
                    <div
                      data-testid="market-card"
                      classList={{ 'mkt-card': true, active: selectedKey() === ekey(e) }}
                      onClick={() => setSelectedKey(ekey(e))}
                    >
                      <div class="mkt-card-top">
                        <span class="mkt-card-name">{e.displayName}</span>
                        <Show when={e.verified}>
                          <span class="mkt-badge ok" data-testid="market-verified" title="Verified">✓ verified</span>
                        </Show>
                        <span class="mkt-source" data-testid="market-source" title="Source">{e.source}</span>
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

        {/* Detail sheet — shared shell, kind-branched definition. */}
        <main class="mkt-detail-pane">
          <Show
            when={selected()}
            fallback={<div class="mkt-detail-empty" data-testid="market-detail-empty">
              {tab() === 'skills' ? 'Select a skill to preview & install it.' : 'Select a server to preview & install it.'}
            </div>}
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

                <Show
                  when={isSkill()}
                  fallback={
                    /* MCP install sub-UI — ONLY for installable entries.
                       discovery/container entries have no packages/remotes to
                       build, so they show just name/description + the source
                       link below; rendering this block for them paints a red
                       build-error preview and a stuck "policy: checking…". */
                    <Show when={isInstallable()}>
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
                    </Show>
                  }
                >
                  {/* Skill definition — the real SKILL.md, fetched before install
                      so approval is bound to the content (spec §9.5). */}
                  <div class="mkt-field">
                    <label class="mkt-label">SKILL.md</label>
                    <Show
                      when={!skillPreview.loading}
                      fallback={<div class="mkt-preview" data-testid="market-preview">Fetching SKILL.md…</div>}
                    >
                      <Show
                        when={!skillPreview.error}
                        fallback={<div class="mkt-preview error" data-testid="market-preview">Failed to fetch SKILL.md: {String(skillPreview.error)}</div>}
                      >
                        <Show when={skillPreview()} fallback={<div class="mkt-preview" data-testid="market-preview">Fetching SKILL.md…</div>}>
                          {(p) => (
                            <div class="mkt-preview mkt-skill-preview" data-testid="market-preview">
                              <div class="mkt-skill-meta">
                                <code class="mkt-skill-name">{p().name}</code>
                                <span class="mkt-skill-desc">{p().description}</span>
                              </div>
                              <pre class="mkt-skill-body"><code>{p().body}</code></pre>
                            </div>
                          )}
                        </Show>
                      </Show>
                    </Show>
                  </div>
                  <Show when={selected()?.repoUrl}>
                    <div class="mkt-field mkt-skill-source">
                      <label class="mkt-label">Source</label>
                      <code class="mkt-skill-repo">{selected()!.repoUrl}</code>
                    </div>
                  </Show>
                </Show>

                {/* Install target picker — add harness × scope rows (shared).
                    A per-project column matrix doesn't scale to N projects, so
                    the user builds an explicit list: pick harness + project,
                    Add, repeat. Selection state is still the `targets` Set. */}
                <div class="mkt-field">
                  <label class="mkt-label">Install to</label>
                  <div class="mkt-target-builder">
                    <select
                      data-testid="market-add-harness"
                      class="mkt-select"
                      value={pendingHarness()}
                      onChange={(ev) => setPendingHarness(ev.currentTarget.value)}
                    >
                      <For each={INSTALL_HARNESSES}>
                        {(h) => <option value={h.id}>{h.label}</option>}
                      </For>
                    </select>
                    <select
                      data-testid="market-add-scope"
                      class="mkt-select"
                      value={pendingScope()}
                      onChange={(ev) => setPendingScope(ev.currentTarget.value)}
                    >
                      <For each={props.scan.scopes}>
                        {(s) => <option value={s.id}>{s.label}</option>}
                      </For>
                    </select>
                    <button
                      type="button"
                      data-testid="market-add-target"
                      class="mkt-add-btn"
                      disabled={!pendingScope() || targets().has(tkey(pendingHarness(), pendingScope()))}
                      onClick={() => addTarget(pendingHarness(), pendingScope())}
                    >
                      + Add
                    </button>
                  </div>
                  <Show
                    when={selectedTargets().length > 0}
                    fallback={
                      <div class="mkt-target-empty" data-testid="market-target-empty">
                        No targets yet — pick a harness &amp; project, then Add.
                      </div>
                    }
                  >
                    <div class="mkt-target-list">
                      <For each={selectedTargets()}>
                        {(t) => (
                          <div class="mkt-target-row" data-testid="market-target-row">
                            <span class="mkt-target-label">{harnessLabel(t.harness)} · {scopeLabel(t.scopeId)}</span>
                            <button
                              type="button"
                              class="mkt-target-remove"
                              data-testid="market-target-remove"
                              title="Remove target"
                              onClick={() => removeTargetKey(tkey(t.harness, t.scopeId))}
                            >
                              ×
                            </button>
                          </div>
                        )}
                      </For>
                    </div>
                  </Show>
                </div>

                {/* Install action + per-target results (shared). Skills and
                    `installable` MCP entries get the real Install button;
                    `container`/`discovery` entries can't be installed from Ward,
                    so they offer a "View source" link (or a shape note when no
                    repo is known) instead of a fabricated install. */}
                <div class="mkt-actions">
                  <Show
                    when={isSkill() || selected()?.installShape === 'installable'}
                    fallback={
                      <Show
                        when={safeHttpUrl(selected()?.repoUrl)}
                        fallback={
                          <span class="mkt-shape-note" data-testid="market-shape-note">
                            {selected()?.installShape === 'container'
                              ? 'Container image — install with your container runtime.'
                              : 'Discovery only — no direct install.'}
                          </span>
                        }
                      >
                        {(href) => (
                          <a
                            class="mkt-view-btn"
                            data-testid="market-view"
                            href={href()}
                            target="_blank"
                            rel="noreferrer noopener"
                          >
                            View source ↗
                          </a>
                        )}
                      </Show>
                    }
                  >
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
                    <Show when={!isSkill() && verdict() === 'denied'}>
                      <span class="mkt-deny-note" data-testid="market-deny-note">
                        Blocked by your MCP policy — adjust it in the Organizer to install this server.
                      </span>
                    </Show>
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
    </div>
  );
}
