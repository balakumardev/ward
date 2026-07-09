import { createMemo, createResource, createSignal, createEffect, onCleanup, For, Index, Show } from 'solid-js';
import type { Destination, HarnessItem, McpConfig, McpPolicy, PolicyVerdict, RestoreInfo, ScanResult, Scope } from '../api';
import '../styles/organizer.css';

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

/** Blank MCP item used to seed the add-mode form — empty config so the
 *  transport fields start at their stdio defaults. The name + scope for
 *  the create come from the Organizer's `newName`/`chosenScope` signals
 *  (read in `addMcp`), NOT from this stub. */
const BLANK_MCP_ITEM: HarnessItem = {
  category: 'mcp', scopeId: '', name: '', path: '',
  movable: false, deletable: true, locked: false, mcpConfig: {},
};

/** Mirror of the backend `parse_mcp_import` precedence, for the live paste
 *  preview: returns the server names found (or a parse error). */
function previewMcpImport(json: string): { servers: string[]; single: boolean; error?: string } {
  const t = json.trim();
  if (!t) return { servers: [], single: false };
  let root: unknown;
  try { root = JSON.parse(t); } catch (e) { return { servers: [], single: false, error: `invalid JSON: ${(e as Error).message}` }; }
  if (typeof root !== 'object' || root === null || Array.isArray(root)) return { servers: [], single: false, error: 'expected a JSON object' };
  const obj = root as Record<string, unknown>;
  const wrap = (obj.mcpServers ?? obj.mcp_servers) as Record<string, unknown> | undefined;
  if (wrap && typeof wrap === 'object') {
    const names = Object.keys(wrap);
    return names.length ? { servers: names, single: false } : { servers: [], single: false, error: 'no MCP servers found' };
  }
  if (typeof obj.command === 'string' || typeof obj.url === 'string') return { servers: [], single: true };
  const names = Object.keys(obj);
  return names.length ? { servers: names, single: false } : { servers: [], single: false, error: 'no MCP servers found' };
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

/** Replace the macOS/Linux home prefix with `~` for display. */
export function homeRelative(path: string): string {
  return path.replace(/^\/(?:Users|home)\/[^/]+/, '~');
}

/** Compact, tail-priority path for the header chip. Config paths read from the
 *  end — every plugin lives under the same `.../plugins/cache/...` root, so the
 *  tail (`claude-md-management/1.0.0`) is what identifies it. Keep the harness
 *  root and the last two segments, eliding the middle; short paths pass through.
 *  Full path stays in the chip's `title` tooltip. */
export function prettyPath(path: string): string {
  const rel = homeRelative(path);
  const segs = rel.split('/');
  if (segs.length <= 5) return rel;
  return `${segs.slice(0, 2).join('/')}/…/${segs.slice(-2).join('/')}`;
}

/** Best-effort clipboard copy: native Clipboard API first (works in the Tauri
 *  WKWebView on a user gesture), falling back to a hidden-textarea execCommand. */
async function copyToClipboard(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) { await navigator.clipboard.writeText(text); return true; }
  } catch { /* fall through to the legacy path */ }
  try {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand('copy');
    document.body.removeChild(ta);
    return ok;
  } catch { return false; }
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

/** Hooks / settings / plugins are "meta" items: entries inside a shared file
 *  (or an install manifest), not standalone editable files. They render a
 *  structured read-only detail card instead of the raw-file editor. */
