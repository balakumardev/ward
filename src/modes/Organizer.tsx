import { createMemo, createResource, createSignal, createEffect, For, Show } from 'solid-js';
import type { Destination, HarnessItem, McpPolicy, PolicyVerdict, RestoreInfo, ScanResult } from '../api';

function effectiveBadge(item: HarnessItem): string | null {
  if (!item.effective) return null;
  if (item.effective === 'shadowed') return '🌫 shadowed';
  if (item.effective === 'conflict') return '⚠ conflict';
  if (item.effective === 'ancestor') return '↑ ancestor';
  return null;
}

function policyBadge(v: PolicyVerdict | undefined): { label: string; color: string } | null {
  if (v === 'allowed') return { label: '✓ allowed', color: 'var(--accent)' };
  if (v === 'denied') return { label: '🚫 denied', color: 'var(--danger)' };
  return null;
}

function itemKey(item: HarnessItem): string {
  return `${item.category}::${item.name}::${item.scopeId}::${item.path}`;
}

/** Category → accent colour (drives the dot on category rows and item rows). */
const CAT_COLORS: Record<string, string> = {
  skill: 'var(--cat-skill)', memory: 'var(--cat-memory)', mcp: 'var(--cat-mcp)',
  command: 'var(--cat-command)', agent: 'var(--cat-agent)', plan: 'var(--cat-plan)',
  rule: 'var(--cat-rule)', config: 'var(--cat-config)', hook: 'var(--cat-hook)',
  plugin: 'var(--cat-plugin)', session: 'var(--cat-session)', setting: 'var(--cat-setting)',
};
function catDot(id: string): string { return CAT_COLORS[id] ?? 'var(--text-mute)'; }

function fileName(path: string): string {
  const base = path.split('/').pop() ?? path;
  return base.includes('#') ? base.split('#').pop() ?? base : base;
}

function langTag(item: HarnessItem): string {
  const p = item.path.toLowerCase();
  if (p.endsWith('.md')) return 'markdown';
  if (p.endsWith('.json')) return 'json';
  if (p.endsWith('.toml')) return 'toml';
  if (p.endsWith('.yaml') || p.endsWith('.yml')) return 'yaml';
  if (p.endsWith('.sh')) return 'shell';
  if (p.includes('#')) return 'entry';
  return 'text';
}

/** Markdown-ish files get an Edit / Preview toggle. */
function isMarkdownItem(item: HarnessItem): boolean {
  return item.path.toLowerCase().endsWith('.md')
    || ['memory', 'skill', 'plan', 'command', 'rule', 'agent'].includes(item.category);
}

export interface OrganizerApi {
  listDestinations: (item: HarnessItem) => Promise<Destination[]>;
  moveItem: (item: HarnessItem, destScopeId: string) => Promise<RestoreInfo>;
  deleteItem: (item: HarnessItem) => Promise<RestoreInfo>;
  restore: (info: RestoreInfo) => Promise<void>;
  bulkRestore: (infos: RestoreInfo[]) => Promise<void>;
  saveFile: (path: string, content: string) => Promise<void>;
  bulk: (items: HarnessItem[], op: 'move' | 'delete', destScopeId?: string) => Promise<RestoreInfo[]>;
  // Plan 04 — MCP controls.
  mcpGetDisabled: (projectPath: string) => Promise<string[]>;
  mcpSetDisabled: (projectPath: string, list: string[]) => Promise<RestoreInfo>;
  mcpGetPolicy: () => Promise<McpPolicy>;
}

