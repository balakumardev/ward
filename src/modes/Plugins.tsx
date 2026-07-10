import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from 'solid-js';
import type { ComponentCounts, PluginEntry, PluginScan, RestoreInfo, ScanResult } from '../api';
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
  /** Enable/disable an installed plugin (`plugin_key` = `name@marketplace`).
   *  A surgical single-key flip in settings.json — works even without the CLI —
   *  that returns a `plugin-enable` RestoreInfo so the toast can offer Undo. */
  setEnabled: (pluginKey: string, enabled: boolean) => Promise<RestoreInfo>;
  /** Uninstall an installed plugin from its scope (shells out to `claude`).
   *  Returns a fresh PluginScan reflecting the removal. */
  uninstall: (plugin: string, scope: string) => Promise<PluginScan>;
  /** Update one marketplace (`name`) or every marketplace (`undefined`) from its
   *  source (shells out to `claude`). Returns a fresh PluginScan. */
  marketplaceUpdate: (name?: string) => Promise<PluginScan>;
  /** Reverse a `plugin-enable` flip via the shared restore engine (App injects
   *  the harness). Backs the toast's Undo affordance. */
  restore: (info: RestoreInfo) => Promise<void>;
  /** Present for parity with the backend command; the authoritative flag is
   *  `PluginScan.cliAvailable`, which the mode reads from the scan directly. */
  cliAvailable?: () => Promise<boolean>;
}

/** Plan 28 — every plugin action is fixed to USER scope (`~/.claude`). The
 *  CLI's `--scope project|local` is cwd-relative, but Ward's Plugins mode has no
 *  project/cwd context (a Finder-launched app runs with cwd `/`), and
 *  enable/disable + the scan only ever read/write the user `settings.json`, so
 *  project/local actions would target the wrong place. Project/local support is
 *  a documented follow-up (needs a project picker + `current_dir` + per-scope
 *  settings read/write); the Rust arg builders already accept a scope string, so
 *  it lands without churn. */
const PLUGIN_SCOPE = 'user';
const PLUGIN_SCOPE_LABEL = 'User · ~/.claude';

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
  /** Title tone: 'info' for benign confirms (install / add marketplace),
   *  'danger' for destructive ones (uninstall). Omitted → the shared
   *  `.modal-title` default (warning-orange), so other modals don't regress. */
  tone?: 'info' | 'danger';
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
      <PluginsBody api={props.api} />
    </Show>
  );
}

type Tab = 'discover' | 'installed';

/** The tabbed mode body: a Discover ↔ Installed tab bar over a shared scan
 *  resource. Discover browses/installs from marketplaces; Installed manages the
 *  plugins already on disk (enable/disable · uninstall · update). Both share the
 *  CLI-absent banner, toast, and in-app confirm modal, and every mutating action
 *  runs at the fixed user scope (see `PLUGIN_SCOPE`). */
