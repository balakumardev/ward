import { createEffect, createMemo, createResource, createSignal, For, Index, onCleanup, Show } from 'solid-js';
import type { RestoreInfo, ScanResult, SettingRow } from '../api';
import '../styles/settings.css';

/** Plan 29 — Settings mode.
 *
 * A curated, searchable browser over Claude Code's `settings.json` (+ a handful
 * of `~/.claude.json` global-config keys) that the published JSON Schema can't
 * give you on its own: every row carries a human label + description, the
 * **effective value** resolved across the scope chain, a **source-scope chip**
 * (which scope actually set it), and an inline editor picked by the setting's
 * `valueType`. Simple scalars (bool / enum / number / string) edit in place;
 * complex `array` / `object` settings surface an "Edit…" affordance that a later
 * task wires to bespoke editors (permissions / hooks / env / sandbox /
 * statusLine) — it is intentionally inert here so those rows never crash the
 * list.
 *
 * Ward writes the **user** scope only (`~/.claude/settings.json`, or
 * `~/.claude.json` for the handful of global-config keys — routed by the def's
 * `targetFile`). Project/local scopes are cwd-relative and meaningless for a
 * Finder-launched app; they're a documented follow-up. Managed settings can't
 * be overridden from user scope, so those rows render read-only.
 *
 * Every write returns a `setting-write` RestoreInfo, so the success toast offers
 * a one-click Undo through the shared restore engine. Ward can't reload Claude
 * Code itself, so the toast notes that some settings need a restart to apply.
 *
 * Class-based (`settings.css`); no inline styling drives interactive state. No
 * `window.confirm()` anywhere — set/unset are reversible via Undo, so there's
 * nothing to confirm (and WKWebView's confirm() is a silent no-op regardless).
 */

/** The api surface the Settings mode needs. App.tsx wires these to `api.*`
 *  (`set`/`unset` are surgical single-key writes returning a `setting-write`
 *  RestoreInfo; `restore` reverses one through the shared engine — settings are
 *  Claude-only, so App always injects the 'claude' harness); tests pass a
 *  lightweight stub. */
export interface SettingsApi {
  catalog: () => Promise<SettingRow[]>;
  set: (scope: string, key: string, targetFile: string, value: unknown) => Promise<RestoreInfo>;
  unset: (scope: string, key: string, targetFile: string) => Promise<RestoreInfo>;
  restore: (info: RestoreInfo) => Promise<void>;
}

/** Plan 29 — every write is fixed to USER scope (see the module doc). The Rust
 *  `settings_set`/`settings_unset` commands accept a scope string, so
 *  project/local lands later without churn. */
const SETTINGS_SCOPE = 'user';

/** Human label for a target file — routes the "where does this write" caption. */
function targetFileLabel(targetFile: string): string {
  return targetFile === 'claudeJson' ? '~/.claude.json' : '~/.claude/settings.json';
}

/** The value resolved across the scope chain. An UNSET row carries no
 *  `effective`, so it falls through to the documented default (which may itself
 *  be absent — then there is genuinely no value). */
function effectiveOf(row: SettingRow): unknown {
  return row.effective !== undefined ? row.effective : row.def.default;
}

/** A compact, read-only rendering of a value for the row's effective readout.
 *  Arrays join with commas; objects show their JSON (the row's real editor is
 *  the "Edit…" button, so this is only a glance). */
function formatValue(v: unknown, valueType: string): string {
  if (v === undefined || v === null) return '—';
  if (valueType === 'bool') return v ? 'On' : 'Off';
  if (valueType === 'array') {
    if (!Array.isArray(v)) return String(v);
    return v.length ? v.map((x) => (typeof x === 'string' ? x : JSON.stringify(x))).join(', ') : '(empty)';
  }
  if (valueType === 'object') {
    try {
      return JSON.stringify(v);
    } catch {
      return String(v);
    }
  }
  return String(v);
}

/** A managed row can't be overridden from user scope — its editor is read-only.
 *  True when the key is managed-only, or the effective value currently comes
 *  from the managed scope. */
