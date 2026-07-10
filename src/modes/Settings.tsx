import { createEffect, createMemo, createResource, createSignal, For, Index, onCleanup, Show } from 'solid-js';
import type { EnvVarDef, RestoreInfo, SchemaDiff, ScanResult, SettingRow } from '../api';
import '../styles/settings.css';

/** Plan 29 — Settings mode.
 *
 * A curated, searchable browser over Claude Code's `settings.json` (+ a handful
 * of `~/.claude.json` global-config keys) that the published JSON Schema can't
 * give you on its own: every row carries a human label + description, the
 * **effective value** resolved across the scope chain, a **source-scope chip**
 * (which scope actually set it), and an inline editor picked by the setting's
 * `valueType`. Simple scalars (bool / enum / number / string) edit in place;
 * complex `array` / `object` settings open a modal editor — the generic array /
 * env / json editors plus the bespoke permissions / statusLine / sandbox /
 * hooks editors. Every array/object row is now editable; only managed rows stay
 * read-only.
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
  /** Task 15 — the schema-drift tripwire: keys the published JSON Schema lists
   *  that Ward's catalog lacks (add these) + keys Ward curates that the schema
   *  omits. A user-triggered network call, so it's fetched on demand, not on
   *  mount. */
  schemaDiff: () => Promise<SchemaDiff>;
  /** Task 15 — curated env-var metadata (name/description/category) for the
   *  discovery panel. Editing one writes into the `env` object of settings.json
   *  via the existing env editor; the value itself is never collected here. */
  envList: () => Promise<EnvVarDef[]>;
}

/** Plan 29 — every write is fixed to USER scope (see the module doc). The Rust
 *  `settings_set`/`settings_unset` commands accept a scope string, so
 *  project/local lands later without churn. */
const SETTINGS_SCOPE = 'user';

/** Object `editor` kinds that have a working modal editor. All object editors
 *  are wired now — Task 14 added `hooks`, the last and most nested one. */
const EDITABLE_OBJECT_EDITORS = ['env', 'json', 'permissions', 'statusLine', 'sandbox', 'hooks'];

/** `permissions.defaultMode` options (Claude Code's permission modes). The empty
 *  choice leaves the key unset so it inherits from a broader scope. */
const PERM_MODES = ['default', 'acceptEdits', 'plan', 'auto', 'dontAsk', 'bypassPermissions'];

/** The `sandbox.filesystem.*` path lists Ward edits — the real Claude Code
 *  sandbox schema (read/write × allow/deny), not a generic allow/deny pair. Any
 *  other filesystem sub-key present in the value is preserved via merge. */
const SANDBOX_FS_FIELDS: { key: string; label: string; placeholder: string }[] = [
  { key: 'allowRead', label: 'Allow read paths', placeholder: 'e.g. ~/.kube' },
  { key: 'allowWrite', label: 'Allow write paths', placeholder: 'e.g. /tmp/build' },
  { key: 'denyRead', label: 'Deny read paths', placeholder: 'e.g. ~/.aws/credentials' },
  { key: 'denyWrite', label: 'Deny write paths', placeholder: 'e.g. ~/.ssh' },
];

/** The `sandbox.network.*` domain lists Ward edits (Claude Code's real field
 *  names). Other network sub-keys (allowUnixSockets, allowLocalBinding, …) are
 *  preserved via merge. */
const SANDBOX_NET_FIELDS: { key: string; label: string; placeholder: string }[] = [
  { key: 'allowedDomains', label: 'Allowed domains', placeholder: 'e.g. github.com, *.npmjs.org' },
  { key: 'deniedDomains', label: 'Denied domains', placeholder: 'e.g. uploads.github.com' },
];

/** Claude Code's documented hook events — offered in the hooks editor's
 *  "Add event" picker, in the order Claude Code lists them. A free-text field
 *  alongside accepts any other event name, so a newly-added CC event (or a
 *  custom one) works before Ward's list catches up. */
const KNOWN_HOOK_EVENTS = [
  'PreToolUse', 'PostToolUse', 'UserPromptSubmit', 'Notification',
  'Stop', 'SubagentStop', 'PreCompact', 'SessionStart', 'SessionEnd',
];

/** Editable working shape for the hooks editor: an ordered list of events, each
 *  an ordered list of matcher-groups, each an ordered list of command entries.
 *  `timeout` is held as the raw input string and only parsed to a number (or
 *  omitted) at compose time — mirroring the number-input handling elsewhere.
 *
 *  Preserve-everything: Ward reads more than `type:"command"` hooks (its own
 *  `scan_hooks` models `type:"http"` + `url`), so entries this editor can't edit
 *  are carried opaquely and re-emitted verbatim — opening + saving must never
 *  destroy them. `passthrough` holds a non-command entry as-is (the row renders
 *  read-only). `orig` holds a command entry's source object so unknown extra
 *  fields survive the round-trip (the composed `{type,command,timeout}` is merged
 *  onto it). An entry has at most one of these set. */
interface HookCmdDraft { command: string; timeout: string; passthrough?: unknown; orig?: Record<string, unknown> }
interface HookGroupDraft { matcher: string; commands: HookCmdDraft[] }
interface HookEventDraft { event: string; groups: HookGroupDraft[] }

/** A compact read-only label for a preserved non-command hook entry (e.g.
 *  `http → https://…`), shown so the user knows it exists and is kept as-is. */