export function Organizer(props: {
  scan: ScanResult;
  loadFile: (path: string) => Promise<string>;
  api: OrganizerApi;
  canPolicy?: boolean;
  onOpenPolicy?: () => void;
}) {
  const [activeCat, setActiveCat] = createSignal(props.scan.categories[0]?.id ?? '');
  const [detail, setDetail] = createSignal<string>('');
  const [selected, setSelected] = createSignal<string>('');
  const [showEffective, setShowEffective] = createSignal(false);
  const [query, setQuery] = createSignal('');
  const [previewMode, setPreviewMode] = createSignal(false);
  const [destinations, setDestinations] = createSignal<Destination[]>([]);
  const [showMoveMenu, setShowMoveMenu] = createSignal(false);
  const [lastUndo, setLastUndo] = createSignal<RestoreInfo | RestoreInfo[] | null>(null);
  const [dirty, setDirty] = createSignal(false);
  const [statusMsg, setStatusMsg] = createSignal<string>('');
  const [selectedKeys, setSelectedKeys] = createSignal<Set<string>>(new Set());
  const [lastClickKey, setLastClickKey] = createSignal<string>('');
  const [bulkDest, setBulkDest] = createSignal<string>('');

  // Plan 04 — disabled-server state (per project_path) and policy.
  // Disabled list is keyed by absolute project path so toggles stay
  // scoped to the project the user is viewing.
  const [disabledByProject, setDisabledByProject] = createSignal<Record<string, Set<string>>>({});
  const [policyResource] = createResource(() => props.api.mcpGetPolicy());
  // Whenever the policy changes (or the visible items do), recompute
  // every MCP item's verdict. We use the entire `items` array + the
  // policy resource as the effect's tracked deps.
  createEffect(() => {
    const p = policyResource();
    if (!p) return;
    // Recompute against the current items.
    const next: Record<string, PolicyVerdict> = {};
    for (const item of props.scan.items) {
      if (item.category !== 'mcp') continue;
      next[itemKey(item)] = checkPolicyLocal(item.name, ((item.mcpConfig ?? {}) as { command?: string; args?: string[]; url?: string }), p);
    }
    setVerdicts(next);
  });

  // ── Helpers ──

  /** The real on-disk project path for a given scope_id, looked up via
   *  the scan result. Falls back to scope.id for global / unresolved. */
  function projectPathForScope(scopeId: string): string | null {
    if (scopeId === 'global') return null;
    const s = props.scan.scopes.find((sc) => sc.id === scopeId);
    return s?.root ?? null;
  }

  function disabledSetFor(item: HarnessItem): Set<string> {
    if (item.scopeId === 'global') return new Set();
    const proj = projectPathForScope(item.scopeId);
    if (!proj) return new Set();
    return disabledByProject()[proj] ?? new Set();
  }

  function isMcpDisabled(item: HarnessItem): boolean {
    return item.category === 'mcp' && disabledSetFor(item).has(item.name);
  }

  const activeCatLabel = () => props.scan.categories.find((c) => c.id === activeCat())?.label ?? 'items';
  const scopeLabel = (id: string) => props.scan.scopes.find((s) => s.id === id)?.label ?? id;

  const itemsForCat = createMemo(() =>
    props.scan.items.filter((i) => i.category === activeCat())
  );

  const effectiveKeys = createMemo<Set<string>>(() => {
    const s = new Set<string>();
    for (const item of props.scan.items) {
      if (item.effective) { s.add(itemKey(item)); continue; }
      if (item.scopeId !== 'global') { s.add(itemKey(item)); }
    }
    return s;
  });

  const visibleItems = createMemo(() => {
    let list = itemsForCat();
    if (showEffective()) list = list.filter((i) => effectiveKeys().has(itemKey(i)));
    const q = query().trim().toLowerCase();
    if (q) list = list.filter((i) => i.name.toLowerCase().includes(q));
    return list;
  });

  const selectedItem = createMemo<HarnessItem | null>(() => {
    const key = selected();
    if (!key) return null;
    return props.scan.items.find((i) => itemKey(i) === key) ?? null;
  });

  // Per-item policy verdict cache — populated eagerly when an item
  // row is rendered AND the policy resource has resolved. The policy
  // algo runs in-process; no IPC. Stores `undefined` for "not computed
  // yet" (NOT to be confused with `noPolicy` — that's a real verdict).
  const [verdicts, setVerdicts] = createSignal<Record<string, PolicyVerdict>>({});

  function computeAndStoreVerdict(item: HarnessItem): PolicyVerdict {
    const v = computeVerdict(item);
    const k = itemKey(item);
    setVerdicts({ ...verdicts(), [k]: v });
    return v;
  }

  function computeVerdict(item: HarnessItem): PolicyVerdict {
    const policy = policyResource();
    if (!policy) return 'noPolicy';
    const cfg = (item.mcpConfig ?? {}) as { command?: string; args?: string[]; url?: string };
    return checkPolicyLocal(item.name, cfg, policy);
  }

  async function open(item: HarnessItem) {
    setSelected(itemKey(item));
    setShowMoveMenu(false);
    setDirty(false);
    setPreviewMode(false);
    setStatusMsg('');
    setDestinations([]);
    if (item.category === 'mcp') {
      // MCP "items" are entries inside a shared JSON config file (e.g.
      // ~/.claude.json), NOT standalone files. Loading item.path would dump
      // the ENTIRE config file (every server + project + history) into the
      // editor — the "random JSON" problem. Show just this server's config,
      // read-only (saving raw text back would clobber the whole file).
      setDetail(JSON.stringify(item.mcpConfig ?? {}, null, 2));
    } else {
      const body = await props.loadFile(item.path);
      setDetail(body);
    }
    if (item.movable && !item.locked) {
      try {
        const dests = await props.api.listDestinations(item);
        setDestinations(dests);
      } catch (e) {
        setStatusMsg(`destinations error: ${String(e)}`);
      }
    }
    // Plan 04 — load disabled list for this project's scope on first view.
    if (item.category === 'mcp' && item.scopeId !== 'global') {
      const proj = projectPathForScope(item.scopeId);
      if (proj && !(proj in disabledByProject())) {
        try {
          const list = await props.api.mcpGetDisabled(proj);
          setDisabledByProject({ ...disabledByProject(), [proj]: new Set(list) });
        } catch (e) {
          setStatusMsg(`disabled list error: ${String(e)}`);
        }
      }
    }
  }

  function onItemClick(item: HarnessItem, e: MouseEvent) {
    if (e.shiftKey && lastClickKey()) {
      // Extend selection from last click to this one (within visible items).
      const list = visibleItems();
      const a = list.findIndex((i) => itemKey(i) === lastClickKey());
      const b = list.findIndex((i) => itemKey(i) === itemKey(item));
      if (a >= 0 && b >= 0) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        const next = new Set(selectedKeys());
        for (let i = lo; i <= hi; i++) next.add(itemKey(list[i]));
        setSelectedKeys(next);
        setLastClickKey(itemKey(item));
        return;
      }
    } else if (e.metaKey || e.ctrlKey) {
      const next = new Set(selectedKeys());
      if (next.has(itemKey(item))) next.delete(itemKey(item));
      else next.add(itemKey(item));
      setSelectedKeys(next);
      setLastClickKey(itemKey(item));
      return;
    } else {
      setSelectedKeys(new Set<string>());
      setLastClickKey(itemKey(item));
    }
    void open(item);
  }

  // ── Mutations ──

  async function doMove(item: HarnessItem, destScopeId: string) {
    setShowMoveMenu(false);
    try {
      const info = await props.api.moveItem(item, destScopeId);
      setLastUndo(info);
      setStatusMsg(`Moved "${item.name}". Click Undo to reverse.`);
    } catch (e) {
      setStatusMsg(`move failed: ${String(e)}`);
    }
  }

  async function doDelete(item: HarnessItem) {
    if (!confirm(`Delete "${item.name}"?`)) return;
    try {
      const info = await props.api.deleteItem(item);
      setLastUndo(info);
      setStatusMsg(`Deleted "${item.name}". Click Undo to restore.`);
      // Don't clear selected — the undo button lives in the detail
      // pane which is rendered inside <Show when={selectedItem()}>.
      // Clearing here would hide the undo button.
      setDetail('');
      setDestinations([]);
    } catch (e) {
      setStatusMsg(`delete failed: ${String(e)}`);
    }
  }

  async function doUndo() {
    const u = lastUndo();
    if (!u) return;
    try {
      if (Array.isArray(u)) {
        await props.api.bulkRestore(u);
      } else {
        await props.api.restore(u);
      }
      setLastUndo(null);
      setStatusMsg('Undone.');
    } catch (e) {
      setStatusMsg(`undo failed: ${String(e)}`);
    }
  }

  async function doSave() {
    const item = selectedItem();
    if (!item) return;
    try {
      await props.api.saveFile(item.path, detail());
      setDirty(false);
      setStatusMsg(`Saved ${item.path}.`);
    } catch (e) {
      setStatusMsg(`save failed: ${String(e)}`);
    }
  }

  function doRevert() {
    const item = selectedItem();
    if (!item) return;
    void open(item);
  }

  async function doBulk(op: 'move' | 'delete') {
    const items = props.scan.items.filter((i) => selectedKeys().has(itemKey(i)));
    if (items.length < 1) return;
    if (op === 'delete' && !confirm(`Delete ${items.length} items?`)) return;
    if (op === 'move' && !bulkDest()) {
      setStatusMsg('Pick a destination scope for bulk move.');
      return;
    }
    try {
      const infos = await props.api.bulk(items, op, op === 'move' ? bulkDest() : undefined);
      setLastUndo(infos);
      setSelectedKeys(new Set<string>());
      setStatusMsg(`${op === 'move' ? 'Moved' : 'Deleted'} ${items.length} items. Click Undo to reverse.`);
    } catch (e) {
      setStatusMsg(`bulk ${op} failed: ${String(e)}`);
    }
  }

  // Plan 04 — toggle an MCP server's disabled flag for the project.
  async function doToggleMcpDisabled(item: HarnessItem) {
    if (item.category !== 'mcp') return;
    const proj = projectPathForScope(item.scopeId);
    if (!proj) return;
    const current = new Set(disabledByProject()[proj] ?? new Set<string>());
    const wasDisabled = current.has(item.name);
    if (wasDisabled) { current.delete(item.name); } else { current.add(item.name); }
    // Optimistic update.
    setDisabledByProject({ ...disabledByProject(), [proj]: current });
    try {
      const info = await props.api.mcpSetDisabled(proj, Array.from(current));
      setLastUndo(info);
      setStatusMsg(`${wasDisabled ? 'Enabled' : 'Disabled'} "${item.name}". Click Undo to reverse.`);
    } catch (e) {
      // Roll back.
      const rollback = new Set(disabledByProject()[proj] ?? new Set<string>());
      if (wasDisabled) rollback.add(item.name); else rollback.delete(item.name);
      setDisabledByProject({ ...disabledByProject(), [proj]: rollback });
      setStatusMsg(`disabled toggle failed: ${String(e)}`);
    }
  }

  function bulkMoveDestinations(): Destination[] {
    // First selected item's destinations drive the bulk UI (CCO does the
    // same per-item). Caller can pick any scope that's valid for at least
    // one of the selected items.
    for (const item of props.scan.items) {
      if (selectedKeys().has(itemKey(item)) && item.movable && !item.locked) {
        // sync call would be nicer but listDestinations is async; we
        // fall back to scanning the scope list when async result isn't
        // ready.
        const cached = destinations();
        if (cached.length > 0) return cached;
        const dests: Destination[] = props.scan.scopes
          .filter((s) => s.id !== item.scopeId && s.id !== 'global')
          .map((s) => ({ scopeId: s.id, label: s.label, kind: s.kind }));
        const global = props.scan.scopes
          .filter((s) => s.id === 'global')
          .map((s) => ({ scopeId: s.id, label: s.label, kind: s.kind }));
        return [...global, ...dests];
      }
    }
    return [];
  }

  return (
    <div class="org">
      {/* ── Categories ── */}
      <div class="col-cats">
        <div class="micro">Categories</div>
        <For each={props.scan.categories}>
          {(c) => (
            <div
              classList={{ cat: true, active: activeCat() === c.id, zero: c.count === 0 }}
              data-testid={`category-${c.id}`}
              onClick={() => { setActiveCat(c.id); setQuery(''); }}
            >
              <span class="cat-dot" style={{ '--dot': catDot(c.id) }} />
              <span class="cat-label">{c.label}</span>
              <span class="count">{c.count}</span>
            </div>
          )}
        </For>
      </div>

      {/* ── Items ── */}
      <div class="col-items">
        <div class="items-head">
          <div class="search-wrap">
            <span class="ico">⌕</span>
            <input
              class="search"
              data-testid="items-search"
              type="text"
              spellcheck={false}
              placeholder={`Search ${activeCatLabel()}…`}
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
            />
          </div>
          <div class="items-subhead">
            <label class="toggle">
              <input
                type="checkbox"
                data-testid="show-effective-toggle"
                checked={showEffective()}
                onInput={(e) => setShowEffective(e.currentTarget.checked)}
              />
              Show Effective
            </label>
            <Show when={props.canPolicy}>
              <button data-testid="mcp-policy-button" onClick={() => props.onOpenPolicy?.()}
                style={{ 'font-size': '11px', padding: '4px 10px' }}>
                MCP Policy
              </button>
            </Show>
          </div>
        </div>

        <div class="items-scroll">
          <For each={props.scan.scopes}>
            {(scope) => {
              // Only render a scope section when it has items in the active
              // category — otherwise every one of the 50+ scopes emits an
              // empty header, burying the real content under a wall of labels.
              const scopeItems = createMemo(() => visibleItems().filter((i) => i.scopeId === scope.id));
              return (
                <Show when={scopeItems().length > 0}>
                  <div class="scope">
                    <span class="micro">{scope.label}</span>
                    <span class="scope-count">{scopeItems().length}</span>
                    <span class="rule" />
                  </div>
                  <For each={scopeItems()}>
                    {(item) => {
                      const k = itemKey(item);
                      // Eagerly compute verdict for MCP items so the row's
                      // badge appears immediately on render. If the policy
                      // resource hasn't loaded yet, the verdict is `noPolicy`
                      // (matches Rust behavior: empty policy → no-policy).
                      if (item.category === 'mcp' && policyResource()) {
                        if (!(k in verdicts())) {
                          computeAndStoreVerdict(item);
                        }
                      }
                      const disabled = () => isMcpDisabled(item);
                      const verdict = () => verdicts()[k];
                      const badge = () => {
                        const v = verdict();
                        return v ? policyBadge(v) : null;
                      };
                      return (
                        <div
                          classList={{ row: true, selected: selected() === k, checked: selectedKeys().has(k) && selected() !== k }}
                          data-testid="item-row"
                          data-item-name={item.name}
                          data-disabled={disabled() ? 'true' : 'false'}
                          onClick={(e) => onItemClick(item, e)}
                        >
                          <span class="row-dot" style={{ '--dot': catDot(item.category) }} />
                          <Show when={selectedKeys().has(k) && k !== selected()}>
                            <span class="check-mark">☑</span>
                          </Show>
                          <span class="row-name">{item.name}</span>
                          <Show when={item.locked}><span class="row-lock">🔒</span></Show>
                          <span class="row-badges">
                            <Show when={badge()}>
                              {(b) => (
                                <span class="badge" data-testid="policy-badge" style={{ color: b().color }}>{b().label}</span>
                              )}
                            </Show>
                            <Show when={effectiveBadge(item)}>
                              <span class="badge badge-dim">{effectiveBadge(item)}</span>
                            </Show>
                            <Show when={item.category === 'mcp' && item.scopeId !== 'global'}>
                              <button
                                classList={{ 'mcp-toggle': true, off: disabled(), on: !disabled() }}
                                data-testid="mcp-disable-toggle"
                                data-disabled={disabled() ? 'true' : 'false'}
                                onClick={(e) => { e.stopPropagation(); void doToggleMcpDisabled(item); }}
                                title={disabled() ? 'Enable for this project' : 'Disable for this project'}
                              >
                                {disabled() ? '✗ Disabled' : '✓ Enabled'}
                              </button>
                            </Show>
                          </span>
                        </div>
                      );
                    }}
                  </For>
                </Show>
              );
            }}
          </For>
          <Show when={visibleItems().length === 0}>
            <div style={{ padding: '28px 14px', color: 'var(--text-mute)', 'font-size': '12px', 'text-align': 'center' }}>
              No {activeCatLabel().toLowerCase()}{query() ? ` matching “${query()}”` : ''}.
            </div>
          </Show>
        </div>

        <Show when={selectedKeys().size >= 2}>
          <div class="bulk" data-testid="bulk-bar">
            <div class="micro">{selectedKeys().size} selected</div>
            <div class="bulk-row">
              <select value={bulkDest()} onChange={(e) => setBulkDest(e.currentTarget.value)} data-testid="bulk-dest">
                <option value="">Destination…</option>
                <For each={bulkMoveDestinations()}>
                  {(d) => <option value={d.scopeId}>{d.label}</option>}
                </For>
              </select>
              <button data-testid="bulk-move" onClick={() => doBulk('move')}>Move</button>
              <button class="btn-danger" data-testid="bulk-delete" onClick={() => doBulk('delete')}>Delete</button>
            </div>
          </div>
        </Show>
      </div>

      {/* ── Detail / editor ── */}
      <div class="detail">
        <Show when={selectedItem()} fallback={
          <div class="detail-empty">
            <div class="big">◫</div>
            <div>Select an item to view or edit</div>
          </div>
        }>
          {(item) => {
            // Eagerly compute verdict for the selected item so the
            // detail pane's badge appears synchronously.
            const detailItem = item();
            const detailK = itemKey(detailItem);
            if (detailItem.category === 'mcp' && policyResource() && !(detailK in verdicts())) {
              computeAndStoreVerdict(detailItem);
            }
            const detailVerdict = () => verdicts()[detailK];
            const detailBadge = () => {
              const v = detailVerdict();
              return v ? policyBadge(v) : null;
            };
            const md = () => isMarkdownItem(item());
            const isMcp = () => item().category === 'mcp';
            return (
              <div class="rise" style={{ display: 'flex', 'flex-direction': 'column', height: '100%', 'min-height': 0 }}>
                <div class="detail-head">
                  <div class="detail-titlewrap">
                    <div class="detail-title">{item().name}{item().locked ? ' 🔒' : ''}</div>
                    <div class="detail-meta">
                      <span class="chip"><span class="cat-dot" style={{ '--dot': catDot(item().category) }} />{item().category}</span>
                      <span class="chip">{scopeLabel(item().scopeId)}</span>
                      <Show when={detailBadge()}>
                        {(badge) => <span class="badge" data-testid="policy-badge-detail" style={{ color: badge().color }}>{badge().label}</span>}
                      </Show>
                      <span class="chip chip-path" title={item().path}>{item().path}</span>
                    </div>
                  </div>
                  <div class="toolbar">
                    <Show when={destinations().length > 0}>
                      <div class="menu-wrap">
                        <button class="btn" data-testid="move-btn" onClick={() => setShowMoveMenu(!showMoveMenu())}>
                          Move ▾
                        </button>
                        <Show when={showMoveMenu()}>
                          <div class="menu" data-testid="move-menu">
                            <div class="menu-title micro">Move to scope</div>
                            <For each={destinations()}>
                              {(d) => (
                                <div class="menu-item" data-testid="move-dest" data-scope-id={d.scopeId}
                                  onClick={() => doMove(item(), d.scopeId)}>
                                  {d.label}
                                </div>
                              )}
                            </For>
                          </div>
                        </Show>
                      </div>
                    </Show>
                    <Show when={item().deletable && !item().locked}>
                      <button class="btn btn-danger" data-testid="delete-btn" onClick={() => doDelete(item())}>Delete</button>
                    </Show>
                    <Show when={lastUndo() !== null}>
                      <button class="btn btn-ghost" data-testid="undo-btn" onClick={() => doUndo()}>↺ Undo</button>
                    </Show>
                  </div>
                </div>

                <div class="editor-card">
                  <div class="editor-bar">
                    <Show when={dirty()}><span class="dot-unsaved" title="Unsaved changes" /></Show>
                    <span class="editor-fname">{fileName(item().path)}</span>
                    <span class="lang-tag">{isMcp() ? 'json' : langTag(item())}</span>
                    <span style={{ flex: 1 }} />
                    <Show when={md()}>
                      <div class="seg">
                        <button classList={{ 'seg-btn': true, active: !previewMode() }} onClick={() => setPreviewMode(false)}>Edit</button>
                        <button classList={{ 'seg-btn': true, active: previewMode() }} onClick={() => setPreviewMode(true)}>Preview</button>
                      </div>
                    </Show>
                  </div>
                  <Show
                    when={md() && previewMode()}
                    fallback={
                      <textarea
                        class="editor-area"
                        data-testid="detail-editor"
                        spellcheck={false}
                        value={detail()}
                        onInput={(e) => { setDetail(e.currentTarget.value); setDirty(true); }}
                        onKeyDown={(e) => { if ((e.metaKey || e.ctrlKey) && e.key === 's') { e.preventDefault(); void doSave(); } }}
                        disabled={item().locked || isMcp()}
                      />
                    }
                  >
                    <div class="preview" innerHTML={renderMarkdown(detail())} />
                  </Show>
                </div>

                <Show when={!item().locked || statusMsg()}>
                  <div class="editor-foot">
                    <Show when={!item().locked && !isMcp()}>
                      <button class="btn btn-primary" data-testid="save-btn" disabled={!dirty()} onClick={() => doSave()}>Save</button>
                      <button class="btn btn-ghost" data-testid="revert-btn" disabled={!dirty()} onClick={() => doRevert()}>Revert</button>
                      <span class="kbd">⌘S</span>
                    </Show>
                    <Show when={isMcp()}><span class="status">Read-only — MCP server entry in {fileName(item().path)}. Use the Enable/Disable toggle or MCP Policy to change it.</span></Show>
                    <span style={{ flex: 1 }} />
                    <Show when={statusMsg()}>
                      <span classList={{ status: true, err: statusMsg().includes('failed') || statusMsg().includes('error') }}>{statusMsg()}</span>
                    </Show>
                  </div>
                </Show>
              </div>
            );
          }}
        </Show>
      </div>

      <Show when={lastUndo() !== null && !selectedItem()}>
        <div class="toast" data-testid="toast">
          <span class="status">{statusMsg() || 'Action complete.'}</span>
          <button class="btn btn-ghost" data-testid="toast-undo" onClick={() => doUndo()}>↺ Undo</button>
        </div>
      </Show>
    </div>
  );
}

