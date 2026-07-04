import { createMemo, createSignal, For, Show } from 'solid-js';
import type { Destination, HarnessItem, RestoreInfo, ScanResult } from '../api';

function effectiveBadge(item: HarnessItem): string | null {
  if (!item.effective) return null;
  if (item.effective === 'shadowed') return '🌫 shadowed';
  if (item.effective === 'conflict') return '⚠ conflict';
  if (item.effective === 'ancestor') return '↑ ancestor';
  return null;
}

function itemKey(item: HarnessItem): string {
  return `${item.category}::${item.name}::${item.scopeId}::${item.path}`;
}

/** Same as itemKey but takes the constituent fields. Useful when
 *  building a key from a `selected` value or restoring from a path. */
function keyFromPath(category: string, path: string, name: string, scopeId: string): string {
  return `${category}::${name}::${scopeId}::${path}`;
}

export interface OrganizerApi {
  listDestinations: (item: HarnessItem) => Promise<Destination[]>;
  moveItem: (item: HarnessItem, destScopeId: string) => Promise<RestoreInfo>;
  deleteItem: (item: HarnessItem) => Promise<RestoreInfo>;
  restore: (info: RestoreInfo) => Promise<void>;
  bulkRestore: (infos: RestoreInfo[]) => Promise<void>;
  saveFile: (path: string, content: string) => Promise<void>;
  bulk: (items: HarnessItem[], op: 'move' | 'delete', destScopeId?: string) => Promise<RestoreInfo[]>;
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

  // Helper to find the item by a fully-qualified key.
  function findByKey(key: string): HarnessItem | null {
    return props.scan.items.find((i) => itemKey(i) === key) ?? null;
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
      setSelectedKeys(new Set());
      setLastClickKey(itemKey(item));
    }
    void open(item);
  }

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
      setSelectedKeys(new Set());
      setStatusMsg(`${op === 'move' ? 'Moved' : 'Deleted'} ${items.length} items. Click Undo to reverse.`);
    } catch (e) {
      setStatusMsg(`bulk ${op} failed: ${String(e)}`);
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
        const any = props.scan.scopes.filter((s) => s.id !== item.scopeId && s.id !== 'global');
        return [
          ...props.scan.scopes.filter((s) => s.id === 'global'),
          ...any,
        ];
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

      <div style={{ width: '320px', 'border-right': '1px solid var(--border)', padding: '10px 8px', overflow: 'auto' }}>
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
                  return (
                    <div onClick={(e) => onItemClick(item, e)}
                      data-testid="item-row"
                      data-item-name={item.name}
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
                      <Show when={effectiveBadge(item)}>
                        <span style={{ 'font-size': '10px', color: 'var(--text-dim)' }}>{effectiveBadge(item)}</span>
                      </Show>
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
          {(item) => (
            <>
              <div style={{ display: 'flex', 'align-items': 'center', gap: '8px', 'margin-bottom': '8px' }}>
                <strong>{item().name}</strong>
                <span style={{ color: 'var(--text-dim)', 'font-size': '11px' }}>{item().category} · {item().scopeId}</span>
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
          )}
        </Show>
      </div>
    </div>
  );
}