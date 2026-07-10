import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from 'solid-js';
import type { ComponentCounts, PluginEntry, PluginScan, ScanResult } from '../api';
import '../styles/plugins.css';

/** Plan 28 — Plugins mode.
 *
 * Claude Code plugins are marketplace-distributed bundles (commands / agents /
 * skills / hooks / MCP + LSP servers). This mode lets the user browse the
 * marketplaces Claude Code knows about and install a plugin into a scope. The
 * Ward differentiator over `claude plugin` is surfaced right on the card: the
 * plugin's **context cost** (always-on + on-invoke tokens — ties into Context
 * Budget) and its **component counts**, so the user sees what a plugin *loads*
 * before installing it.
 *
 * Ward can never trigger a Claude Code reload itself, so a successful install
 * ends in a toast pointing at `/reload-plugins`. Install/uninstall shell out to
 * the `claude` CLI; when it isn't on PATH those actions are disabled (a banner
 * explains) — enable/disable (a later task) is a pure settings.json flip and
 * stays available. WKWebView's `window.confirm()` is a silent no-op, so every
 * destructive/mutating action is gated by a real in-app modal.
 *
 * Class-based (`plugins.css`); no inline styling drives interactive state.
 */

/** The api surface the Plugins mode needs. App.tsx wires these to `api.*`
 *  (install / marketplaceAdd return a fresh PluginScan so the UI reflects the
 *  new on-disk state in one round-trip); tests pass a lightweight stub. */
export interface PluginsApi {
  scan: () => Promise<PluginScan>;
  install: (plugin: string, marketplace: string, scope: string) => Promise<PluginScan>;
  marketplaceAdd: (src: string, scope: string) => Promise<PluginScan>;
  /** Present for parity with the backend command; the authoritative flag is
   *  `PluginScan.cliAvailable`, which the mode reads from the scan directly. */
  cliAvailable?: () => Promise<boolean>;
}

/** Install / add scopes accepted by `claude plugin … --scope <scope>`
 *  (see `plugins/cli.rs`). */
const SCOPES = [
  { id: 'user', label: 'User · ~/.claude' },
  { id: 'project', label: 'Project · .claude' },
  { id: 'local', label: 'Local · .claude/settings.local.json' },
] as const;

const fmt = (n: number) => n.toLocaleString('en-US');

/** True when Ward has catalog metadata for this plugin (tokens / components /
 *  installs). Uncatalogued community plugins have none of it. */
function hasCatalog(p: PluginEntry): boolean {
  return (
    p.alwaysOnTokens !== undefined ||
    p.onInvokeTokens !== undefined ||
    p.uniqueInstalls !== undefined ||
    !!p.componentCounts
  );
}

/** "1 command · 2 skills · 1 MCP server" — non-zero kinds only, correctly
 *  singularised. Empty string when the plugin ships no components. */
function componentSummary(c: ComponentCounts): string {
  const parts: string[] = [];
  const push = (n: number, singular: string, plural = `${singular}s`) => {
    if (n > 0) parts.push(`${n} ${n === 1 ? singular : plural}`);
  };
  push(c.commands, 'command');
  push(c.agents, 'agent');
  push(c.skills, 'skill');
  push(c.hooks, 'hook');
  push(c.mcpServers, 'MCP server');
  push(c.lspServers, 'LSP server');
  return parts.join(' · ');
}

interface DialogState {
  testid: string;
  title: string;
  message: string;
  confirmLabel: string;
  resolve: (ok: boolean) => void;
}

/** Root: capability-gate the whole mode. Codex has no plugin system, so
 *  `pluginsManageable` is false there — mirror the Backups/Sessions gate with a
 *  "not supported" panel (and skip the on-mount scan entirely). */
export function Plugins(props: { scan: ScanResult; api: PluginsApi }) {
  return (
    <Show
      when={props.scan.capabilities.pluginsManageable}
      fallback={
        <div class="plugins-unsupported" data-testid="plugins-unsupported">
          <span>Plugins aren't supported for this harness.</span>
        </div>
      }
    >
      <PluginsDiscover api={props.api} />
    </Show>
  );
}

/** The Discover tab. (A tabbed shell with an Installed tab is added by a later
 *  task — for now Discover is the whole mode body.) */