// ── Local re-implementation of `checkMcpPolicy` for the badge ──
//
// Kept inline (instead of going through `api.mcpCheckPolicy`) so the
// verdict appears immediately on render without a round-trip. The
// algorithm mirrors `crate::harness::adapters::claude_mcp::check_policy`.
function checkPolicyLocal(serverName: string, cfg: { command?: string; args?: string[]; url?: string }, policy: McpPolicy): PolicyVerdict {
  // Denylist first (absolute precedence).
  for (const entry of policy.denylist) {
    if (matchesEntry(entry, serverName, cfg)) return 'denied';
  }
  if (policy.allowlist.length === 0) return 'noPolicy';
  for (const entry of policy.allowlist) {
    if (matchesEntry(entry, serverName, cfg)) return 'allowed';
  }
  return 'denied';
}

function matchesEntry(
  entry: { serverName?: string; serverCommand?: string[]; serverUrl?: string },
  serverName: string,
  cfg: { command?: string; args?: string[]; url?: string },
): boolean {
  if (entry.serverName && entry.serverName === serverName) return true;
  if (entry.serverCommand && cfg.command) {
    const cmd = [cfg.command, ...(cfg.args ?? [])];
    if (entry.serverCommand.length === cmd.length && entry.serverCommand.every((c, i) => c === cmd[i])) {
      return true;
    }
  }
  if (entry.serverUrl && cfg.url) {
    if (globMatch(entry.serverUrl, cfg.url)) return true;
  }
  return false;
}

