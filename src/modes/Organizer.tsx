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
}) {
  const [activeCat, setActiveCat] = createSignal(props.scan.categories[0]?.id ?? '');
  const [detail, setDetail] = createSignal<string>('');
  const [selected, setSelected] = createSignal<string>('');
  const [showEffective, setShowEffective] = createSignal(false);
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
    const all = itemsForCat();
    if (!showEffective()) return all;
    return all.filter((i) => effectiveKeys().has(itemKey(i)));
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
    setStatusMsg('');
    setDestinations([]);
    const body = await props.loadFile(item.path);
    setDetail(body);
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
    <div style={{ display: 'flex', height: '100%' }}>
      <div style={{ width: '220px', 'border-right': '1px solid var(--border)', padding: '10px 8px' }}>
        <div style={{ 'font-size': '10px', color: 'var(--text-dim)' }}>Categories</div>
        <For each={props.scan.categories}>
          {(c) => (
            <div onClick={() => setActiveCat(c.id)}
              data-testid={`category-${c.id}`}
              style={{ display: 'flex', 'justify-content': 'space-between', padding: '5px 8px', margin: '3px 0',
                'border-radius': 'var(--radius)', cursor: 'pointer',
                background: activeCat() === c.id ? 'rgba(48,209,88,0.14)' : 'transparent' }}>
              <span>{c.label}</span><span style={{ color: 'var(--text-dim)' }}>{c.count}</span>
            </div>
          )}
        </For>
      </div>

      <div style={{ width: '360px', 'border-right': '1px solid var(--border)', padding: '10px 8px', overflow: 'auto' }}>
        <label style={{ display: 'flex', 'align-items': 'center', gap: '6px',
          'font-size': '11px', color: 'var(--text-dim)', padding: '4px 0' }}>
          <input
            type="checkbox"
            data-testid="show-effective-toggle"
            checked={showEffective()}
            onInput={(e) => setShowEffective(e.currentTarget.checked)}
          />
          Show Effective
        </label>
        <For each={props.scan.scopes}>
          {(scope) => (
            <>
              <div style={{ 'font-size': '9px', color: 'var(--text-dim)', margin: '6px 0 3px' }}>{scope.label}</div>
              <For each={visibleItems().filter((i) => i.scopeId === scope.id)}>
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
                    <div onClick={(e) => onItemClick(item, e)}
                      data-testid="item-row"
                      data-item-name={item.name}
                      data-disabled={disabled() ? 'true' : 'false'}
                      style={{ display: 'flex', 'align-items': 'center', 'justify-content': 'space-between',
                        padding: '5px 8px', margin: '3px 0', 'border-radius': 'var(--radius)', cursor: 'pointer',
                        background: selected() === k ? 'var(--surface)' :
                          selectedKeys().has(k) ? 'rgba(48,209,88,0.07)' : 'transparent' }}>
                      <span style={{ display: 'flex', gap: '6px', 'align-items': 'center' }}>
                        <Show when={selectedKeys().has(k) && k !== selected()}>
                          <span style={{ color: 'var(--accent)' }}>☑</span>
                        </Show>
                        {item.name}{item.locked ? ' 🔒' : ''}
                      </span>
                      <span style={{ display: 'flex', gap: '4px', 'align-items': 'center' }}>
                        <Show when={badge()}>
                          {(b) => (
                            <span data-testid="policy-badge"
                              style={{ 'font-size': '10px', color: b().color }}>{b().label}</span>
                          )}
                        </Show>
                        <Show when={effectiveBadge(item)}>
                          <span style={{ 'font-size': '10px', color: 'var(--text-dim)' }}>{effectiveBadge(item)}</span>
                        </Show>
                        <Show when={item.category === 'mcp' && item.scopeId !== 'global'}>
                          <button data-testid="mcp-disable-toggle" data-disabled={disabled() ? 'true' : 'false'}
                            onClick={(e) => { e.stopPropagation(); void doToggleMcpDisabled(item); }}
                            title={disabled() ? 'Enable for this project' : 'Disable for this project'}
                            style={{ padding: '1px 6px', 'font-size': '10px',
                              color: disabled() ? 'var(--danger)' : 'var(--accent)',
                              border: '1px solid var(--border)', 'border-radius': 'var(--radius)' }}>
                            {disabled() ? '✗ Disabled' : '✓ Enabled'}
                          </button>
                        </Show>
                      </span>
                    </div>
                  );
                }}
              </For>
            </>
          )}
        </For>

        <Show when={selectedKeys().size >= 2}>
          <div data-testid="bulk-bar" style={{ 'border-top': '1px solid var(--border)', 'margin-top': '10px', padding: '8px 0' }}>
            <div style={{ 'font-size': '11px', color: 'var(--text-dim)', 'margin-bottom': '4px' }}>
              Bulk: {selectedKeys().size} selected
            </div>
            <div style={{ display: 'flex', gap: '4px', 'align-items': 'center', 'flex-wrap': 'wrap' }}>
              <select value={bulkDest()} onChange={(e) => setBulkDest(e.currentTarget.value)}
                data-testid="bulk-dest"
                style={{ 'font-size': '11px', padding: '3px' }}>
                <option value="">destination…</option>
                <For each={bulkMoveDestinations()}>
                  {(d) => <option value={d.scopeId}>{d.label}</option>}
                </For>
              </select>
              <button data-testid="bulk-move" onClick={() => doBulk('move')}
                style={{ padding: '3px 8px', 'font-size': '11px' }}>Move</button>
              <button data-testid="bulk-delete" onClick={() => doBulk('delete')}
                style={{ padding: '3px 8px', 'font-size': '11px', color: 'var(--danger)' }}>Delete</button>
            </div>
          </div>
        </Show>
      </div>

      <div style={{ flex: 1, padding: '12px', display: 'flex', 'flex-direction': 'column' }}>
        <Show when={selectedItem()} fallback={
          <div style={{ color: 'var(--text-dim)' }}>Select an item</div>
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
            return (
            <>
              <div style={{ display: 'flex', 'align-items': 'center', gap: '8px', 'margin-bottom': '8px' }}>
                <strong>{item().name}</strong>
                <span style={{ color: 'var(--text-dim)', 'font-size': '11px' }}>{item().category} · {item().scopeId}</span>
                <Show when={detailBadge()}>
                  {(badge) => <span data-testid="policy-badge-detail" style={{ 'font-size': '11px', color: badge().color }}>{badge().label}</span>}
                </Show>
                <span style={{ flex: 1 }} />
                <Show when={destinations().length > 0}>
                  <div style={{ position: 'relative' }}>
                    <button data-testid="move-btn" onClick={() => setShowMoveMenu(!showMoveMenu())}
                      style={{ padding: '4px 10px', 'font-size': '12px' }}>
                      Move ▾
                    </button>
                    <Show when={showMoveMenu()}>
                      <div data-testid="move-menu"
                        style={{ position: 'absolute', top: '100%', right: 0,
                          background: 'var(--surface)', border: '1px solid var(--border)',
                          'border-radius': 'var(--radius)', 'z-index': 10, 'min-width': '160px' }}>
                        <For each={destinations()}>
                          {(d) => (
                            <div data-testid="move-dest" data-scope-id={d.scopeId}
                              onClick={() => doMove(item(), d.scopeId)}
                              style={{ padding: '6px 10px', cursor: 'pointer', 'font-size': '12px' }}>
                              {d.label}
                            </div>
                          )}
                        </For>
                      </div>
                    </Show>
                  </div>
                </Show>
                <Show when={item().deletable && !item().locked}>
                  <button data-testid="delete-btn" onClick={() => doDelete(item())}
                    style={{ padding: '4px 10px', 'font-size': '12px', color: 'var(--danger)' }}>
                    Delete
                  </button>
                </Show>
                <Show when={lastUndo() !== null}>
                  <button data-testid="undo-btn" onClick={() => doUndo()}
                    style={{ padding: '4px 10px', 'font-size': '12px' }}>
                    Undo
                  </button>
                </Show>
              </div>
              <Show when={statusMsg()}>
                <div style={{ 'font-size': '11px', color: 'var(--text-dim)', 'margin-bottom': '6px' }}>
                  {statusMsg()}
                </div>
              </Show>
              <textarea data-testid="detail-editor" value={detail()}
                onInput={(e) => { setDetail(e.currentTarget.value); setDirty(true); }}
                disabled={item().locked}
                style={{ flex: 1, 'font-family': 'var(--font-mono)', 'font-size': '12px',
                  padding: '8px', 'border-radius': 'var(--radius)',
                  border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--text)',
                  resize: 'none' }}
              />
              <Show when={!item().locked}>
                <div style={{ display: 'flex', gap: '8px', 'margin-top': '8px', 'align-items': 'center' }}>
                  <button data-testid="save-btn" disabled={!dirty()} onClick={() => doSave()}
                    style={{ padding: '4px 10px', 'font-size': '12px' }}>Save</button>
                  <button data-testid="revert-btn" disabled={!dirty()} onClick={() => doRevert()}
                    style={{ padding: '4px 10px', 'font-size': '12px' }}>Revert</button>
                  <Show when={dirty()}>
                    <span style={{ 'font-size': '11px', color: 'var(--text-dim)' }}>unsaved changes</span>
                  </Show>
                </div>
              </Show>
            </>
            );
          }}
        </Show>
      </div>
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