function passthroughLabel(v: unknown): string {
  if (v && typeof v === 'object' && !Array.isArray(v)) {
    const o = v as Record<string, unknown>;
    const type = typeof o.type === 'string' ? o.type : 'hook';
    const target = typeof o.url === 'string' ? o.url : typeof o.command === 'string' ? o.command : '';
    return target ? `${type} → ${target}` : type;
  }
  try { return JSON.stringify(v); } catch { return String(v); }
}

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
  // Task 15 — when the env-var discovery panel's "Add to env" opens the shared
  // `env` object editor, this carries the variable NAME to pre-seed as a new
  // (blank-value) row. Null for a normal env edit. Cleared whenever the modal
  // closes or any other row's editor opens, so a stale seed never leaks in.
  const [envSeedName, setEnvSeedName] = createSignal<string | null>(null);

  const rows = () => catalog() ?? [];

  // Open the shared `env` object editor pre-seeded with `name` (from the env-var
  // discovery panel). Finds the catalog's env row (editor==='env'); no-op if the
  // catalog somehow lacks one. Managed env rows can't be user-edited, so skip.
  function openEnvWith(name: string) {
    const envRow = rows().find((r) => r.def.editor === 'env');
    if (!envRow || isManaged(envRow)) return;
    setEnvSeedName(name);
    setEditorRow(envRow);
  }
  // Open a row's editor via the normal "Edit…" path — always with a clean seed.
  function openEditor(row: SettingRow) {
    setEnvSeedName(null);
    setEditorRow(row);
  }
  function closeEditor() {
    setEditorRow(null);
    setEnvSeedName(null);
  }

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
                      onEdit={openEditor}
                    />
                  )}
                </For>
              </Show>

              {/* Tools region — the schema-drift tripwire + the env-var
                  discovery panel. Rendered below the filtered list but
                  independent of the active category / search, so both stay
                  reachable once the catalog has loaded. */}
              <div class="set-tools" data-testid="settings-tools">
                <SchemaDiffPanel api={props.api} />
                <EnvVarPanel api={props.api} onAdd={openEnvWith} />
              </div>
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
            seedEnvName={envSeedName()}
            onSave={(value) => applySet(row, value)}
            onClose={closeEditor}
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

  // Which complex-value rows have a working modal editor: every `array` plus the
  // object editors `env` (key/value), generic `json`, and the bespoke editors
  // `permissions` / `statusLine` / `sandbox` / `hooks`. Every array/object row
  // is editable now; only managed rows are never user-editable.
  const canEditComplex = () =>
    !managed() &&
    (def().valueType === 'array' ||
      (def().valueType === 'object' && EDITABLE_OBJECT_EDITORS.includes(def().editor ?? '')));

  return (
    <Show when={def().valueType === 'bool'} fallback={
      <Show when={def().valueType === 'enum'} fallback={
        <Show when={def().valueType === 'number'} fallback={
          <Show when={def().valueType === 'string'} fallback={
            // array / object — an "Edit…" affordance that opens a modal editor
            // for array / env / json / permissions / statusLine / sandbox / hooks
            // rows; only managed rows stay inert (a disabled button that never
            // opens anything).
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

/** A controlled add/remove list of string entries — the shared sub-UI behind
 *  the generic `array` editor AND every array field inside the bespoke
 *  permissions / sandbox editors. The parent owns `entries` + `onChange`; only
 *  the add-row draft is local. `prefix` scopes the data-testids so multiple
 *  instances in one modal stay individually addressable (the generic array
 *  editor passes `setting-array`, preserving its Task-12 testids exactly). */
function ArrayListEditor(props: {
  prefix: string;
  entries: string[];
  onChange: (next: string[]) => void;
  busy: boolean;
  label?: string;
  placeholder?: string;
  inputRef?: (el: HTMLInputElement) => void;
  /** Task 15 — registers a getter for the add-row input's current text so the
   *  parent's Save path can fold a half-typed (never-Added) draft into the array
   *  instead of silently dropping it. Called once at setup with a stable getter. */
  pendingDraftRef?: (getDraft: () => string) => void;
}) {
  const [draft, setDraft] = createSignal('');
  // Hand the parent a live getter for the pending add-row text (see Save path).
  props.pendingDraftRef?.(() => draft());
  function add() {
    const val = draft().trim();
    if (!val) return;
    props.onChange([...props.entries, val]);
    setDraft('');
  }
  const remove = (i: number) => props.onChange(props.entries.filter((_, idx) => idx !== i));

  return (
    <div class="set-arr" data-testid={props.prefix}>
      <Show when={props.label}>
        <div class="set-list-label">{props.label}</div>
      </Show>
      <Show
        when={props.entries.length > 0}
        fallback={<p class="set-arr-empty">No entries yet. Add one below.</p>}
      >
        <ul class="set-arr-list">
          <For each={props.entries}>
            {(entry, i) => (
              <li class="set-arr-item" data-testid={`${props.prefix}-item`}>
                <span class="set-arr-val">{entry}</span>
                <button
                  type="button"
                  class="set-arr-rm"
                  data-testid={`${props.prefix}-remove`}
                  title="Remove entry"
                  disabled={props.busy}
                  onClick={() => remove(i())}
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
          ref={(el) => props.inputRef?.(el)}
          class="set-arr-input"
          data-testid={`${props.prefix}-input`}
          type="text"
          placeholder={props.placeholder ?? 'Add an entry…'}
          value={draft()}
          disabled={props.busy}
          onInput={(e) => setDraft(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              add();
            }
          }}
        />
        <button
          type="button"
          class="btn set-arr-addbtn"
          data-testid={`${props.prefix}-add`}
          disabled={props.busy || !draft().trim()}
          onClick={add}
        >
          Add
        </button>
      </div>
    </div>
  );
}

/** Modal editor for a complex-value setting — dispatched on the row:
 *
 *   • `array`            → an add/remove list of string entries.
 *   • object `editor='env'`  → a NAME/value key-value table.
 *   • object `editor='permissions'` → defaultMode select + allow/ask/deny +
 *                              additionalDirectories lists → the `permissions`
 *                              object (empty arrays / unset mode omitted).
 *   • object `editor='statusLine'`  → type + command + optional padding.
 *   • object `editor='sandbox'`     → filesystem + network path/domain lists →
 *                              the nested `{filesystem,network}` object.
 *   • object `editor='json'` → a raw JSON textarea, parsed (and validated)
 *                              before it can save.
 *
 *  Only `hooks` never reaches here — its row keeps the inert "Edit…" button
 *  until Task 14. The three bespoke editors compose their object by MERGING onto
 *  a fresh clone of `row.effective`, so keys they don't manage
 *  (disableBypassPermissionsMode, sandbox.enabled, statusLine.refreshInterval,
 *  …) survive a save. Working state is seeded once from `row.effective` (the
 *  modal is rendered `keyed`, so re-opening reseeds). On save the composed value
 *  routes back through the caller's user-scope `applySet`, which re-reads the
 *  catalog and shows the toast + Undo. Shares the global `.modal` shell
 *  (WKWebView's confirm()/prompt() are silent no-ops) with Esc-to-cancel, a
 *  focus trap, and focus restoration to the trigger. */
function EditorModal(props: {
  row: SettingRow;
  busy: boolean;
  /** Task 15 — for the `env` editor only: a variable name to pre-seed as a new
   *  blank-value row (sent by the env-var discovery panel's "Add to env"). */
  seedEnvName?: string | null;
  onSave: (value: unknown) => void | Promise<void>;
  onClose: () => void;
}) {
  const def = () => props.row.def;
  const d0 = def();
  const mode: 'array' | 'env' | 'json' | 'permissions' | 'statusLine' | 'sandbox' | 'hooks' =
    d0.valueType === 'array'
      ? 'array'
      : d0.editor === 'permissions'
        ? 'permissions'
        : d0.editor === 'statusLine'
          ? 'statusLine'
          : d0.editor === 'sandbox'
            ? 'sandbox'
            : d0.editor === 'hooks'
              ? 'hooks'
              : d0.editor === 'env'
                ? 'env'
                : 'json';

  // A fresh shallow clone of the row's effective object (or {} when it isn't a
  // plain object) — the merge base for the bespoke editors so unmanaged keys
  // survive. Recomputed on demand; `row.effective` is stable while the keyed
  // modal is open.
  const objBase = (): Record<string, unknown> => {
    const v = props.row.effective;
    return v && typeof v === 'object' && !Array.isArray(v) ? { ...(v as Record<string, unknown>) } : {};
  };
  // A string[] read of one key of an object (skips non-arrays; stringifies
  // non-string members defensively).
  const pickArr = (o: Record<string, unknown> | undefined, k: string): string[] => {
    const a = o?.[k];
    return Array.isArray(a) ? a.map((x) => (typeof x === 'string' ? x : JSON.stringify(x))) : [];
  };
  // Set `o[k]` to a non-empty array, or delete the key entirely (never write an
  // empty `[]` — that's the omit-empty rule the tests pin).
  const setOrDeleteArr = (o: Record<string, unknown>, k: string, arr: string[]) => {
    if (arr.length) o[k] = arr;
    else delete o[k];
  };

  // ── Working state, seeded once from the row's effective value ──
  const seedEntries = (): string[] => {
    const v = props.row.effective;
    return Array.isArray(v) ? v.map((x) => (typeof x === 'string' ? x : JSON.stringify(x))) : [];
  };
  const seedPairs = (): { name: string; value: string }[] => {
    const v = props.row.effective;
    const out: { name: string; value: string }[] =
      v && typeof v === 'object' && !Array.isArray(v)
        ? Object.entries(v as Record<string, unknown>).map(([name, val]) => ({
            name,
            value: typeof val === 'string' ? val : JSON.stringify(val),
          }))
        : [];
    // Task 15 — pre-seed a blank-value row for the env var the discovery panel
    // asked to add (unless it's already present), so "Add to env" lands the user
    // on a ready-to-fill row instead of an empty table.
    const seed = props.seedEnvName?.trim();
    if (seed && !out.some((p) => p.name === seed)) out.push({ name: seed, value: '' });
    return out;
  };
  const seedJson = (): string => JSON.stringify(props.row.effective ?? {}, null, 2);

  const [entries, setEntries] = createSignal<string[]>(seedEntries());
  const [pairs, setPairs] = createSignal<{ name: string; value: string }[]>(seedPairs());
  const [text, setText] = createSignal(seedJson());
  const [jsonErr, setJsonErr] = createSignal<string | null>(null);
  // Task 15 — a live getter for the generic array editor's pending add-row text.
  // Registered by that ArrayListEditor; read in save() so a half-typed entry the
  // user never clicked "Add" on is folded into the saved array, not dropped.
  let arrayDraft: () => string = () => '';

  // ── Permissions working state (seeded from the effective permissions object) ──
  const permSeed = objBase();
  const [permDefaultMode, setPermDefaultMode] = createSignal(
    typeof permSeed.defaultMode === 'string' ? permSeed.defaultMode : '',
  );
  const [permAllow, setPermAllow] = createSignal(pickArr(permSeed, 'allow'));
  const [permAsk, setPermAsk] = createSignal(pickArr(permSeed, 'ask'));
  const [permDeny, setPermDeny] = createSignal(pickArr(permSeed, 'deny'));
  const [permDirs, setPermDirs] = createSignal(pickArr(permSeed, 'additionalDirectories'));
  function composePermissions(): Record<string, unknown> {
    const out = objBase(); // preserves disableBypassPermissionsMode / skipDangerousModePermissionPrompt / …
    setOrDeleteArr(out, 'allow', permAllow());
    setOrDeleteArr(out, 'ask', permAsk());
    setOrDeleteArr(out, 'deny', permDeny());
    setOrDeleteArr(out, 'additionalDirectories', permDirs());
    const dm = permDefaultMode().trim();
    if (dm) out.defaultMode = dm;
    else delete out.defaultMode;
    return out;
  }

  // ── Status-line working state ──
  const slSeed = objBase();
  const [slType, setSlType] = createSignal(typeof slSeed.type === 'string' ? slSeed.type : 'command');
  const [slCommand, setSlCommand] = createSignal(typeof slSeed.command === 'string' ? slSeed.command : '');
  const [slPadding, setSlPadding] = createSignal(
    slSeed.padding === undefined || slSeed.padding === null ? '' : String(slSeed.padding),
  );
  function composeStatusLine(): Record<string, unknown> {
    const out = objBase(); // preserves refreshInterval / hideVimModeIndicator / …
    const t = slType().trim();
    if (t) out.type = t;
    else delete out.type;
    const c = slCommand().trim();
    if (c) out.command = c;
    else delete out.command; // symmetric with type/padding — never persist statusLine.command: ""
    const p = slPadding().trim();
    if (p === '') delete out.padding;
    else {
      const n = Number(p);
      if (Number.isNaN(n)) delete out.padding;
      else out.padding = n;
    }
    return out;
  }

  // ── Sandbox working state (one list per filesystem/network field, keyed by a
  //     flat `section.key` path so a single signal drives the whole editor) ──
  const seedSandbox = (): Record<string, string[]> => {
    const base = objBase();
    const sub = (k: string): Record<string, unknown> => {
      const v = base[k];
      return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
    };
    const fs = sub('filesystem');
    const net = sub('network');
    const out: Record<string, string[]> = {};
    for (const f of SANDBOX_FS_FIELDS) out[`filesystem.${f.key}`] = pickArr(fs, f.key);
    for (const f of SANDBOX_NET_FIELDS) out[`network.${f.key}`] = pickArr(net, f.key);
    return out;
  };
  const [sandboxLists, setSandboxLists] = createSignal<Record<string, string[]>>(seedSandbox());
  const updateSandbox = (path: string, next: string[]) =>
    setSandboxLists({ ...sandboxLists(), [path]: next });
  function composeSandbox(): Record<string, unknown> {
    const out = objBase(); // preserves enabled / excludedCommands / autoAllowBashIfSandboxed / …
    const cloneSub = (k: string): Record<string, unknown> => {
      const v = out[k];
      return v && typeof v === 'object' && !Array.isArray(v) ? { ...(v as Record<string, unknown>) } : {};
    };
    const fs = cloneSub('filesystem'); // preserves other filesystem sub-keys
    const net = cloneSub('network'); // preserves allowUnixSockets / allowLocalBinding / …
    const lists = sandboxLists();
    for (const f of SANDBOX_FS_FIELDS) setOrDeleteArr(fs, f.key, lists[`filesystem.${f.key}`] ?? []);
    for (const f of SANDBOX_NET_FIELDS) setOrDeleteArr(net, f.key, lists[`network.${f.key}`] ?? []);
    if (Object.keys(fs).length) out.filesystem = fs;
    else delete out.filesystem;
    if (Object.keys(net).length) out.network = net;
    else delete out.network;
    return out;
  }

  // ── Hooks working state (event → matcher-group → command list) ──
  // Seed every parseable event present in `row.effective` into an editable
  // draft tree (so unknown-named events render as sections too). Non-array /
  // malformed event values are skipped here and survive via the objBase() clone
  // in composeHooks — nothing is ever dropped silently.
  const seedHooks = (): HookEventDraft[] => {
    const v = props.row.effective;
    if (!v || typeof v !== 'object' || Array.isArray(v)) return [];
    const out: HookEventDraft[] = [];
    for (const [event, groupsVal] of Object.entries(v as Record<string, unknown>)) {
      if (!Array.isArray(groupsVal)) continue;
      const groups: HookGroupDraft[] = [];
      for (const g of groupsVal) {
        if (!g || typeof g !== 'object' || Array.isArray(g)) continue;
        const go = g as Record<string, unknown>;
        const matcher = typeof go.matcher === 'string' ? go.matcher : '';
        const cmdsVal = Array.isArray(go.hooks) ? go.hooks : [];
        const commands: HookCmdDraft[] = [];
        for (const c of cmdsVal) {
          if (!c || typeof c !== 'object' || Array.isArray(c)) continue;
          const co = c as Record<string, unknown>;
          // Editable only when it's a command hook with a string command; every
          // other shape (http/url, or an unknown/malformed entry) is preserved
          // verbatim as passthrough so a round-trip never drops it.
          const isCommand = (co.type === undefined || co.type === 'command') && typeof co.command === 'string';
          if (isCommand) {
            const timeout = co.timeout === undefined || co.timeout === null ? '' : String(co.timeout);
            commands.push({ command: co.command as string, timeout, orig: { ...co } });
          } else {
            commands.push({ command: '', timeout: '', passthrough: c });
          }
        }
        groups.push({ matcher, commands });
      }
      out.push({ event, groups });
    }
    return out;
  };
  const [hookEvents, setHookEvents] = createSignal<HookEventDraft[]>(seedHooks());
  // The "Add event" controls: a picker over the not-yet-present known events and
  // a free-text field for any other event name (the field wins when non-empty).
  const [hookPick, setHookPick] = createSignal('');
  const [hookCustom, setHookCustom] = createSignal('');
  const hookEventNames = () => hookEvents().map((e) => e.event);
  const availableHookEvents = () => KNOWN_HOOK_EVENTS.filter((e) => !hookEventNames().includes(e));
  // The select's live value: the user's pick when still available, else the first
  // available known event (keeps the control coherent as events are added).
  const effectiveHookPick = () => {
    const avail = availableHookEvents();
    const p = hookPick();
    return p && avail.includes(p) ? p : (avail[0] ?? '');
  };

  // ── Immutable, position-indexed hook mutators (paired with <Index> rendering
  //    so text inputs stay mounted and never lose focus mid-edit) ──
  const updateHookEvent = (ei: number, fn: (e: HookEventDraft) => HookEventDraft) =>
    setHookEvents(hookEvents().map((e, i) => (i === ei ? fn(e) : e)));
  const removeHookEvent = (ei: number) => setHookEvents(hookEvents().filter((_, i) => i !== ei));
  function addHookEvent() {
    const custom = hookCustom().trim();
    const name = custom || effectiveHookPick();
    if (!name) return;
    setHookCustom('');
    setHookPick('');
    if (hookEventNames().includes(name)) return; // never a duplicate section
    // Seed a fresh event with one group + one empty command so the matcher and
    // command inputs are immediately present (the common add-then-type flow).
    setHookEvents([
      ...hookEvents(),
      { event: name, groups: [{ matcher: '', commands: [{ command: '', timeout: '' }] }] },
    ]);
  }
  const addHookGroup = (ei: number) =>
    updateHookEvent(ei, (e) => ({
      ...e,
      groups: [...e.groups, { matcher: '', commands: [{ command: '', timeout: '' }] }],
    }));
  const removeHookGroup = (ei: number, gi: number) =>
    updateHookEvent(ei, (e) => ({ ...e, groups: e.groups.filter((_, i) => i !== gi) }));
  const setHookMatcher = (ei: number, gi: number, matcher: string) =>
    updateHookEvent(ei, (e) => ({
      ...e,
      groups: e.groups.map((g, i) => (i === gi ? { ...g, matcher } : g)),
    }));
  const addHookCommand = (ei: number, gi: number) =>
    updateHookEvent(ei, (e) => ({
      ...e,
      groups: e.groups.map((g, i) =>
        i === gi ? { ...g, commands: [...g.commands, { command: '', timeout: '' }] } : g,
      ),
    }));
  const removeHookCommand = (ei: number, gi: number, ci: number) =>
    updateHookEvent(ei, (e) => ({
      ...e,
      groups: e.groups.map((g, i) =>
        i === gi ? { ...g, commands: g.commands.filter((_, k) => k !== ci) } : g,
      ),
    }));
  const setHookCommandField = (ei: number, gi: number, ci: number, field: 'command' | 'timeout', val: string) =>
    updateHookEvent(ei, (e) => ({
      ...e,
      groups: e.groups.map((g, i) =>
        i === gi
          ? { ...g, commands: g.commands.map((c, k) => (k === ci ? { ...c, [field]: val } : c)) }
          : g,
      ),
    }));

  // Compose the exact CC hooks shape. Start from a clone of the effective object
  // so any unmanaged/malformed top-level key survives, then overwrite (or delete)
  // each event we manage. Emptied structures cascade-omit: an empty command entry
  // → dropped; a group with no (command or passthrough) entries → dropped; an
  // event with no groups → deleted. An empty matcher is written as key-absence
  // (CC accepts an absent matcher); a blank/NaN timeout is omitted; a present
  // timeout is a number. Passthrough (non-command) entries are re-emitted
  // verbatim in their original position, so a group/event that holds ONLY
  // passthrough entries still survives; unknown fields on a command entry are
  // preserved by merging the composed fields onto its original object.
  function composeHooks(): Record<string, unknown> {
    const out = objBase();
    for (const ev of hookEvents()) {
      const name = ev.event.trim();
      if (!name) continue; // an object can't be keyed on an empty string
      const groups: unknown[] = [];
      for (const g of ev.groups) {
        const cmds: unknown[] = [];
        for (const c of g.commands) {
          if (c.passthrough !== undefined) {
            cmds.push(c.passthrough); // preserved verbatim (e.g. an http hook)
            continue;
          }
          const command = c.command.trim();
          if (!command) continue;
          const entry: Record<string, unknown> = c.orig ? { ...c.orig } : {};
          entry.type = 'command';
          entry.command = command;
          const t = c.timeout.trim();
          const n = t === '' ? NaN : Number(t);
          if (Number.isNaN(n)) delete entry.timeout;
          else entry.timeout = n;
          cmds.push(entry);
        }
        if (!cmds.length) continue;
        const group: Record<string, unknown> = {};
        const matcher = g.matcher.trim();
        if (matcher) group.matcher = matcher;
        group.hooks = cmds;
        groups.push(group);
      }
      if (groups.length) out[name] = groups;
      else delete out[name];
    }
    return out;
  }

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
      // Fold a half-typed add-row draft (never clicked "Add") into the array so
      // clicking Save straight after typing doesn't silently lose it.
      const pending = arrayDraft().trim();
      void commit(pending ? [...entries(), pending] : entries());
    } else if (mode === 'env') {
      void commit(composeEnv());
    } else if (mode === 'permissions') {
      void commit(composePermissions());
    } else if (mode === 'statusLine') {
      void commit(composeStatusLine());
    } else if (mode === 'sandbox') {
      void commit(composeSandbox());
    } else if (mode === 'hooks') {
      void commit(composeHooks());
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
    mode === 'array'
      ? 'Edit list'
      : mode === 'env'
        ? 'Edit environment variables'
        : mode === 'permissions'
          ? 'Edit permissions'
          : mode === 'statusLine'
            ? 'Edit status line'
            : mode === 'sandbox'
              ? 'Edit sandbox'
              : mode === 'hooks'
                ? 'Edit hooks'
                : 'Edit JSON';
  const testid = () =>
    mode === 'array'
      ? 'setting-array-editor'
      : mode === 'env'
        ? 'setting-env-editor'
        : mode === 'permissions'
          ? 'setting-perms-editor'
          : mode === 'statusLine'
            ? 'setting-statusline-editor'
            : mode === 'sandbox'
              ? 'setting-sandbox-editor'
              : mode === 'hooks'
                ? 'setting-hooks-editor'
                : 'setting-json-editor';

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
    // A data-entry modal: a backdrop click must NOT dismiss it (that would drop
    // unsaved edits silently). Close is deliberate — Cancel or Esc only. (Boolean
    // confirm dialogs elsewhere keep backdrop-dismiss; this restriction is
    // scoped to the form editor.)
    <div class="modal-overlay" data-testid="settings-modal-overlay">
      <div
        ref={dialogRef}
        class="modal set-editor-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-editor-title"
        data-testid={testid()}
      >
        <div class="modal-title is-info" id="settings-editor-title">{title()}</div>
        <code class="set-editor-key">{def().key}</code>

        <div class="modal-body set-editor-body">
          {/* ── Array: add/remove string entries (shared list sub-UI) ── */}
          <Show when={mode === 'array'}>
            <ArrayListEditor
              prefix="setting-array"
              entries={entries()}
              onChange={setEntries}
              busy={props.busy}
              inputRef={(el) => (firstFocus = el)}
              pendingDraftRef={(g) => (arrayDraft = g)}
            />
          </Show>

          {/* ── Permissions: defaultMode + allow/ask/deny + additionalDirectories ── */}
          <Show when={mode === 'permissions'}>
            <div class="set-perms" data-testid="setting-perms">
              <label class="set-field">
                <span class="set-field-label">Default mode</span>
                <select
                  class="set-enum"
                  data-testid="setting-perms-defaultmode"
                  disabled={props.busy}
                  value={permDefaultMode()}
                  onChange={(e) => setPermDefaultMode(e.currentTarget.value)}
                >
                  <option value="">Unset (inherit)</option>
                  <For each={PERM_MODES}>{(m) => <option value={m}>{m}</option>}</For>
                </select>
              </label>
              <ArrayListEditor
                prefix="setting-list-allow"
                label="Allow rules"
                entries={permAllow()}
                onChange={setPermAllow}
                busy={props.busy}
                placeholder="e.g. Bash(git status)"
              />
              <ArrayListEditor
                prefix="setting-list-ask"
                label="Ask rules"
                entries={permAsk()}
                onChange={setPermAsk}
                busy={props.busy}
                placeholder="e.g. Bash(git push:*)"
              />
              <ArrayListEditor
                prefix="setting-list-deny"
                label="Deny rules"
                entries={permDeny()}
                onChange={setPermDeny}
                busy={props.busy}
                placeholder="e.g. Bash(rm *)"
              />
              <ArrayListEditor
                prefix="setting-list-dirs"
                label="Additional directories"
                entries={permDirs()}
                onChange={setPermDirs}
                busy={props.busy}
                placeholder="e.g. ~/shared-context"
              />
            </div>
          </Show>

          {/* ── Status line: type + command + optional padding ── */}
          <Show when={mode === 'statusLine'}>
            <div class="set-sl" data-testid="setting-sl">
              <label class="set-field">
                <span class="set-field-label">Type</span>
                <input
                  class="set-text"
                  data-testid="setting-statusline-type"
                  type="text"
                  disabled={props.busy}
                  value={slType()}
                  onInput={(e) => setSlType(e.currentTarget.value)}
                />
              </label>
              <label class="set-field">
                <span class="set-field-label">Command</span>
                <input
                  class="set-text"
                  data-testid="setting-statusline-command"
                  type="text"
                  placeholder="e.g. npx -y ccstatusline@latest"
                  disabled={props.busy}
                  value={slCommand()}
                  onInput={(e) => setSlCommand(e.currentTarget.value)}
                />
              </label>
              <label class="set-field">
                <span class="set-field-label">Padding (optional)</span>
                <input
                  class="set-number"
                  data-testid="setting-statusline-padding"
                  type="number"
                  disabled={props.busy}
                  value={slPadding()}
                  onInput={(e) => setSlPadding(e.currentTarget.value)}
                />
              </label>
            </div>
          </Show>

          {/* ── Sandbox: filesystem + network path/domain lists (real CC schema) ── */}
          <Show when={mode === 'sandbox'}>
            <div class="set-sandbox" data-testid="setting-sandbox">
              <div class="set-sandbox-group">
                <div class="set-sandbox-group-title">Filesystem</div>
                <For each={SANDBOX_FS_FIELDS}>
                  {(f) => (
                    <ArrayListEditor
                      prefix={`setting-list-fs-${f.key}`}
                      label={f.label}
                      entries={sandboxLists()[`filesystem.${f.key}`] ?? []}
                      onChange={(next) => updateSandbox(`filesystem.${f.key}`, next)}
                      busy={props.busy}
                      placeholder={f.placeholder}
                    />
                  )}
                </For>
              </div>
              <div class="set-sandbox-group">
                <div class="set-sandbox-group-title">Network</div>
                <For each={SANDBOX_NET_FIELDS}>
                  {(f) => (
                    <ArrayListEditor
                      prefix={`setting-list-net-${f.key}`}
                      label={f.label}
                      entries={sandboxLists()[`network.${f.key}`] ?? []}
                      onChange={(next) => updateSandbox(`network.${f.key}`, next)}
                      busy={props.busy}
                      placeholder={f.placeholder}
                    />
                  )}
                </For>
              </div>
            </div>
          </Show>

          {/* ── Hooks: event → matcher-group → command list (nested CC shape) ── */}
          <Show when={mode === 'hooks'}>
            <div class="set-hooks" data-testid="setting-hooks">
              <Show
                when={hookEvents().length > 0}
                fallback={<p class="set-arr-empty">No hooks yet. Add an event below to attach a command.</p>}
              >
                <Index each={hookEvents()}>
                  {(ev, ei) => (
                    <section class="set-hooks-event" data-testid="hooks-event">
                      <div class="set-hooks-event-head">
                        <span class="set-hooks-event-name">{ev().event}</span>
                        <button
                          type="button"
                          class="set-arr-rm"
                          data-testid="hooks-remove-event"
                          title="Remove this event and all its hooks"
                          disabled={props.busy}
                          onClick={() => removeHookEvent(ei)}
                        >
                          ×
                        </button>
                      </div>

                      <div class="set-hooks-groups">
                        <Index each={ev().groups}>
                          {(g, gi) => (
                            <div class="set-hooks-group" data-testid="hooks-group">
                              <div class="set-hooks-group-head">
                                <label class="set-field set-hooks-matcher-field">
                                  <span class="set-field-label">Matcher (optional)</span>
                                  <input
                                    class="set-text"
                                    data-testid="hooks-matcher"
                                    type="text"
                                    placeholder="e.g. Bash or Edit|Write (blank = all)"
                                    disabled={props.busy}
                                    value={g().matcher}
                                    onInput={(e) => setHookMatcher(ei, gi, e.currentTarget.value)}
                                  />
                                </label>
                                <button
                                  type="button"
                                  class="set-arr-rm set-hooks-group-rm"
                                  data-testid="hooks-remove-group"
                                  title="Remove this matcher group"
                                  disabled={props.busy}
                                  onClick={() => removeHookGroup(ei, gi)}
                                >
                                  ×
                                </button>
                              </div>

                              <div class="set-hooks-cmds">
                                <Index each={g().commands}>
                                  {(c, ci) => (
                                    <Show
                                      when={c().passthrough === undefined}
                                      fallback={
                                        <div
                                          class="set-hooks-passthrough"
                                          data-testid="hooks-passthrough"
                                          title="Ward's editor only edits command hooks — this entry is preserved exactly as-is."
                                        >
                                          <span class="set-hooks-passthrough-tag">preserved</span>
                                          <span class="set-hooks-passthrough-label">{passthroughLabel(c().passthrough)}</span>
                                        </div>
                                      }
                                    >
                                      <div class="set-hooks-cmd" data-testid="hooks-command">
                                        <input
                                          class="set-text set-hooks-cmd-input"
                                          data-testid="hooks-command-input"
                                          type="text"
                                          placeholder="Shell command, e.g. echo hi"
                                          disabled={props.busy}
                                          value={c().command}
                                          onInput={(e) => setHookCommandField(ei, gi, ci, 'command', e.currentTarget.value)}
                                        />
                                        <input
                                          class="set-number set-hooks-cmd-timeout"
                                          data-testid="hooks-timeout"
                                          type="number"
                                          placeholder="timeout (s)"
                                          title="Timeout in seconds (optional)"
                                          disabled={props.busy}
                                          value={c().timeout}
                                          onInput={(e) => setHookCommandField(ei, gi, ci, 'timeout', e.currentTarget.value)}
                                        />
                                        <button
                                          type="button"
                                          class="set-arr-rm"
                                          data-testid="hooks-command-remove"
                                          title="Remove this command"
                                          disabled={props.busy}
                                          onClick={() => removeHookCommand(ei, gi, ci)}
                                        >
                                          ×
                                        </button>
                                      </div>
                                    </Show>
                                  )}
                                </Index>
                                <button
                                  type="button"
                                  class="btn set-hooks-addcmd"
                                  data-testid="hooks-add-command"
                                  disabled={props.busy}
                                  onClick={() => addHookCommand(ei, gi)}
                                >
                                  + Add command
                                </button>
                              </div>
                            </div>
                          )}
                        </Index>
                        <button
                          type="button"
                          class="btn set-hooks-addgroup"
                          data-testid="hooks-add-group"
                          disabled={props.busy}
                          onClick={() => addHookGroup(ei)}
                        >
                          + Add matcher group
                        </button>
                      </div>
                    </section>
                  )}
                </Index>
              </Show>

              {/* Add an event: pick a known one or type any custom name. */}
              <div class="set-hooks-addevent" data-testid="hooks-addevent">
                <select
                  class="set-enum"
                  data-testid="hooks-add-event-select"
                  disabled={props.busy || availableHookEvents().length === 0}
                  value={effectiveHookPick()}
                  onChange={(e) => setHookPick(e.currentTarget.value)}
                >
                  <For each={availableHookEvents()}>{(e) => <option value={e}>{e}</option>}</For>
                  <Show when={availableHookEvents().length === 0}>
                    <option value="" disabled>All known events added</option>
                  </Show>
                </select>
                <input
                  class="set-text set-hooks-custom"
                  data-testid="hooks-add-event-custom"
                  type="text"
                  placeholder="or a custom event name"
                  disabled={props.busy}
                  value={hookCustom()}
                  onInput={(e) => setHookCustom(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      addHookEvent();
                    }
                  }}
                />
                <button
                  type="button"
                  class="btn set-hooks-addevent-btn"
                  data-testid="hooks-add-event"
                  disabled={props.busy || (!hookCustom().trim() && !effectiveHookPick())}
                  onClick={addHookEvent}
                >
                  Add event
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

/** Task 15 — the schema-drift tripwire. A user-triggered "Check for new
 *  settings" button fetches `settings_schema_diff` and renders the two drift
 *  lists: keys the published schema lists that Ward's curated catalog lacks
 *  ("add these to the catalog") and keys Ward curates that the schema no longer
 *  lists. An empty result is the all-clear. The call is on demand (it can hit
 *  the network), never on mount, with explicit loading + error states. */
function SchemaDiffPanel(props: { api: SettingsApi }) {
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [result, setResult] = createSignal<SchemaDiff | null>(null);

  async function check() {
    setLoading(true);
    setError(null);
    try {
      setResult(await props.api.schemaDiff());
    } catch (e) {
      setResult(null);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  // Both drift lists empty → the catalog matches the schema (the all-clear).
  const upToDate = () => {
    const r = result();
    return !!r && r.inSchemaNotCatalog.length === 0 && r.inCatalogNotSchema.length === 0;
  };

  return (
    <section class="set-tool-panel set-schema" data-testid="settings-schema-panel">
      <div class="set-tool-head">
        <div class="set-tool-titles">
          <h3 class="set-tool-title">Check for new settings</h3>
          <p class="set-tool-sub">
            Compares Ward's curated catalog against Claude Code's published settings schema.
            Keys the schema adds surface here so they can be added to the catalog.
          </p>
        </div>
        <button
          type="button"
          class="btn set-schema-btn"
          data-testid="settings-schema-diff"
          disabled={loading()}
          onClick={check}
        >
          {loading() ? 'Checking…' : 'Check for new settings'}
        </button>
      </div>

      <Show when={error()}>
        <p class="set-tool-err" data-testid="schema-diff-error">Schema check failed: {error()}</p>
      </Show>

      <Show when={result()}>
        <div class="set-schema-results" data-testid="schema-diff-results">
          <Show
            when={!upToDate()}
            fallback={
              <p class="set-schema-ok" data-testid="schema-diff-ok">
                Catalog is up to date with the published schema.
              </p>
            }
          >
            <Show when={result()!.inSchemaNotCatalog.length > 0}>
              <div class="set-schema-group">
                <div class="set-schema-group-title set-schema-add">
                  New in the schema — add to the catalog ({result()!.inSchemaNotCatalog.length})
                </div>
                <ul class="set-schema-list">
                  <For each={result()!.inSchemaNotCatalog}>
                    {(k) => (
                      <li class="set-schema-item" data-testid="schema-diff-add">
                        <code>{k}</code>
                      </li>
                    )}
                  </For>
                </ul>
              </div>
            </Show>
            <Show when={result()!.inCatalogNotSchema.length > 0}>
              <div class="set-schema-group">
                <div class="set-schema-group-title set-schema-drop">
                  In Ward's catalog but not the schema ({result()!.inCatalogNotSchema.length})
                </div>
                <ul class="set-schema-list">
                  <For each={result()!.inCatalogNotSchema}>
                    {(k) => (
                      <li class="set-schema-item" data-testid="schema-diff-extra">
                        <code>{k}</code>
                      </li>
                    )}
                  </For>
                </ul>
              </div>
            </Show>
          </Show>
        </div>
      </Show>
    </section>
  );
}

/** Task 15 — the env-var discovery panel. A search-driven list of curated
 *  environment variables (name + description + category) that map into the
 *  `env` object of `settings.json`. Each row's "Add to env" calls `onAdd(name)`,
 *  which opens the shared env editor pre-seeded with that name — the value is
 *  entered and written through the normal env editor, never collected here.
 *  Loaded once on mount; the list is bounded + scrollable (the real catalog runs
 *  to dozens of vars) and narrows live as the search box is typed. */
function EnvVarPanel(props: { api: SettingsApi; onAdd: (name: string) => void }) {
  const [list] = createResource(() => props.api.envList());
  const [q, setQ] = createSignal('');
  const rows = () => list() ?? [];
  const filtered = createMemo<EnvVarDef[]>(() => {
    const needle = q().trim().toLowerCase();
    if (!needle) return rows();
    return rows().filter(
      (e) => e.name.toLowerCase().includes(needle) || e.description.toLowerCase().includes(needle),
    );
  });

  return (
    <section class="set-tool-panel set-envpanel" data-testid="settings-env-list">
      <div class="set-tool-head">
        <div class="set-tool-titles">
          <h3 class="set-tool-title">Environment variables</h3>
          <p class="set-tool-sub">
            Common Claude Code environment variables. <strong>Add to env</strong> opens the{' '}
            <code>env</code> editor pre-filled with the name — enter the value there.
          </p>
        </div>
      </div>

      <input
        class="set-search set-env-search"
        data-testid="env-search"
        type="search"
        placeholder="Search environment variables by name or description…"
        value={q()}
        onInput={(e) => setQ(e.currentTarget.value)}
      />

      <Show
        when={!list.loading}
        fallback={<div class="set-status" data-testid="env-loading">Loading environment variables…</div>}
      >
        <Show
          when={!list.error}
          fallback={<div class="set-status err" data-testid="env-error">Failed to load env vars: {String(list.error)}</div>}
        >
          <Show
            when={filtered().length > 0}
            fallback={<p class="set-arr-empty" data-testid="env-empty">No environment variables match your search.</p>}
          >
            <ul class="set-env-vars">
              <For each={filtered()}>
                {(e) => (
                  <li class="set-env-var" data-testid="env-var-row">
                    <div class="set-env-var-main">
                      <div class="set-env-var-top">
                        <code class="set-env-var-name">{e.name}</code>
                        <span class="set-env-var-cat">{e.category}</span>
                      </div>
                      <p class="set-env-var-desc">{e.description}</p>
                    </div>
                    <button
                      type="button"
                      class="btn set-env-var-add"
                      data-testid="env-add"
                      title={`Add ${e.name} to the env setting`}
                      onClick={() => props.onAdd(e.name)}
                    >
                      Add to env
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </Show>
      </Show>
    </section>
  );
}