/** `*` matches any run of chars; everything else is a literal. */
function globMatch(pattern: string, value: string): boolean {
  let pi = 0, vi = 0;
  let star: number | null = null;
  let starVi = 0;
  while (vi < value.length) {
    if (pi < pattern.length && pattern[pi] === '*') {
      star = pi;
      starVi = vi;
      pi++;
    } else if (pi < pattern.length && pattern[pi] === value[vi]) {
      pi++;
      vi++;
    } else if (star !== null) {
      pi = star + 1;
      starVi++;
      vi = starVi;
    } else {
      return false;
    }
  }
  while (pi < pattern.length && pattern[pi] === '*') pi++;
  return pi === pattern.length;
}

// ── Minimal, dependency-free Markdown → HTML for the preview pane ──
// Escapes HTML first, then applies a small subset (headings, lists, code
// fences, inline code/bold/italic/links, blockquote, hr). Enough to make
// skill/memory/plan files render nicely; not a spec-complete parser.
function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function mdInline(s: string): string {
  return s
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/(^|[^*])\*([^*]+)\*/g, '$1<em>$2</em>')
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1</a>');
}

function renderMarkdown(src: string): string {
  const lines = escapeHtml(src).split('\n');
  const out: string[] = [];
  let inList = false;
  let listTag: 'ul' | 'ol' = 'ul';
  const closeList = () => { if (inList) { out.push(`</${listTag}>`); inList = false; } };
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (/^```/.test(line)) {
      closeList();
      const buf: string[] = [];
      i++;
      while (i < lines.length && !/^```/.test(lines[i])) { buf.push(lines[i]); i++; }
      i++;
      out.push(`<pre><code>${buf.join('\n')}</code></pre>`);
      continue;
    }
    const h = line.match(/^(#{1,6})\s+(.*)$/);
    if (h) { closeList(); const lvl = h[1].length; out.push(`<h${lvl}>${mdInline(h[2])}</h${lvl}>`); i++; continue; }
    if (/^\s*(---|\*\*\*|___)\s*$/.test(line)) { closeList(); out.push('<hr/>'); i++; continue; }
    if (/^>\s?/.test(line)) { closeList(); out.push(`<blockquote>${mdInline(line.replace(/^>\s?/, ''))}</blockquote>`); i++; continue; }
    const ul = line.match(/^\s*[-*+]\s+(.*)$/);
    const ol = line.match(/^\s*\d+\.\s+(.*)$/);
    if (ul || ol) {
      const tag: 'ul' | 'ol' = ul ? 'ul' : 'ol';
      if (!inList || listTag !== tag) { closeList(); listTag = tag; out.push(`<${tag}>`); inList = true; }
      out.push(`<li>${mdInline(ul ? ul[1] : ol![1])}</li>`);
      i++; continue;
    }
    if (/^\s*$/.test(line)) { closeList(); i++; continue; }
    closeList();
    out.push(`<p>${mdInline(line)}</p>`);
    i++;
  }
  closeList();
  return out.join('\n');
}