function isManaged(row: SettingRow): boolean {
  return row.def.managedOnly || row.sourceScope === 'managed';
}

/** Root: capability-gate the whole mode. Codex has no editable settings catalog,
 *  so `settingsEditable` is false there — mirror the Plugins gate with a "not
 *  supported" panel (and skip the on-mount catalog fetch entirely). */
export function Settings(props: { scan: ScanResult; api: SettingsApi }) {
  return (
    <Show
      when={props.scan.capabilities.settingsEditable}
      fallback={
        <div class="settings-unsupported" data-testid="settings-unsupported">
          <span>Settings aren't editable for this harness.</span>
        </div>
      }
    >
      <SettingsBody api={props.api} />
    </Show>
  );
}

/** The mode body: a category rail + searchable list of setting rows over a
 *  shared catalog resource. Simple editors write on change; array/object rows
 *  show an inert "Edit…" button. Every write is fixed to user scope and, on
 *  success, re-reads the catalog and shows a toast with Undo. */
function SettingsBody(props: { api: SettingsApi }) {
  const [catalog, { refetch }] = createResource(() => props.api.catalog());

  const [cat, setCat] = createSignal('all');
  const [search, setSearch] = createSignal('');
  const [toast, setToast] = createSignal<string | null>(null);
  // Set alongside every successful set/unset; drives the toast's Undo button.
  const [undoInfo, setUndoInfo] = createSignal<RestoreInfo | null>(null);
  const [busy, setBusy] = createSignal(false);
  // The row whose complex-value editor (array / env / generic JSON) is open in a
  // modal, or null when none. Keyed rendering reseeds the editor's working state
  // each time it opens (see EditorModal).
  const [editorRow, setEditorRow] = createSignal<SettingRow | null>(null);

  const rows = () => catalog() ?? [];

  // Ordered, unique categories in first-seen order — the rail's contents.
  const categories = createMemo<string[]>(() => {
    const seen: string[] = [];
    for (const r of rows()) {
      if (!seen.includes(r.def.category)) seen.push(r.def.category);
    }
    return seen;
  });

  // Per-category counts for the rail badges.
  const catCount = (c: string) => rows().filter((r) => r.def.category === c).length;

  const filtered = createMemo<SettingRow[]>(() => {
    const q = search().trim().toLowerCase();
    const c = cat();
    return rows().filter((r) => {
      if (c !== 'all' && r.def.category !== c) return false;
      if (!q) return true;
      return (
        r.def.key.toLowerCase().includes(q) ||
        r.def.label.toLowerCase().includes(q) ||
        r.def.description.toLowerCase().includes(q)
      );
    });
  });

  // Show a toast, optionally with an Undo bound to a `setting-write` RestoreInfo.
  function notify(msg: string, undo: RestoreInfo | null = null) {
    setUndoInfo(undo);
    setToast(msg);
  }

  // ── Mutations (all fixed to user scope; targetFile from the def) ──
  async function applySet(row: SettingRow, value: unknown) {
    if (isManaged(row)) return; // defensive — managed editors are disabled
    setBusy(true);
    try {
      const info = await props.api.set(SETTINGS_SCOPE, row.def.key, row.def.targetFile, value);
      await refetch();
      notify(`Saved ${row.def.label}. Some settings need a Claude Code restart to apply.`, info);
    } catch (e) {
      notify(`Save failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function applyReset(row: SettingRow) {
    setBusy(true);
    try {
      const info = await props.api.unset(SETTINGS_SCOPE, row.def.key, row.def.targetFile);
      await refetch();
      notify(`Reset ${row.def.label} to its default. Some settings need a Claude Code restart to apply.`, info);
    } catch (e) {
      notify(`Reset failed: ${e instanceof Error ? e.message : String(e)}`);
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
      notify('Reverted the last change.');
    } catch (e) {
      notify(`Undo failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="settings-mode" data-testid="settings-mode">
      {/* ── Category rail ── */}
      <aside class="set-rail" data-testid="settings-cats">
        <div class="set-rail-title">Categories</div>
        <button
          type="button"
          class="set-cat"
          classList={{ active: cat() === 'all' }}
          data-testid="settings-cat"
          onClick={() => setCat('all')}
        >
          <span class="set-cat-label">All</span>
          <span class="set-cat-count">{rows().length}</span>
        </button>
        <For each={categories()}>
          {(c) => (
            <button
              type="button"
              class="set-cat"
              classList={{ active: cat() === c }}
              data-testid="settings-cat"
              onClick={() => setCat(c)}
            >
              <span class="set-cat-label">{c}</span>
              <span class="set-cat-count">{catCount(c)}</span>
            </button>
          )}
        </For>
      </aside>

      {/* ── Main list ── */}
      <div class="set-main">
        <header class="set-head">
          <div class="set-head-row">
            <input
              class="set-search"
              data-testid="settings-search"
              type="search"
              placeholder="Search settings by name or description…"
              value={search()}
              onInput={(e) => setSearch(e.currentTarget.value)}
            />
          </div>
          <p class="set-scope-note" data-testid="settings-scope-note">
            Ward writes the <strong>User</strong> scope · <code>~/.claude</code>. Project and local
            scopes are a documented follow-up.
          </p>
        </header>

        <div class="set-list">
          <Show
            when={!catalog.loading}
            fallback={<div class="set-status" data-testid="settings-loading">Reading settings…</div>}
          >
            <Show
              when={!catalog.error}
              fallback={<div class="set-status err" data-testid="settings-error">Failed to read settings: {String(catalog.error)}</div>}
            >
              <Show
                when={filtered().length > 0}
                fallback={<div class="set-status" data-testid="settings-empty">No settings match. Clear the search or pick another category.</div>}
              >
                <For each={filtered()}>
                  {(row) => (
                    <SettingRowView
                      row={row}
                      busy={busy()}
                      onSet={applySet}
                      onReset={applyReset}
                      onEdit={(r) => setEditorRow(r)}
                    />
                  )}
                </For>
              </Show>
            </Show>
          </Show>
        </div>
      </div>

      {/* Transient notification. Ward can't reload Claude Code itself, so every
          write ends here noting a restart may be needed, with an Undo bound to
          the returned RestoreInfo. */}
      <Show when={toast()} keyed>
        {(t) => (
          <div class="set-toast" data-testid="settings-toast" role="status">
            <span class="set-toast-msg">{t}</span>
            <Show when={undoInfo()}>
              <button
                class="set-toast-undo"
                data-testid="settings-undo"
                disabled={busy()}
                onClick={doUndo}
              >
                Undo
              </button>
            </Show>
            <button
              class="set-toast-x"
              data-testid="settings-toast-dismiss"
              title="Dismiss"
              onClick={() => { setToast(null); setUndoInfo(null); }}
            >
              ×
            </button>
          </div>
        )}
      </Show>

      {/* Complex-value editor. `keyed` reseeds the modal's working state from the
          row every time it opens; the write routes through the same user-scope
          `applySet` (so it gets a toast + Undo). WKWebView's confirm()/prompt()
          are silent no-ops, so this is a real in-app modal. */}
      <Show when={editorRow()} keyed>
        {(row) => (
          <EditorModal
            row={row}
            busy={busy()}
            onSave={(value) => applySet(row, value)}
            onClose={() => setEditorRow(null)}
          />
        )}
      </Show>
    </div>
  );
}

/** One catalog row: label + description + source chip on top, then an inline
 *  editor (by valueType) beside the effective/default readout and — for a
 *  user-set, non-managed row — a Reset-to-default button. */
function SettingRowView(props: {
  row: SettingRow;
  busy: boolean;
  onSet: (row: SettingRow, value: unknown) => void;
  onReset: (row: SettingRow) => void;
  onEdit: (row: SettingRow) => void;
}) {
  const row = () => props.row;
  const def = () => props.row.def;
  const managed = () => isManaged(props.row);
  const eff = () => effectiveOf(props.row);
  // Reset only makes sense for a value a user actually set — never for an unset
  // row (nothing to clear) or a managed one (user scope can't override it).
  const canReset = () => props.row.isSet && !managed();

  return (
    <article class="set-row" data-testid="setting-row" data-key={def().key}>
      <div class="set-row-head">
        <div class="set-row-titles">
          <span class="set-row-label">{def().label}</span>
          <code class="set-row-key">{def().key}</code>
        </div>
        <div class="set-row-badges">
          <Show when={managed()}>
            <span class="set-managed" data-testid="setting-managed" title="Managed by enterprise settings — read-only">
              Managed · read-only
            </span>
          </Show>
          <span
            class="set-source"
            data-testid="setting-source"
            classList={{
              'is-user': row().sourceScope === 'user',
              'is-managed': row().sourceScope === 'managed',
              'is-default': !row().sourceScope || row().sourceScope === 'default',
            }}
            title="Which scope set the effective value"
          >
            {row().sourceScope ?? 'default'}
          </span>
        </div>
      </div>

      <p class="set-row-desc">{def().description}</p>

      <div class="set-row-ctl">
        <div class="set-editor">
          <SettingEditor row={props.row} busy={props.busy} onSet={props.onSet} onEdit={props.onEdit} />
        </div>

        <div class="set-row-meta">
          <span class="set-value-wrap">
            <span class="set-value-k">Now</span>
            <span class="set-value" data-testid="setting-value">{formatValue(eff(), def().valueType)}</span>
          </span>
          <Show when={def().default !== undefined}>
            <span class="set-default" data-testid="setting-default" title="Documented default">
              default: {formatValue(def().default, def().valueType)}
            </span>
          </Show>
          <span class="set-target" title="Where Ward writes this setting">
            {targetFileLabel(def().targetFile)}
          </span>
        </div>

        <Show when={canReset()}>
          <button
            type="button"
            class="set-reset"
            data-testid="setting-reset"
            disabled={props.busy}
            title="Reset to default (clears the user-scope value)"
            onClick={() => props.onReset(props.row)}
          >
            Reset
          </button>
        </Show>
      </div>
    </article>
  );
}

/** The inline editor for a row, dispatched on `def.valueType`. Simple scalars
 *  edit in place (bool → toggle, enum → select, number/string → input) and are
 *  disabled while a write is in-flight or the row is managed. Complex
 *  array/object settings surface an inert "Edit…" button (a later task wires the
 *  bespoke editors). */
function SettingEditor(props: {
  row: SettingRow;
  busy: boolean;
  onSet: (row: SettingRow, value: unknown) => void;
  onEdit: (row: SettingRow) => void;
}) {
  const def = () => props.row.def;
  const managed = () => isManaged(props.row);
  const disabled = () => props.busy || managed();
  const eff = () => effectiveOf(props.row);

  // Which complex-value rows have a working modal editor today: every `array`
  // plus the `env` (key/value) and generic `json` object editors. The four
  // bespoke object editors (permissions / hooks / sandbox / statusLine) land in
  // Tasks 13-14 and stay inert; managed rows are never user-editable.
  const canEditComplex = () =>
    !managed() &&
    (def().valueType === 'array' ||
      (def().valueType === 'object' && (def().editor === 'env' || def().editor === 'json')));

  return (
    <Show when={def().valueType === 'bool'} fallback={
      <Show when={def().valueType === 'enum'} fallback={
        <Show when={def().valueType === 'number'} fallback={
          <Show when={def().valueType === 'string'} fallback={
            // array / object — an "Edit…" affordance that opens a modal editor
            // for array / env / json rows; the bespoke object editors and managed
            // rows stay inert (a disabled button that never opens anything).
            <Show
              when={canEditComplex()}
              fallback={
                <button
                  type="button"
                  class="set-edit"
                  data-testid="setting-edit"
                  disabled
                  title="Editor coming in a later step"
                >
                  Edit…
                </button>
              }
            >
              <button
                type="button"
                class="set-edit is-active"
                data-testid="setting-edit"
                disabled={props.busy}
                title={`Edit ${def().label}`}
                onClick={() => props.onEdit(props.row)}
              >
                Edit…
              </button>
            </Show>
          }>
            <input
              class="set-text"
              data-testid="setting-text"
              type="text"
              disabled={disabled()}
              value={eff() === undefined || eff() === null ? '' : String(eff())}
              onChange={(e) => props.onSet(props.row, e.currentTarget.value)}
            />
          </Show>
        }>
          <input
            class="set-number"
            data-testid="setting-number"
            type="number"
            disabled={disabled()}
            value={eff() === undefined || eff() === null ? '' : String(eff())}
            onChange={(e) => {
              const raw = e.currentTarget.value;
              // Blank clears back toward default via Reset, not an invalid NaN write.
              if (raw.trim() === '') return;
              const n = Number(raw);
              if (Number.isNaN(n)) return;
              props.onSet(props.row, n);
            }}
          />
        </Show>
      }>
        <select
          class="set-enum"
          data-testid="setting-enum"
          disabled={disabled()}
          value={eff() === undefined || eff() === null ? '' : String(eff())}
          onChange={(e) => props.onSet(props.row, e.currentTarget.value)}
        >
          {/* When the effective value isn't one of the enum options (or is
              unset), a leading placeholder keeps the select coherent. */}
          <Show when={!def().enumValues.includes(String(eff() ?? ''))}>
            <option value="" disabled>Choose…</option>
          </Show>
          <For each={def().enumValues}>{(v) => <option value={v}>{v}</option>}</For>
        </select>
      </Show>
    }>
      <button
        type="button"
        class="set-toggle"
        role="switch"
        aria-checked={eff() === true ? 'true' : 'false'}
        aria-label={`Toggle ${def().label}`}
        data-testid="setting-toggle"
        disabled={disabled()}
        title={managed() ? 'Managed — read-only' : eff() === true ? 'Turn off' : 'Turn on'}
        onClick={() => props.onSet(props.row, !(eff() === true))}
      >
        <span class="set-toggle-track"><span class="set-toggle-knob" /></span>
      </button>
    </Show>
  );
}

/** Modal editor for a complex-value setting — dispatched on the row:
 *
 *   • `array`            → an add/remove list of string entries.
 *   • object `editor='env'`  → a NAME/value key-value table.
 *   • object `editor='json'` → a raw JSON textarea, parsed (and validated)
 *                              before it can save.
 *
 *  The four bespoke object editors (permissions / hooks / sandbox / statusLine)
 *  never reach here — their row keeps the inert "Edit…" button until Tasks 13-14.
 *  Working state is seeded once from `row.effective` (the modal is rendered
 *  `keyed`, so re-opening reseeds). On save the composed value routes back
 *  through the caller's user-scope `applySet`, which re-reads the catalog and
 *  shows the toast + Undo. Shares the global `.modal` shell (WKWebView's
 *  confirm()/prompt() are silent no-ops) with Esc-to-cancel, a focus trap, and
 *  focus restoration to the trigger. */
function EditorModal(props: {
  row: SettingRow;
  busy: boolean;
  onSave: (value: unknown) => void | Promise<void>;
  onClose: () => void;
}) {
  const def = () => props.row.def;
  const mode: 'array' | 'env' | 'json' =
    def().valueType === 'array' ? 'array' : def().editor === 'env' ? 'env' : 'json';

  // ── Working state, seeded once from the row's effective value ──
  const seedEntries = (): string[] => {
    const v = props.row.effective;
    return Array.isArray(v) ? v.map((x) => (typeof x === 'string' ? x : JSON.stringify(x))) : [];
  };
  const seedPairs = (): { name: string; value: string }[] => {
    const v = props.row.effective;
    if (v && typeof v === 'object' && !Array.isArray(v)) {
      return Object.entries(v as Record<string, unknown>).map(([name, val]) => ({
        name,
        value: typeof val === 'string' ? val : JSON.stringify(val),
      }));
    }
    return [];
  };
  const seedJson = (): string => JSON.stringify(props.row.effective ?? {}, null, 2);

  const [entries, setEntries] = createSignal<string[]>(seedEntries());
  const [draft, setDraft] = createSignal(''); // the array add-row input
  const [pairs, setPairs] = createSignal<{ name: string; value: string }[]>(seedPairs());
  const [text, setText] = createSignal(seedJson());
  const [jsonErr, setJsonErr] = createSignal<string | null>(null);

  // ── Array ops ──
  function addEntry() {
    const val = draft().trim();
    if (!val) return;
    setEntries([...entries(), val]);
    setDraft('');
  }
  const removeEntry = (i: number) => setEntries(entries().filter((_, idx) => idx !== i));

  // ── Env ops (an <Index> keeps the inputs mounted so typing never loses focus) ──
  const addPair = () => setPairs([...pairs(), { name: '', value: '' }]);
  const updatePair = (i: number, field: 'name' | 'value', v: string) =>
    setPairs(pairs().map((p, idx) => (idx === i ? { ...p, [field]: v } : p)));
  const removePair = (i: number) => setPairs(pairs().filter((_, idx) => idx !== i));
  function composeEnv(): Record<string, string> {
    const out: Record<string, string> = {};
    for (const p of pairs()) {
      const name = p.name.trim();
      if (name) out[name] = p.value; // last write wins on a duplicate name
    }
    return out;
  }

  // ── Commit paths (managed rows never open this modal, so no guard here) ──
  async function commit(value: unknown) {
    await props.onSave(value);
    props.onClose();
  }
  function save() {
    if (mode === 'array') {
      void commit(entries());
    } else if (mode === 'env') {
      void commit(composeEnv());
    } else {
      let parsed: unknown;
      try {
        parsed = JSON.parse(text());
      } catch (e) {
        // Block the write and surface the parse error inline (never write bad JSON).
        setJsonErr(e instanceof Error ? e.message : 'Invalid JSON');
        return;
      }
      setJsonErr(null);
      void commit(parsed);
    }
  }

  const title = () =>
    mode === 'array' ? 'Edit list' : mode === 'env' ? 'Edit environment variables' : 'Edit JSON';
  const testid = () =>
    mode === 'array' ? 'setting-array-editor' : mode === 'env' ? 'setting-env-editor' : 'setting-json-editor';

  // ── Modal a11y: Esc cancels, Tab is trapped, focus starts inside + restores ──
  let dialogRef: HTMLDivElement | undefined;
  let firstFocus: HTMLElement | undefined;
  createEffect(() => {
    const prevFocused = document.activeElement as HTMLElement | null;
    queueMicrotask(() =>
      (firstFocus ?? dialogRef?.querySelector<HTMLElement>('button, input, textarea, select'))?.focus(),
    );
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        props.onClose();
      } else if (e.key === 'Tab') {
        const nodes = dialogRef?.querySelectorAll<HTMLElement>(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        );
        if (!nodes || nodes.length === 0) return;
        const list = Array.from(nodes).filter((n) => !(n as HTMLButtonElement).disabled);
        if (list.length === 0) return;
        const first = list[0];
        const last = list[list.length - 1];
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
      // Restore focus to the trigger; a re-render may have removed it — never throw.
      if (prevFocused && document.contains(prevFocused) && typeof prevFocused.focus === 'function') {
        prevFocused.focus();
      }
    });
  });

  return (
    <div class="modal-overlay" data-testid="settings-modal-overlay" onClick={() => props.onClose()}>
      <div
        ref={dialogRef}
        class="modal set-editor-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-editor-title"
        data-testid={testid()}
        onClick={(e) => e.stopPropagation()}
      >
        <div class="modal-title is-info" id="settings-editor-title">{title()}</div>
        <code class="set-editor-key">{def().key}</code>

        <div class="modal-body set-editor-body">
          {/* ── Array: add/remove string entries ── */}
          <Show when={mode === 'array'}>
            <div class="set-arr" data-testid="setting-array">
              <Show
                when={entries().length > 0}
                fallback={<p class="set-arr-empty">No entries yet. Add one below.</p>}
              >
                <ul class="set-arr-list">
                  <For each={entries()}>
                    {(entry, i) => (
                      <li class="set-arr-item" data-testid="setting-array-item">
                        <span class="set-arr-val">{entry}</span>
                        <button
                          type="button"
                          class="set-arr-rm"
                          data-testid="setting-array-remove"
                          title="Remove entry"
                          disabled={props.busy}
                          onClick={() => removeEntry(i())}
                        >
                          ×
                        </button>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
              <div class="set-arr-add">
                <input
                  ref={(el) => (firstFocus = el)}
                  class="set-arr-input"
                  data-testid="setting-array-input"
                  type="text"
                  placeholder="Add an entry…"
                  value={draft()}
                  disabled={props.busy}
                  onInput={(e) => setDraft(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      addEntry();
                    }
                  }}
                />
                <button
                  type="button"
                  class="btn set-arr-addbtn"
                  data-testid="setting-array-add"
                  disabled={props.busy || !draft().trim()}
                  onClick={addEntry}
                >
                  Add
                </button>
              </div>
            </div>
          </Show>

          {/* ── Env: NAME/value key-value table ── */}
          <Show when={mode === 'env'}>
            <div class="set-env" data-testid="setting-env">
              <Show
                when={pairs().length > 0}
                fallback={<p class="set-arr-empty">No variables yet. Add one below.</p>}
              >
                <div class="set-env-table">
                  <Index each={pairs()}>
                    {(p, i) => (
                      <div class="set-env-row" data-testid="setting-env-row">
                        <input
                          ref={(el) => { if (i === 0) firstFocus = el; }}
                          class="set-env-name"
                          data-testid="setting-env-name"
                          type="text"
                          placeholder="NAME"
                          value={p().name}
                          disabled={props.busy}
                          onInput={(e) => updatePair(i, 'name', e.currentTarget.value)}
                        />
                        <input
                          class="set-env-value"
                          data-testid="setting-env-value"
                          type="text"
                          placeholder="value"
                          value={p().value}
                          disabled={props.busy}
                          onInput={(e) => updatePair(i, 'value', e.currentTarget.value)}
                        />
                        <button
                          type="button"
                          class="set-arr-rm"
                          data-testid="setting-env-remove"
                          title="Remove variable"
                          disabled={props.busy}
                          onClick={() => removePair(i)}
                        >
                          ×
                        </button>
                      </div>
                    )}
                  </Index>
                </div>
              </Show>
              <button
                type="button"
                class="btn set-env-add"
                data-testid="setting-env-add"
                disabled={props.busy}
                onClick={addPair}
              >
                + Add variable
              </button>
            </div>
          </Show>

          {/* ── JSON: raw textarea, parsed + validated before save ── */}
          <Show when={mode === 'json'}>
            <div class="set-json">
              <textarea
                ref={(el) => (firstFocus = el)}
                class="set-json-area"
                data-testid="setting-json-textarea"
                spellcheck={false}
                disabled={props.busy}
                value={text()}
                onInput={(e) => {
                  setText(e.currentTarget.value);
                  if (jsonErr()) setJsonErr(null); // typing clears the stale parse error
                }}
              />
              <Show when={jsonErr()}>
                <p class="set-json-err" data-testid="setting-json-error">
                  <span>Invalid JSON: {jsonErr()}</span>
                </p>
              </Show>
            </div>
          </Show>
        </div>

        <div class="modal-actions">
          <button
            type="button"
            class="btn btn-ghost"
            data-testid="settings-editor-cancel"
            disabled={props.busy}
            onClick={() => props.onClose()}
          >
            Cancel
          </button>
          <button
            type="button"
            class="btn btn-primary"
            data-testid="settings-editor-save"
            disabled={props.busy}
            onClick={save}
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