function PluginsBody(props: { api: PluginsApi }) {
  const [scanRes, { mutate, refetch }] = createResource(() => props.api.scan());

  const [tab, setTab] = createSignal<Tab>('discover');
  const [search, setSearch] = createSignal('');
  const [mpFilter, setMpFilter] = createSignal('all');
  const [addSrc, setAddSrc] = createSignal('');
  const [toast, setToast] = createSignal<string | null>(null);
  // Set alongside the toast only after an enable/disable flip; drives the
  // toast's Undo button. Cleared for every other (non-undoable) action.
  const [undoInfo, setUndoInfo] = createSignal<RestoreInfo | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [dialog, setDialog] = createSignal<DialogState | null>(null);

  // The CLI flag is authoritative once the scan resolves; treat "unknown"
  // (pre-load) as available so the toolbar isn't briefly disabled on first paint.
  const cliAvailable = () => scanRes()?.cliAvailable ?? true;
  const marketplaces = () => scanRes()?.marketplaces ?? [];

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

  // The Installed tab lists on-disk plugins only (independent of the Discover
  // search/marketplace filters), newest catalog order preserved.
  const installedPlugins = createMemo<PluginEntry[]>(() =>
    (scanRes()?.plugins ?? []).filter((p) => p.installed),
  );

  // ── In-app confirm modal (WKWebView's confirm() silently returns false) ──
  let dialogRef: HTMLDivElement | undefined;
  let okBtnRef: HTMLButtonElement | undefined;
  let cancelBtnRef: HTMLButtonElement | undefined;

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

  // While the modal is open: Escape cancels, Enter confirms (unless Cancel holds
  // focus — then Enter cancels, so Tab-to-Cancel + Enter can't silently confirm),
  // Tab is trapped inside the dialog. Focus starts on the confirm button and is
  // restored to whatever triggered the modal once it closes.
  createEffect(() => {
    if (!dialog()) return;
    // Remember the trigger (e.g. the Install/Uninstall button) to restore later.
    const prevFocused = document.activeElement as HTMLElement | null;
    queueMicrotask(() => okBtnRef?.focus());
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        resolveDialog(false);
      } else if (e.key === 'Enter') {
        e.preventDefault();
        // Focus-aware: when Cancel is the active control, Enter must cancel — not
        // confirm — so a Tab-to-Cancel then Enter never installs/uninstalls.
        resolveDialog(document.activeElement !== cancelBtnRef);
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
    onCleanup(() => {
      window.removeEventListener('keydown', onKey);
      // Restore focus to the trigger after close. Guard: a re-render may have
      // removed it from the DOM — never throw on a stale node.
      if (prevFocused && document.contains(prevFocused) && typeof prevFocused.focus === 'function') {
        prevFocused.focus();
      }
    });
  });

  // ── Mutations ──
  async function requestInstall(p: PluginEntry) {
    const ok = await askConfirm({
      testid: 'plugin-install-confirm',
      title: 'Install plugin',
      message: `Install ${p.displayName} (${p.name}@${p.marketplace}) into ${PLUGIN_SCOPE_LABEL}?`,
      confirmLabel: 'Install',
      tone: 'info',
    });
    if (!ok) return;
    setBusy(true);
    try {
      const fresh = await props.api.install(p.name, p.marketplace, PLUGIN_SCOPE);
      mutate(fresh);
      notify('Installed — restart Claude Code or run /reload-plugins to apply.');
    } catch (e) {
      notify(`Install failed: ${e instanceof Error ? e.message : String(e)}`);
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
      message: `Add marketplace source "${src}" (${PLUGIN_SCOPE_LABEL})? Ward will run \`claude plugin marketplace add\`.`,
      confirmLabel: 'Add',
      tone: 'info',
    });
    if (!ok) return;
    setBusy(true);
    try {
      const fresh = await props.api.marketplaceAdd(src, PLUGIN_SCOPE);
      mutate(fresh);
      setAddSrc('');
      notify(`Added marketplace source "${src}".`);
    } catch (e2) {
      notify(`Add failed: ${e2 instanceof Error ? e2.message : String(e2)}`);
    } finally {
      setBusy(false);
    }
  }

  // Show a toast, optionally with an Undo bound to a `plugin-enable` RestoreInfo.
  // Every non-enable action passes no `undo`, which clears any stale Undo button.
  function notify(msg: string, undo: RestoreInfo | null = null) {
    setUndoInfo(undo);
    setToast(msg);
  }

  // ── Installed-tab mutations ──
  // Enable/disable is a pure settings.json flip — it works with or WITHOUT the
  // CLI, so its control is never gated on `cliAvailable`. We re-scan afterward
  // so the row reflects the new on-disk flag, and keep the RestoreInfo for Undo.
  async function toggleEnabled(p: PluginEntry) {
    const next = !p.enabled;
    const key = `${p.name}@${p.marketplace}`;
    setBusy(true);
    try {
      const info = await props.api.setEnabled(key, next);
      await refetch();
      notify(
        `${next ? 'Enabled' : 'Disabled'} ${p.displayName} — restart Claude Code or run /reload-plugins to apply.`,
        info,
      );
    } catch (e) {
      notify(`${next ? 'Enable' : 'Disable'} failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function doUndo() {
    const info = undoInfo();
    if (!info) return;
    setBusy(true);
    try {
      await props.api.restore(info);
      await refetch();
      notify('Reverted the enable/disable change.');
    } catch (e) {
      notify(`Undo failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  // Uninstall shells out to `claude`, so it's CLI-gated and confirmed in-app
  // (WKWebView's confirm() is a no-op). Always targets the fixed user scope
  // (see `PLUGIN_SCOPE`) — Ward's Plugins mode has no project/cwd context.
  async function requestUninstall(p: PluginEntry) {
    const ok = await askConfirm({
      testid: 'plugin-uninstall-confirm',
      title: 'Uninstall plugin',
      message: `Uninstall ${p.displayName} (${p.name}@${p.marketplace}) from ${PLUGIN_SCOPE_LABEL}? Ward will run \`claude plugin uninstall\`.`,
      confirmLabel: 'Uninstall',
      tone: 'danger',
    });
    if (!ok) return;
    setBusy(true);
    try {
      const fresh = await props.api.uninstall(p.name, PLUGIN_SCOPE);
      mutate(fresh);
      notify(`Uninstalled ${p.displayName} — restart Claude Code or run /reload-plugins to apply.`);
    } catch (e) {
      notify(`Uninstall failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  // Update refreshes the plugin's marketplace from source (CLI-backed). Not
  // destructive, so no confirm modal — just the CLI gate.
  async function requestUpdate(p: PluginEntry) {
    setBusy(true);
    try {
      const fresh = await props.api.marketplaceUpdate(p.marketplace);
      mutate(fresh);
      notify(`Updated ${p.marketplace} — restart Claude Code or run /reload-plugins to apply.`);
    } catch (e) {
      notify(`Update failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="plugins-mode" data-testid="plugins-mode">
      {/* Only the active tabpanel is mounted, so the tabs carry NO
          `aria-controls` (it would dangle at an id that isn't in the DOM). The
          a11y link runs the other way instead: each panel's `aria-labelledby`
          points back at its always-present tab button. */}
      <div class="plg-tabs" data-testid="plugins-tabs" role="tablist" aria-label="Plugins views">
        <button
          type="button"
          role="tab"
          id="plg-tab-discover"
          class="plg-tab"
          classList={{ active: tab() === 'discover' }}
          data-testid="plugins-tab-discover"
          aria-selected={tab() === 'discover' ? 'true' : 'false'}
          onClick={() => setTab('discover')}
        >
          Discover
        </button>
        <button
          type="button"
          role="tab"
          id="plg-tab-installed"
          class="plg-tab"
          classList={{ active: tab() === 'installed' }}
          data-testid="plugins-tab-installed"
          aria-selected={tab() === 'installed' ? 'true' : 'false'}
          onClick={() => setTab('installed')}
        >
          <span>Installed</span>
          <span class="plg-tab-count" data-testid="plugins-installed-count">{installedPlugins().length}</span>
        </button>
      </div>

      {/* Install/uninstall/update shell out to `claude`; enable/disable don't.
          Shown on both tabs since either can attempt a CLI-backed action. */}
      <Show when={scanRes() && !cliAvailable()}>
        <div class="plg-cli-banner" data-testid="plugin-cli-banner">
          <span class="plg-cli-icon" aria-hidden="true">⚠</span>
          <span>
            Install/uninstall/update need the Claude Code CLI on PATH; enable/disable still work.
          </span>
        </div>
      </Show>

      <Show when={tab() === 'discover'}>
      <div
        class="plugins-discover"
        data-testid="plugins-discover"
        role="tabpanel"
        id="plg-panel-discover"
        aria-labelledby="plg-tab-discover"
      >
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
          <p class="plg-scope-note" data-testid="plugins-scope-note">
            Install, add, and uninstall are managed at user scope · <code>~/.claude</code>
          </p>
        </header>

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
      </Show>

      <Show when={tab() === 'installed'}>
        <div
          class="plg-installed-view"
          data-testid="plugins-installed"
          role="tabpanel"
          id="plg-panel-installed"
          aria-labelledby="plg-tab-installed"
        >
          <Show
            when={!scanRes.error}
            fallback={<div class="plg-status err" data-testid="plugins-installed-error">Failed to read plugins: {String(scanRes.error)}</div>}
          >
            <Show
              when={scanRes()}
              fallback={<div class="plg-status" data-testid="plugins-installed-loading">Reading plugins…</div>}
            >
              <Show
                when={installedPlugins().length > 0}
                fallback={
                  <div class="plg-status" data-testid="plugins-installed-empty">
                    No plugins installed. Switch to Discover to browse and install from a marketplace.
                  </div>
                }
              >
                <div class="plg-inst-list">
                  <For each={installedPlugins()}>
                    {(p) => (
                      <article class="plg-inst-row" data-testid="plugin-installed-row">
                        <div class="plg-inst-main">
                          <div class="plg-inst-head">
                            <span class="plg-inst-name">{p.displayName}</span>
                            <span class="plg-source" data-testid="plugin-source" title="Marketplace">{p.marketplace}</span>
                            <Show when={p.version}><span class="plg-ver">v{p.version}</span></Show>
                            <span
                              class="plg-inst-state"
                              classList={{ off: !p.enabled }}
                              data-testid="plugin-installed-state"
                            >
                              {p.enabled ? 'Enabled' : 'Disabled'}
                            </span>
                          </div>
                          <div class="plg-inst-sub">
                            <code>{p.name}</code>
                            <Show when={p.componentCounts && componentSummary(p.componentCounts)}>
                              <span class="plg-inst-dot" aria-hidden="true">·</span>
                              <span>{componentSummary(p.componentCounts!)}</span>
                            </Show>
                            <Show when={p.alwaysOnTokens !== undefined}>
                              <span class="plg-inst-dot" aria-hidden="true">·</span>
                              <span><span class="plg-inst-tok">{fmt(p.alwaysOnTokens!)}</span> always-on tokens</span>
                            </Show>
                          </div>
                        </div>
                        <div class="plg-inst-actions">
                          <button
                            type="button"
                            class="plg-toggle"
                            role="switch"
                            aria-checked={p.enabled ? 'true' : 'false'}
                            aria-label={`${p.enabled ? 'Disable' : 'Enable'} ${p.displayName}`}
                            data-testid="plugin-enable-toggle"
                            disabled={busy()}
                            title={p.enabled ? 'Disable plugin' : 'Enable plugin'}
                            onClick={() => toggleEnabled(p)}
                          >
                            <span class="plg-toggle-track"><span class="plg-toggle-knob" /></span>
                          </button>
                          <button
                            type="button"
                            class="plg-inst-btn"
                            data-testid="plugin-update"
                            disabled={!cliAvailable() || busy()}
                            title={cliAvailable() ? `Update the whole ${p.marketplace} marketplace from source (refreshes every plugin in it, not just this one)` : 'Requires the Claude Code CLI on PATH'}
                            onClick={() => requestUpdate(p)}
                          >
                            Update marketplace
                          </button>
                          <button
                            type="button"
                            class="plg-inst-btn plg-inst-btn-danger"
                            data-testid="plugin-uninstall"
                            disabled={!cliAvailable() || busy()}
                            title={cliAvailable() ? 'Uninstall this plugin' : 'Requires the Claude Code CLI on PATH'}
                            onClick={() => requestUninstall(p)}
                          >
                            Uninstall
                          </button>
                        </div>
                      </article>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
          </Show>
        </div>
      </Show>

      {/* Transient notification. Ward can't reload Claude Code itself, so a
          successful install ends here pointing at /reload-plugins. Enable/disable
          also surfaces an Undo bound to the returned RestoreInfo. */}
      <Show when={toast()} keyed>
        {(t) => (
          <div class="plg-toast" data-testid="plugin-reload-toast" role="status">
            <span class="plg-toast-msg">{t}</span>
            <Show when={undoInfo()}>
              <button
                class="plg-toast-undo"
                data-testid="plugin-undo"
                disabled={busy()}
                onClick={doUndo}
              >
                Undo
              </button>
            </Show>
            <button
              class="plg-toast-x"
              data-testid="plugin-toast-dismiss"
              title="Dismiss"
              onClick={() => { setToast(null); setUndoInfo(null); }}
            >
              ×
            </button>
          </div>
        )}
      </Show>

      {/* In-app confirm modal — install, add-marketplace & uninstall all route
          through this (WKWebView's confirm() is a silent no-op). */}
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
              <div
                class="modal-title"
                classList={{ 'is-info': d.tone === 'info', 'is-danger': d.tone === 'danger' }}
                id="plugins-modal-title"
              >{d.title}</div>
              <div class="modal-body">{d.message}</div>
              <div class="modal-actions">
                <button ref={cancelBtnRef} class="btn btn-ghost" data-testid="plugin-confirm-cancel" onClick={() => resolveDialog(false)}>Cancel</button>
                <button ref={okBtnRef} class="btn btn-primary" data-testid="plugin-confirm-ok" onClick={() => resolveDialog(true)}>{d.confirmLabel}</button>
              </div>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}