function PluginsDiscover(props: { api: PluginsApi }) {
  const [scanRes, { mutate }] = createResource(() => props.api.scan());

  const [search, setSearch] = createSignal('');
  const [mpFilter, setMpFilter] = createSignal('all');
  const [scope, setScope] = createSignal<string>('user');
  const [addSrc, setAddSrc] = createSignal('');
  const [toast, setToast] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [dialog, setDialog] = createSignal<DialogState | null>(null);

  // The CLI flag is authoritative once the scan resolves; treat "unknown"
  // (pre-load) as available so the toolbar isn't briefly disabled on first paint.
  const cliAvailable = () => scanRes()?.cliAvailable ?? true;
  const marketplaces = () => scanRes()?.marketplaces ?? [];
  const scopeLabel = (id: string) => SCOPES.find((s) => s.id === id)?.label ?? id;

  const filtered = createMemo<PluginEntry[]>(() => {
    const s = scanRes();
    if (!s) return [];
    const q = search().trim().toLowerCase();
    const mp = mpFilter();
    return s.plugins.filter((p) => {
      if (mp !== 'all' && p.marketplace !== mp) return false;
      if (!q) return true;
      return (
        p.name.toLowerCase().includes(q) ||
        p.displayName.toLowerCase().includes(q) ||
        p.description.toLowerCase().includes(q)
      );
    });
  });

  // ── In-app confirm modal (WKWebView's confirm() silently returns false) ──
  let dialogRef: HTMLDivElement | undefined;
  let okBtnRef: HTMLButtonElement | undefined;

  function askConfirm(opts: Omit<DialogState, 'resolve'>): Promise<boolean> {
    return new Promise((resolve) => setDialog({ ...opts, resolve }));
  }
  function resolveDialog(ok: boolean) {
    const d = dialog();
    if (d) {
      d.resolve(ok);
      setDialog(null);
    }
  }

  // While the modal is open: Escape cancels, Enter confirms, Tab is trapped
  // inside the dialog. Focus starts on the confirm button.
  createEffect(() => {
    if (!dialog()) return;
    queueMicrotask(() => okBtnRef?.focus());
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        resolveDialog(false);
      } else if (e.key === 'Enter') {
        e.preventDefault();
        resolveDialog(true);
      } else if (e.key === 'Tab') {
        const nodes = dialogRef?.querySelectorAll<HTMLElement>(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        );
        if (!nodes || nodes.length === 0) return;
        const first = nodes[0];
        const last = nodes[nodes.length - 1];
        const active = document.activeElement as HTMLElement | null;
        if (e.shiftKey && active === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && active === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

  // ── Mutations ──
  async function requestInstall(p: PluginEntry) {
    const ok = await askConfirm({
      testid: 'plugin-install-confirm',
      title: 'Install plugin',
      message: `Install ${p.displayName} (${p.name}@${p.marketplace}) into ${scopeLabel(scope())}?`,
      confirmLabel: 'Install',
    });
    if (!ok) return;
    setBusy(true);
    try {
      const fresh = await props.api.install(p.name, p.marketplace, scope());
      mutate(fresh);
      setToast('Installed — restart Claude Code or run /reload-plugins to apply.');
    } catch (e) {
      setToast(`Install failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function requestAdd(e?: Event) {
    e?.preventDefault();
    const src = addSrc().trim();
    if (!src) return;
    const ok = await askConfirm({
      testid: 'plugin-add-confirm',
      title: 'Add marketplace',
      message: `Add marketplace source "${src}" (${scopeLabel(scope())})? Ward will run \`claude plugin marketplace add\`.`,
      confirmLabel: 'Add',
    });
    if (!ok) return;
    setBusy(true);
    try {
      const fresh = await props.api.marketplaceAdd(src, scope());
      mutate(fresh);
      setAddSrc('');
      setToast(`Added marketplace source "${src}".`);
    } catch (e2) {
      setToast(`Add failed: ${e2 instanceof Error ? e2.message : String(e2)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="plugins-mode" data-testid="plugins-mode">
      <div class="plugins-discover" data-testid="plugins-discover">
        <header class="plg-toolbar">
          <div class="plg-toolbar-row">
            <label class="plg-field">
              <span class="plg-field-label">Marketplace</span>
              <select
                class="plg-select"
                data-testid="plugins-marketplace-pick"
                value={mpFilter()}
                onChange={(e) => setMpFilter(e.currentTarget.value)}
              >
                <option value="all">All marketplaces</option>
                <For each={marketplaces()}>{(m) => <option value={m.name}>{m.name}</option>}</For>
              </select>
            </label>
            <label class="plg-field">
              <span class="plg-field-label">Install scope</span>
              <select
                class="plg-select"
                data-testid="plugins-scope-pick"
                value={scope()}
                onChange={(e) => setScope(e.currentTarget.value)}
              >
                <For each={SCOPES}>{(s) => <option value={s.id}>{s.label}</option>}</For>
              </select>
            </label>
            <label class="plg-field plg-field-grow">
              <span class="plg-field-label">Search</span>
              <input
                class="plg-search"
                data-testid="plugins-search"
                type="search"
                placeholder="Filter plugins by name or description…"
                value={search()}
                onInput={(e) => setSearch(e.currentTarget.value)}
              />
            </label>
          </div>
          <form class="plg-add" onSubmit={requestAdd}>
            <input
              class="plg-add-input"
              data-testid="plugins-add-src"
              type="text"
              placeholder="Add a marketplace — owner/repo, URL, or local path…"
              value={addSrc()}
              onInput={(e) => setAddSrc(e.currentTarget.value)}
            />
            <button
              type="submit"
              class="plg-add-btn"
              data-testid="plugins-add-btn"
              disabled={!cliAvailable() || busy() || !addSrc().trim()}
            >
              Add marketplace
            </button>
          </form>
        </header>

        <Show when={scanRes() && !cliAvailable()}>
          <div class="plg-cli-banner" data-testid="plugin-cli-banner">
            <span class="plg-cli-icon" aria-hidden="true">⚠</span>
            <span>
              Install/uninstall need the Claude Code CLI on PATH; enable/disable still work.
            </span>
          </div>
        </Show>

        <div class="plg-body">
          <Show
            when={!scanRes.loading}
            fallback={<div class="plg-status" data-testid="plugins-loading">Reading plugins…</div>}
          >
            <Show
              when={!scanRes.error}
              fallback={<div class="plg-status err" data-testid="plugins-error">Failed to read plugins: {String(scanRes.error)}</div>}
            >
              <Show
                when={filtered().length > 0}
                fallback={<div class="plg-status" data-testid="plugins-empty">No plugins match. Add a marketplace or clear the search.</div>}
              >
                <div class="plg-grid">
                  <For each={filtered()}>
                    {(p) => (
                      <article class="plg-card" data-testid="plugin-card">
                        <div class="plg-card-top">
                          <span class="plg-card-name">{p.displayName}</span>
                          <span class="plg-source" data-testid="plugin-source" title="Marketplace">{p.marketplace}</span>
                        </div>
                        <div class="plg-card-id">
                          <code>{p.name}</code>
                          <Show when={p.version}><span class="plg-ver">v{p.version}</span></Show>
                        </div>
                        <p class="plg-card-desc">{p.description}</p>

                        <Show
                          when={hasCatalog(p)}
                          fallback={
                            <div class="plg-meta plg-meta-none" data-testid="plugin-uncatalogued">
                              <span>No catalog metadata — token &amp; component data unavailable.</span>
                            </div>
                          }
                        >
                          <div class="plg-meta">
                            <Show when={p.alwaysOnTokens !== undefined || p.onInvokeTokens !== undefined}>
                              <span class="plg-tokens" data-testid="plugin-tokens" title="Context budget cost">
                                <Show when={p.alwaysOnTokens !== undefined}>
                                  <span class="plg-tok-k">always-on</span>
                                  <span class="plg-tok-v">{fmt(p.alwaysOnTokens!)}</span>
                                </Show>
                                <Show when={p.onInvokeTokens !== undefined}>
                                  <span class="plg-tok-k">on-invoke</span>
                                  <span class="plg-tok-v">{fmt(p.onInvokeTokens!)}</span>
                                </Show>
                                <span class="plg-tok-unit">tokens</span>
                              </span>
                            </Show>
                            <Show when={p.componentCounts && componentSummary(p.componentCounts)}>
                              <span class="plg-components" data-testid="plugin-components">
                                {componentSummary(p.componentCounts!)}
                              </span>
                            </Show>
                            <Show when={p.uniqueInstalls !== undefined}>
                              <span class="plg-installs" data-testid="plugin-installs">
                                {fmt(p.uniqueInstalls!)} installs
                              </span>
                            </Show>
                          </div>
                        </Show>

                        <div class="plg-card-foot">
                          <Show
                            when={!p.installed}
                            fallback={
                              <span
                                class="plg-installed"
                                classList={{ off: !p.enabled }}
                                data-testid="plugin-installed"
                              >
                                {p.enabled ? '✓ Installed' : '✓ Installed · disabled'}
                              </span>
                            }
                          >
                            <button
                              class="plg-install-btn"
                              data-testid="plugin-install"
                              disabled={!cliAvailable() || busy()}
                              title={cliAvailable() ? 'Install this plugin' : 'Requires the Claude Code CLI on PATH'}
                              onClick={() => requestInstall(p)}
                            >
                              Install
                            </button>
                          </Show>
                        </div>
                      </article>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
          </Show>
        </div>
      </div>

      {/* Transient notification. Ward can't reload Claude Code itself, so a
          successful install ends here pointing at /reload-plugins. */}
      <Show when={toast()} keyed>
        {(t) => (
          <div class="plg-toast" data-testid="plugin-reload-toast" role="status">
            <span class="plg-toast-msg">{t}</span>
            <button class="plg-toast-x" data-testid="plugin-toast-dismiss" title="Dismiss" onClick={() => setToast(null)}>×</button>
          </div>
        )}
      </Show>

      {/* In-app confirm modal — install & add both route through this. */}
      <Show when={dialog()} keyed>
        {(d) => (
          <div class="modal-overlay" data-testid="plugins-modal-overlay" onClick={() => resolveDialog(false)}>
            <div
              ref={dialogRef}
              class="modal"
              role="dialog"
              aria-modal="true"
              aria-labelledby="plugins-modal-title"
              data-testid={d.testid}
              onClick={(e) => e.stopPropagation()}
            >
              <div class="modal-title" id="plugins-modal-title">{d.title}</div>
              <div class="modal-body">{d.message}</div>
              <div class="modal-actions">
                <button class="btn btn-ghost" data-testid="plugin-confirm-cancel" onClick={() => resolveDialog(false)}>Cancel</button>
                <button ref={okBtnRef} class="btn btn-primary" data-testid="plugin-confirm-ok" onClick={() => resolveDialog(true)}>{d.confirmLabel}</button>
              </div>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}