function isMetaItem(item: HarnessItem): boolean {
  return ['hook', 'setting', 'plugin'].includes(item.category);
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
  // Plan 18 — MCP marketplace: upsert (install/edit) a server entry into a
  // scope's shared config file. Persists the structured MCP form's Save.
  upsertMcpEntry: (item: HarnessItem, config: McpConfig) => Promise<RestoreInfo>;
  // Plan 24 — import one or more MCP servers from a pasted mcpServers JSON
  // blob. Fans out to the SAME upsert engine; returns one RestoreInfo per
  // server for a batch Undo. The App bridge injects the harness + re-scans.
  mcpImportJson: (scopeId: string, json: string, fallbackName?: string) => Promise<RestoreInfo[]>;
  // Plan 19 — creatable skills: scaffold a new `<skills_dir>/<name>/SKILL.md`
  // in the chosen scope. The App bridge injects the harness + re-scans.
  skillUpsert: (scopeId: string, name: string, content: string) => Promise<RestoreInfo>;
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
  // In-app confirmation dialog. Replaces window.confirm(), which in the native
  // macOS WKWebView returns false WITHOUT showing a panel — so Delete silently
  // no-op'd (no warning, no removal). `askConfirm` opens this and resolves the
  // promise from the modal's buttons; works in both WKWebView and the browser.
  const [confirmDialog, setConfirmDialog] = createSignal<
    { message: string; confirmLabel: string; resolve: (ok: boolean) => void } | null
  >(null);
  // Path that was just copied (for the header chip's transient "✓ copied" flash).
  const [copiedPath, setCopiedPath] = createSignal<string | null>(null);
  const [dirty, setDirty] = createSignal(false);
  const [statusMsg, setStatusMsg] = createSignal<string>('');
  const [selectedKeys, setSelectedKeys] = createSignal<Set<string>>(new Set());
  const [lastClickKey, setLastClickKey] = createSignal<string>('');
  const [bulkDest, setBulkDest] = createSignal<string>('');

  // Plan 18 — "Add MCP Server" flow. `addingMcp` gates the detail pane onto
  // a blank add-mode McpForm (independent of `selectedItem()`); `newName` and
  // `chosenScope` hold the create's identity until Save.
  const [addingMcp, setAddingMcp] = createSignal(false);
  const [newName, setNewName] = createSignal('');
  const [chosenScope, setChosenScope] = createSignal('');
  // Plan 24 — the Add-MCP pane has two tabs: the structured Form (default) and
  // a Paste JSON view that imports an `mcpServers` blob (or a single server).
  const [addTab, setAddTab] = createSignal<'form' | 'paste'>('form');
  const [pasteJson, setPasteJson] = createSignal('');

  // Plan 19 — "Add Skill" flow. `addingSkill` gates the detail pane onto a
  // name+scope dialog; on Create we scaffold a starter SKILL.md and upsert it.
  const [addingSkill, setAddingSkill] = createSignal(false);
  const [skillName, setSkillName] = createSignal('');
  const [chosenSkillScope, setChosenSkillScope] = createSignal('');

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
  // Plan 18 — the editable MCP form + "+ Add" only render when the harness
  // declares a working upsert backend. Codex stays read-only until then.
  const mcpEditable = () => props.scan.capabilities.mcpEditable;
  // Plan 19 — the "+ Add Skill" control only renders when the harness can
  // create a skill (Claude true; Codex false until its write path lands).
  const skillCreatable = () => props.scan.capabilities.skillCreatable;

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
    } else if (isMetaItem(item)) {
      // Hooks / settings / plugins are entries inside a shared file (or an
      // install manifest), not standalone editable files. Loading item.path
      // would dump the whole settings.json / plugin dir. The structured detail
      // travels in `mcpConfig`; the detail pane renders a focused card, so we
      // don't need to (and shouldn't) load the raw file here.
      setDetail('');
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

  async function doCopyPath(path: string) {
    if (await copyToClipboard(path)) {
      setCopiedPath(path);
      setTimeout(() => setCopiedPath((p) => (p === path ? null : p)), 1300);
    }
  }

  // ── Confirmation dialog ──

  /** Open the in-app confirm modal; resolves true (confirmed) / false (cancelled). */
  function askConfirm(message: string, confirmLabel = 'Delete'): Promise<boolean> {
    return new Promise((resolve) => setConfirmDialog({ message, confirmLabel, resolve }));
  }
  function resolveConfirm(ok: boolean) {
    const c = confirmDialog();
    if (c) { c.resolve(ok); setConfirmDialog(null); }
  }
  // Keyboard: Escape cancels, Enter confirms — only while the dialog is open.
  createEffect(() => {
    if (!confirmDialog()) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); resolveConfirm(false); }
      else if (e.key === 'Enter') { e.preventDefault(); resolveConfirm(true); }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

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
    if (!(await askConfirm(`Delete "${item.name}"? This removes it from ${fileName(item.path)}.`))) return;
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
    if (op === 'delete' && !(await askConfirm(`Delete ${items.length} items? This cannot be undone except via the Undo button.`))) return;
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

  // Plan 18 — persist a structured MCP config edit via upsert. Reuses the
  // same lastUndo + statusMsg wiring as doToggleMcpDisabled so the toolbar
  // Undo button and status line surface the result. The upsert bridge
  // re-scans on success, so the row reflects the new config immediately.
  async function saveMcp(config: McpConfig): Promise<void> {
    const item = selectedItem();
    if (!item) return;
    try {
      const info = await props.api.upsertMcpEntry(item, config);
      setLastUndo(info);
      setStatusMsg(`Saved "${item.name}". Click Undo to reverse.`);
    } catch (e) {
      setStatusMsg(`MCP save failed: ${String(e)}`);
    }
  }

  // Plan 18 — open the blank "Add MCP Server" form in the detail pane.
  // Clears any current selection so the add view (gated on `addingMcp`)
  // owns the pane, and seeds the scope picker to the first scope.
  function startAddMcp() {
    // Add-mode is unreachable when the harness can't edit MCP (the button is
    // hidden too, but guard here so it can never be triggered programmatically).
    if (!mcpEditable()) return;
    setNewName('');
    setChosenScope(props.scan.scopes[0]?.id ?? 'global');
    setAddTab('form');
    setPasteJson('');
    setSelected('');
    setSelectedKeys(new Set<string>());
    setStatusMsg('');
    setAddingMcp(true);
  }

  // Plan 18 — create a new MCP server. The stub item carries an EMPTY
  // `path` so the App bridge forwards `targetPath: undefined` → Rust
  // resolves the chosen scope's config file. Name + scope come from the
  // add-form signals. On success the App's refetch surfaces the new row.
  async function addMcp(config: McpConfig): Promise<void> {
    const name = newName().trim();
    if (!name) { setStatusMsg('MCP add failed: name is required.'); return; }
    try {
      const info = await props.api.upsertMcpEntry(
        { category: 'mcp', scopeId: chosenScope(), name, path: '', movable: false, deletable: true, locked: false },
        config,
      );
      setAddingMcp(false);
      setLastUndo(info);
      setStatusMsg(`Added "${name}". Click Undo to reverse.`);
    } catch (e) {
      setStatusMsg(`MCP add failed: ${String(e)}`);
    }
  }

  // Plan 24 — import a pasted `mcpServers` JSON blob (or a single bare server
  // object) into the chosen scope. Mirrors the backend precedence for the
  // pre-flight preview, forwards a fallback name only for a single server, and
  // stores the batch RestoreInfo[] the same array form `doBulk` uses for Undo.
  async function doImportMcp(): Promise<void> {
    const prev = previewMcpImport(pasteJson());
    if (prev.error) { setStatusMsg(`MCP import failed: ${prev.error}`); return; }
    if (prev.single && !newName().trim()) { setStatusMsg('MCP import failed: a single server needs a name.'); return; }
    try {
      const infos = await props.api.mcpImportJson(chosenScope(), pasteJson(), prev.single ? newName().trim() : undefined);
      setAddingMcp(false);
      setPasteJson('');
      setAddTab('form');
      setLastUndo(infos);
      setStatusMsg(`Imported ${infos.length} server(s). Click Undo to reverse.`);
    } catch (e) {
      setStatusMsg(`MCP import failed: ${String((e as { message?: string })?.message ?? e)}`);
    }
  }

  // Plan 19 — open the blank "Add Skill" dialog in the detail pane. Clears any
  // current selection so the add view (gated on `addingSkill`) owns the pane
  // and seeds the scope picker to the first scope.
  function startAddSkill() {
    // Unreachable when the harness can't create skills (the button is hidden
    // too, but guard here so it can never be triggered programmatically).
    if (!skillCreatable()) return;
    setSkillName('');
    setChosenSkillScope(props.scan.scopes[0]?.id ?? 'global');
    setAddingMcp(false);
    setSelected('');
    setSelectedKeys(new Set<string>());
    setStatusMsg('');
    setAddingSkill(true);
  }

  // Plan 19 — scaffold a new skill: build a starter SKILL.md (frontmatter +
  // heading + guidance comment) and create it via skillUpsert. On success the
  // App's refetch surfaces the new item so the user fills it in with the
  // existing markdown editor and Saves via the existing `save_file` path.
  async function createSkill() {
    if (!skillCreatable()) return;
    const name = skillName().trim();
    if (!name) { setStatusMsg('Skill add failed: name is required.'); return; }
    const content = `---\nname: ${name}\ndescription: TODO one-line description\n---\n\n# ${name}\n\n<!-- Describe when this skill applies and what it does. -->\n`;
    try {
      const info = await props.api.skillUpsert(chosenSkillScope(), name, content);
      setAddingSkill(false);
      setLastUndo(info);
      setStatusMsg(`Created skill ${name}. Click Undo to reverse.`);
    } catch (e) {
      setStatusMsg(`Skill add failed: ${String(e)}`);
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
            <Show when={activeCat() === 'mcp' && mcpEditable()}>
              <button class="mcp-add-btn" data-testid="mcp-add-button" onClick={() => startAddMcp()}>
                + Add
              </button>
            </Show>
            <Show when={activeCat() === 'skill' && skillCreatable()}>
              <button class="mcp-add-btn" data-testid="skill-add-button" onClick={() => startAddSkill()}>
                + Add Skill
              </button>
            </Show>
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
                          <Show when={isMetaItem(item) && item.description} fallback={
                            <span class="row-name">{item.name}</span>
                          }>
                            <span class="row-namewrap">
                              <span class="row-name">{item.name}</span>
                              <span class="row-sub">{item.description}</span>
                            </span>
                          </Show>
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
        {/* Plan 19 — the "Add Skill" dialog owns the pane when active (gated
            OUTERMOST). Plan 18 — the "Add MCP Server" view owns it next,
            independent of the current selection. Both are gated before the
            keyed edit-mode <Show> below. */}
        <Show when={addingSkill()} fallback={
        <Show when={addingMcp() && mcpEditable()} fallback={
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
            // Only render the editable structured form when the harness can
            // actually persist an upsert. Otherwise MCP falls back to a
            // read-only pane (see the keyed <Show> fallback below).
            const showMcpForm = () => isMcp() && mcpEditable();
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
                      <span
                        classList={{ chip: true, 'chip-path': true, copyable: true, copied: copiedPath() === item().path }}
                        role="button" tabindex="0"
                        title={copiedPath() === item().path ? 'Copied to clipboard' : `Copy path — ${item().path}`}
                        onClick={() => doCopyPath(item().path)}
                        onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); void doCopyPath(item().path); } }}
                      >{copiedPath() === item().path ? '✓ copied' : prettyPath(item().path)}</span>
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

                {/* Plan 18 — MCP entries get a structured stdio/http form
                    (Save persists via upsertMcpEntry). Hooks/settings/plugins
                    get a focused read-only card. Everything else gets the file
                    editor / markdown preview. */}
                <Show when={showMcpForm() ? item() : null} keyed fallback={
                  <Show when={isMetaItem(item())} fallback={
                  <Show when={isMcp()} fallback={
                    <div class="editor-card">
                      <div class="editor-bar">
                        <Show when={dirty()}><span class="dot-unsaved" title="Unsaved changes" /></Show>
                        <span class="editor-fname">{fileName(item().path)}</span>
                        <span class="lang-tag">{langTag(item())}</span>
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
                            disabled={item().locked}
                          />
                        }
                      >
                        <div class="preview" innerHTML={renderMarkdown(detail())} />
                      </Show>
                    </div>
                  }>
                    {/* Plan 18 — MCP without an upsert backend (e.g. Codex):
                        read-only display of the server config as pretty JSON. */}
                    <div class="editor-card">
                      <div class="editor-bar">
                        <span class="editor-fname">{fileName(item().path)}</span>
                        <span class="lang-tag">json</span>
                      </div>
                      <textarea
                        class="editor-area"
                        data-testid="detail-editor"
                        spellcheck={false}
                        value={JSON.stringify(item().mcpConfig ?? {}, null, 2)}
                        disabled
                      />
                      <div class="mcp-readonly-note" data-testid="mcp-readonly-note">
                        Read-only — MCP server entry in {fileName(item().path)}. Editing MCP for this harness isn’t supported yet.
                      </div>
                    </div>
                  </Show>
                  }>
                    <MetaCard item={item()} />
                  </Show>
                }>
                  {(mcpItem) => <McpForm item={mcpItem} onSave={saveMcp} harness={props.scan.harnessId} />}
                </Show>

                <Show when={(!item().locked && !isMcp()) || statusMsg()}>
                  <div class="editor-foot">
                    <Show when={!item().locked && !isMcp()}>
                      <button class="btn btn-primary" data-testid="save-btn" disabled={!dirty()} onClick={() => doSave()}>Save</button>
                      <button class="btn btn-ghost" data-testid="revert-btn" disabled={!dirty()} onClick={() => doRevert()}>Revert</button>
                      <span class="kbd">⌘S</span>
                    </Show>
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
        }>
          {/* ── Plan 18 — Add MCP Server (blank form: name + scope + config) ── */}
          <div class="rise" style={{ display: 'flex', 'flex-direction': 'column', height: '100%', 'min-height': 0 }}>
            <div class="detail-head">
              <div class="detail-titlewrap">
                <div class="detail-title">Add MCP Server</div>
                <div class="detail-meta">
                  <span class="chip"><span class="cat-dot" style={{ '--dot': catDot('mcp') }} />mcp</span>
                  <span class="chip">new server</span>
                </div>
              </div>
              <div class="toolbar">
                <button class="btn btn-ghost" data-testid="mcp-add-cancel" onClick={() => setAddingMcp(false)}>Cancel</button>
              </div>
            </div>
            {/* Plan 24 — Form / Paste JSON tabs. Form is the structured add;
                Paste imports an mcpServers blob (or a single server object). */}
            <div class="seg mcp-add-tabs">
              <button classList={{ 'seg-btn': true, active: addTab() === 'form' }}
                data-testid="mcp-form-tab" onClick={() => setAddTab('form')}>Form</button>
              <button classList={{ 'seg-btn': true, active: addTab() === 'paste' }}
                data-testid="mcp-paste-tab" onClick={() => setAddTab('paste')}>Paste JSON</button>
            </div>
            <Show when={addTab() === 'paste'} fallback={
              <McpForm
                mode="add"
                item={BLANK_MCP_ITEM}
                scopes={props.scan.scopes}
                name={newName()}
                onName={setNewName}
                scopeId={chosenScope()}
                onScope={setChosenScope}
                onSave={addMcp}
                harness={props.scan.harnessId}
              />
            }>
              <div class="mcp-paste" data-testid="mcp-paste">
                <label class="mcp-label">Scope</label>
                <select class="mcp-input mcp-scope-select" data-testid="mcp-paste-scope"
                  value={chosenScope()} onChange={(e) => setChosenScope(e.currentTarget.value)}>
                  <For each={props.scan.scopes}>{(s) => <option value={s.id}>{s.label}</option>}</For>
                </select>
                <label class="mcp-label">Paste an mcpServers JSON block (or a single server object)</label>
                <textarea class="mcp-input mcp-paste-area" data-testid="mcp-paste-json" spellcheck={false}
                  placeholder={'{\n  "mcpServers": {\n    "context7": { "command": "npx", "args": ["-y", "@upstash/context7-mcp"] }\n  }\n}'}
                  value={pasteJson()} onInput={(e) => setPasteJson(e.currentTarget.value)} />
                {/* single-server paste needs a name */}
                <Show when={previewMcpImport(pasteJson()).single}>
                  <label class="mcp-label">Name (single server)</label>
                  <input class="mcp-input" data-testid="mcp-paste-name" placeholder="server-name" spellcheck={false}
                    value={newName()} onInput={(e) => setNewName(e.currentTarget.value)} />
                </Show>
                <div class="mcp-paste-preview" data-testid="mcp-paste-preview">
                  <Show when={previewMcpImport(pasteJson()).error}>
                    <span class="mcp-paste-err">{previewMcpImport(pasteJson()).error}</span>
                  </Show>
                  <Show when={previewMcpImport(pasteJson()).servers.length > 0}>
                    <span>Will add: {previewMcpImport(pasteJson()).servers.join(', ')}</span>
                  </Show>
                </div>
                <div class="editor-foot">
                  <button class="btn btn-primary" data-testid="mcp-paste-import"
                    disabled={previewMcpImport(pasteJson()).servers.length === 0 && !previewMcpImport(pasteJson()).single}
                    onClick={() => void doImportMcp()}>Import</button>
                </div>
              </div>
            </Show>
            <Show when={statusMsg()}>
              <div class="editor-foot">
                <span style={{ flex: 1 }} />
                <span classList={{ status: true, err: statusMsg().includes('failed') || statusMsg().includes('error') }}>{statusMsg()}</span>
              </div>
            </Show>
          </div>
        </Show>
        }>
          {/* ── Plan 19 — Add Skill (name + scope → scaffold SKILL.md) ── */}
          <div class="rise" style={{ display: 'flex', 'flex-direction': 'column', height: '100%', 'min-height': 0 }}>
            <div class="detail-head">
              <div class="detail-titlewrap">
                <div class="detail-title">Add Skill</div>
                <div class="detail-meta">
                  <span class="chip"><span class="cat-dot" style={{ '--dot': catDot('skill') }} />skill</span>
                  <span class="chip">new skill</span>
                </div>
              </div>
              <div class="toolbar">
                <button class="btn btn-ghost" data-testid="skill-add-cancel" onClick={() => setAddingSkill(false)}>Cancel</button>
              </div>
            </div>
            <div class="skill-add" data-testid="skill-add-form">
              <label class="mcp-label">Name</label>
              <input class="mcp-input" data-testid="skill-add-name" placeholder="skill-name" spellcheck={false}
                value={skillName()} onInput={(e) => setSkillName(e.currentTarget.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); void createSkill(); } }} />
              <label class="mcp-label">Scope</label>
              <select class="mcp-input mcp-scope-select" data-testid="skill-add-scope"
                value={chosenSkillScope()} onChange={(e) => setChosenSkillScope(e.currentTarget.value)}>
                <For each={props.scan.scopes}>
                  {(s) => <option value={s.id}>{s.label}</option>}
                </For>
              </select>
              <div class="skill-add-hint">
                A starter <code>SKILL.md</code> is scaffolded; fill it in with the editor and Save.
              </div>
              <div class="editor-foot">
                <button class="btn btn-primary" data-testid="skill-add-create" onClick={() => void createSkill()}>Create</button>
              </div>
            </div>
            <Show when={statusMsg()}>
              <div class="editor-foot">
                <span style={{ flex: 1 }} />
                <span classList={{ status: true, err: statusMsg().includes('failed') || statusMsg().includes('error') }}>{statusMsg()}</span>
              </div>
            </Show>
          </div>
        </Show>
      </div>

      <Show when={lastUndo() !== null && !selectedItem()}>
        <div class="toast" data-testid="toast">
          <span class="status">{statusMsg() || 'Action complete.'}</span>
          <button class="btn btn-ghost" data-testid="toast-undo" onClick={() => doUndo()}>↺ Undo</button>
        </div>
      </Show>

      {/* In-app confirmation dialog (see askConfirm). Native WKWebView's
          window.confirm() silently returns false, so Delete needs a real modal. */}
      <Show when={confirmDialog()} keyed>
        {(c) => (
          <div class="modal-overlay" data-testid="confirm-overlay" onClick={() => resolveConfirm(false)}>
            <div class="modal" role="dialog" aria-modal="true" aria-labelledby="confirm-title"
              onClick={(e) => e.stopPropagation()}>
              <div class="modal-title" id="confirm-title">⚠ Confirm</div>
              <div class="modal-body">{c.message}</div>
              <div class="modal-actions">
                <button class="btn btn-ghost" data-testid="confirm-cancel" onClick={() => resolveConfirm(false)}>Cancel</button>
                <button class="btn btn-danger" data-testid="confirm-ok" onClick={() => resolveConfirm(true)}>{c.confirmLabel}</button>
              </div>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}

// ── Meta-item detail card (hooks / settings / plugins) ──
//
// These categories are entries inside a shared file (settings.json) or an
// install manifest, NOT standalone editable files. Instead of dumping the raw
// JSON into the editor, we render a focused read-only card: the fields that
// matter for that item, plus the source file it came from. The structured data
// travels from the Rust scanner in `item.mcpConfig` (a generic JSON blob).
function MetaCard(props: { item: HarnessItem }) {
  const meta = () => (props.item.mcpConfig ?? {}) as Record<string, unknown>;
  const kind = () => String(meta().kind ?? props.item.category);
  const str = (k: string) => {
    const v = meta()[k];
    return v == null ? '' : String(v);
  };
  return (
    <div class="meta-card" data-testid="meta-card" data-kind={kind()}>
      <Show when={kind() === 'hook'}>
        <div class="meta-rows">
          <MetaRow label="Event" value={str('event')} mono />
          <Show when={str('matcher')}>
            <MetaRow label="Matcher" value={str('matcher')} mono />
          </Show>
          <MetaRow label="Type" value={str('type') || 'command'} />
          <Show when={str('timeout')}>
            <MetaRow label="Timeout" value={`${str('timeout')}s`} />
          </Show>
          <MetaRow label={str('type') === 'http' ? 'URL' : 'Command'} value={str('action')} mono block />
          <MetaRow label="Source" value={str('source')} mono />
        </div>
        <div class="meta-note">
          Runs on the <code>{str('event')}</code> lifecycle event
          {str('matcher') && str('matcher') !== '*' ? <> when the matcher matches</> : <> (all events)</>}.
          Hooks are defined in <code>{str('source')}</code> and run as shell commands regardless of what Claude decides.
        </div>
      </Show>

      <Show when={kind() === 'setting'}>
        <div class="meta-rows">
          <MetaRow label="Key" value={str('key')} mono />
          <MetaRow label="Source" value={str('source')} mono />
        </div>
        <div class="meta-value-block">
          <div class="meta-value-label">Value</div>
          <pre class="meta-value-json"><code>{JSON.stringify(meta().value ?? null, null, 2)}</code></pre>
        </div>
      </Show>

      <Show when={kind() === 'plugin'}>
        <div class="meta-rows">
          <MetaRow label="Plugin" value={props.item.name} mono />
          <Show when={props.item.description}>
            <MetaRow label="Details" value={props.item.description ?? ''} />
          </Show>
          <MetaRow label="Install path" value={homeRelative(props.item.path)} mono block />
        </div>
        <div class="meta-note">
          Managed by Claude Code's plugin system. Toggle it with
          {' '}<code>/plugin</code> or by editing <code>enabledPlugins</code> in <code>settings.json</code>.
        </div>
      </Show>
    </div>
  );
}

function MetaRow(props: { label: string; value: string; mono?: boolean; block?: boolean }) {
  return (
    <div classList={{ 'meta-row': true, block: !!props.block }}>
      <span class="meta-row-label">{props.label}</span>
      <span classList={{ 'meta-row-value': true, mono: !!props.mono }}>{props.value || '—'}</span>
    </div>
  );
}

// ── Plan 18 — structured MCP server edit form ──
// MCP "items" are entries inside a shared JSON/TOML config file, not
// standalone files, so the raw-text editor would clobber the whole file.
// Instead we surface a structured form: a stdio ⇄ http transport toggle
// with the fields each transport owns. Save patches a CLONE of the
// original config (so unknown keys round-trip) and persists via
// `upsertMcpEntry`.
//
// Two modes (Plan 18):
//  - `edit` (default): the server name is NOT editable — a rename is a
//    delete + re-add, out of scope for an in-place edit.
//  - `add`: a name input + scope picker appear at the top and the config
//    starts blank. Name/scope are controlled by the parent (Organizer's
//    `newName`/`chosenScope` signals) via the `name`/`onName`/`scopeId`/
//    `onScope` props, so `onSave(config)` only needs to carry the config.
/** Remote (non-stdio) MCP transport `type` values. Used to reconcile a
 *  stale `type` key when the user toggles the stdio ⇄ http transport so an
 *  entry never carries a `type` that contradicts its transport shape. */
const REMOTE_TYPES = ['http', 'sse'];

function McpForm(props: {
  item: HarnessItem;
  onSave: (config: McpConfig) => Promise<void>;
  mode?: 'add' | 'edit';
  scopes?: Scope[];
  name?: string;
  onName?: (v: string) => void;
  scopeId?: string;
  onScope?: (v: string) => void;
  harness?: string;
}) {
  const isAdd = () => props.mode === 'add';
  // Codex persists an `enabled` bool inside `[mcp_servers.<name>]` (there's no
  // Claude-style per-project row toggle for Codex), so we expose an Enabled
  // checkbox for Codex only. Claude ignores the field and never shows it.
  const isCodex = () => props.harness === 'codex';
  const seed = () => (props.item.mcpConfig ?? {}) as McpConfig;
  const [transport, setTransport] = createSignal<'stdio' | 'http'>(seed().url ? 'http' : 'stdio');
  const [command, setCommand] = createSignal(seed().command ?? '');
  const [args, setArgs] = createSignal<string[]>([...(seed().args ?? [])]);
  const [env, setEnv] = createSignal<[string, string][]>(Object.entries<string>(seed().env ?? {}));
  const [url, setUrl] = createSignal(seed().url ?? '');
  const [headers, setHeaders] = createSignal<[string, string][]>(Object.entries<string>(seed().headers ?? {}));
  // `enabled` defaults to true when the config omits it (Codex treats a missing
  // key as enabled).
  const [enabled, setEnabled] = createSignal<boolean>(seed().enabled ?? true);
  const [busy, setBusy] = createSignal(false);

  async function save() {
    setBusy(true);
    // Patch a CLONE of the original so unknown keys survive.
    const next: McpConfig = { ...((props.item.mcpConfig as McpConfig) ?? {}) };
    if (transport() === 'stdio') {
      next.command = command();
      next.args = args();
      next.env = Object.fromEntries(env().filter(([k]) => k));
      delete next.url; delete next.headers;
      // A leftover remote `type` (http/sse) contradicts a stdio entry — drop it.
      if (typeof next.type === 'string' && REMOTE_TYPES.includes(next.type)) delete next.type;
    } else {
      next.url = url();
      next.headers = Object.fromEntries(headers().filter(([k]) => k));
      delete next.command; delete next.args; delete next.env;
      // A leftover non-remote `type` (e.g. 'stdio') contradicts an http entry.
      // Realign it to a remote type when one was present; never fabricate one.
      if (next.type !== undefined && !REMOTE_TYPES.includes(String(next.type))) next.type = 'http';
    }
    // Codex owns the `enabled` bool; Claude leaves whatever was there untouched.
    if (isCodex()) next.enabled = enabled();
    try { await props.onSave(next); } finally { setBusy(false); }
  }

  return (
    <div class="mcp-form" data-testid="mcp-form">
      <Show when={isAdd()}>
        <label class="mcp-label">Name</label>
        <input class="mcp-input" data-testid="mcp-name" placeholder="server-name" spellcheck={false}
          value={props.name ?? ''} onInput={(e) => props.onName?.(e.currentTarget.value)} />
        <label class="mcp-label">Scope</label>
        <select class="mcp-input mcp-scope-select" data-testid="mcp-scope-pick"
          value={props.scopeId ?? ''} onChange={(e) => props.onScope?.(e.currentTarget.value)}>
          <For each={props.scopes ?? []}>
            {(s) => <option value={s.id}>{s.label}</option>}
          </For>
        </select>
      </Show>
      <div class="seg mcp-transport">
        <button classList={{ 'seg-btn': true, active: transport() === 'stdio' }}
          data-testid="mcp-transport-stdio" onClick={() => setTransport('stdio')}>stdio</button>
        <button classList={{ 'seg-btn': true, active: transport() === 'http' }}
          data-testid="mcp-transport-http" onClick={() => setTransport('http')}>http</button>
      </div>
      <Show when={isCodex()}>
        <label class="toggle mcp-enabled-row">
          <input type="checkbox" data-testid="mcp-enabled" checked={enabled()}
            onInput={(e) => setEnabled(e.currentTarget.checked)} />
          Enabled
        </label>
      </Show>
      <Show when={transport() === 'stdio'} fallback={
        <>
          <label class="mcp-label">URL</label>
          <input class="mcp-input" data-testid="mcp-url" value={url()} onInput={(e) => setUrl(e.currentTarget.value)} />
          <KeyValRows label="Headers" rows={headers()} setRows={setHeaders} testid="mcp-header" />
        </>
      }>
        <label class="mcp-label">Command</label>
        <input class="mcp-input" data-testid="mcp-command" value={command()} onInput={(e) => setCommand(e.currentTarget.value)} />
        <ListRows label="Args" rows={args()} setRows={setArgs} testid="mcp-arg" />
        <KeyValRows label="Env" rows={env()} setRows={setEnv} testid="mcp-env" />
      </Show>
      <div class="editor-foot">
        <button class="btn btn-primary" data-testid="mcp-save" disabled={busy()} onClick={() => void save()}>Save</button>
      </div>
    </div>
  );
}

/** A list of single-value rows (e.g. stdio `args`), each with a remove
 *  button and an "Add" control that appends an empty row. Uses <Index>
 *  (not <For>) so editing a row keeps a stable DOM node — <For> keys by
 *  value and would recreate the input on each keystroke, losing focus. */
function ListRows(props: { label: string; rows: string[]; setRows: (v: string[]) => void; testid: string }) {
  const add = () => props.setRows([...props.rows, '']);
  const update = (i: number, v: string) => props.setRows(props.rows.map((r, j) => (j === i ? v : r)));
  const remove = (i: number) => props.setRows(props.rows.filter((_, j) => j !== i));
  return (
    <div class="mcp-rows">
      <div class="mcp-rows-head">
        <label class="mcp-label">{props.label}</label>
        <button class="mcp-row-add" data-testid={`${props.testid}-add`} onClick={() => add()}>+ Add</button>
      </div>
      <Index each={props.rows}>
        {(row, i) => (
          <div class="mcp-row" data-testid={`${props.testid}-row`}>
            <input class="mcp-input" data-testid={`${props.testid}-input`} value={row()}
              onInput={(e) => update(i, e.currentTarget.value)} />
            <button class="mcp-row-del" data-testid={`${props.testid}-remove`}
              title="Remove" onClick={() => remove(i)}>✕</button>
          </div>
        )}
      </Index>
    </div>
  );
}

/** A list of key/value rows (e.g. stdio `env` or http `headers`), each
 *  with two inputs, a remove button, and an "Add" control. Uses <Index>
 *  for the same stable-DOM-node reason as ListRows. */
function KeyValRows(props: { label: string; rows: [string, string][]; setRows: (v: [string, string][]) => void; testid: string }) {
  const add = () => props.setRows([...props.rows, ['', '']]);
  const update = (i: number, which: 0 | 1, v: string) =>
    props.setRows(props.rows.map((r, j): [string, string] =>
      j === i ? (which === 0 ? [v, r[1]] : [r[0], v]) : r,
    ));
  const remove = (i: number) => props.setRows(props.rows.filter((_, j) => j !== i));
  return (
    <div class="mcp-rows">
      <div class="mcp-rows-head">
        <label class="mcp-label">{props.label}</label>
        <button class="mcp-row-add" data-testid={`${props.testid}-add`} onClick={() => add()}>+ Add</button>
      </div>
      <Index each={props.rows}>
        {(row, i) => (
          <div class="mcp-row mcp-row-kv" data-testid={`${props.testid}-row`}>
            <input class="mcp-input mcp-input-key" data-testid={`${props.testid}-key`} placeholder="key"
              value={row()[0]} onInput={(e) => update(i, 0, e.currentTarget.value)} />
            <input class="mcp-input" data-testid={`${props.testid}-value`} placeholder="value"
              value={row()[1]} onInput={(e) => update(i, 1, e.currentTarget.value)} />
            <button class="mcp-row-del" data-testid={`${props.testid}-remove`}
              title="Remove" onClick={() => remove(i)}>✕</button>
          </div>
        )}
      </Index>
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
