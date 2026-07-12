# Ward — Organizer → dedicated-mode signpost handoff (design)

**Date:** 2026-07-12
**Status:** Approved (brainstorming complete; ready for implementation plan)
**Scope:** Small UX / information-architecture fix. No new modes, no backend scan changes.

## Problem

Three config item types appear **twice** in Ward's navigation:

| Type | As an Organizer category (left rail) | As a top-level sidebar mode |
|---|---|---|
| Settings | `setting` — read-only `MetaCard` (`Key / Source / Value`) | **Settings** mode (133-key catalog, effective-value, editors, undo) |
| Plugins | `plugin` — read-only `MetaCard` (name/version/install-path) | **Plugins** mode (Discover/Installed, install/enable, analytics) |
| Sessions | `session` — read-only file card (raw `.jsonl` textarea) | **Sessions** mode (titles, transcript viewer, search) |

The same two words (“Settings”, “Plugins”) — and “Sessions” — sit in both the Organizer's category rail and the main sidebar. The Organizer version is a shallow read-only view that gives **no hint the deep tool exists**. Worse, the Organizer's plugin card actively misdirects: it says *“Toggle it with `/plugin` or by editing `enabledPlugins` in settings.json,”* pointing at the CLI instead of Ward's own Plugins mode.

This is not functional duplication (Organizer = read-only browse, mode = deep editor), but it is an information-architecture smell: **two doors, same label, different depth, no connection between them.**

## Goal

Keep the Organizer as the complete config inventory (every type visible in one place, with counts and the Organizer's unique read-only lens — e.g. the *literal* file value of a setting, including keys absent from the 133-catalog), but turn each of the three duplicated categories into a **signpost** that hands off to its dedicated mode, landing on the exact item.

### Non-goals

- Not removing any category from the Organizer, and not removing any top-level mode.
- No change to what the scanner produces, to scopes (user-scope only, unchanged), or to which modes are Claude-only.
- No change to MCP (its Organizer-vs-Marketplace split is verb-based — manage-existing vs install-new — and is not confusing).

## Design

### 1. One signpost banner in the Organizer detail pane

Add a single **signpost banner** at the top of the Organizer detail pane (`Organizer.tsx`), rendered above whichever body renderer draws the item (`MetaCard` for setting/plugin, the `editor-card` textarea for session). It renders **only when both**:

- the selected item's `category ∈ {setting, plugin, session}`, **and**
- the destination mode is supported for the active harness (capability guard, see §4).

The banner is a short read-only note plus a **primary** button: **“Manage in Settings →” / “Manage in Plugins →” / “Manage in Sessions →”**. Clicking it calls `onNavigate(<mode>, { category, key })`.

The existing read-only summary (`MetaCard` / editor-card) stays beneath the banner — it retains genuine value (literal file value; install path; raw transcript peek).

Remove the misdirecting sentence from the plugin `MetaCard` note (the `/plugin` / hand-edit-`settings.json` text). The banner now provides the correct handoff; the note keeps only the neutral “Managed by Claude Code's plugin system” framing.

### 2. Navigation wiring (`App.tsx`)

`App` already owns `const [mode, setMode] = createSignal('organizer')`. Add:

- `const [focus, setFocus] = createSignal<FocusTarget | null>(null)` where `FocusTarget = { mode: string; category: string; key: string }`.
- `function navigate(targetMode: string, target?: FocusTarget) { setFocus(target ?? null); setMode(targetMode); }`
- Pass `onNavigate={navigate}` into `<Organizer>`.
- Pass `focus={focus()}` into `<Settings>`, `<Plugins>`, `<Sessions>`, plus a way to clear it once consumed (`onFocusConsumed={() => setFocus(null)}`), so the deep-link fires once and doesn't re-trigger on later interaction.

### 3. Deep-link focus behavior (per destination mode)

Each dedicated mode accepts an optional `focus` prop and, when it targets that mode, selects + scrolls to the item, then calls `onFocusConsumed()`:

- **Settings** (`focus.key` = setting key = `item.name`): set the search/filter to the key so the row is guaranteed visible regardless of the active category filter, select/highlight that row, scroll it into view.
- **Plugins** (`focus.key` = plugin short-name = `item.name`): switch to the **Installed** tab, filter/highlight the matching row, scroll into view.
- **Sessions** (`focus.key` = session identity = `item.path`, the rollout/`.jsonl` path): set the title/search filter so the target surfaces on the first page (the list is ~2,390 items and paginated ~50/page), then select it and scroll into view. Match on the stable path/id, not the display title.

Focus is consumed once (guard against re-firing when the user later changes tabs/search inside the mode).

### 4. Capability guard (per harness)

The banner/button renders only when the destination mode is usable on the active harness, read from `props.scan.capabilities`:

- `setting` → `settingsEditable` (Claude only; the `setting` category does not exist on Codex anyway).
- `plugin` → `pluginsManageable` (**Claude only — Codex advertises a `plugin` category but `plugins_manageable: false`, so on Codex the banner is suppressed and the current read-only card is unchanged**).
- `session` → `sessions` (true on **both** Claude and Codex — handoff works on both).

When the guard is false, the detail pane is byte-for-byte its current behavior (no button).

## Affected files

- `src/App.tsx` — focus signal + `navigate` + thread props.
- `src/modes/Organizer.tsx` — `onNavigate` prop; signpost banner in the detail pane; drop the plugin-note misdirection.
- `src/modes/Settings.tsx` — `focus` prop + select-by-key.
- `src/modes/Plugins.tsx` — `focus` prop + Installed-tab select-by-name.
- `src/modes/Sessions.tsx` — `focus` prop + filter+select-by-path.
- Tests alongside each (`*.test.tsx`).

## Testing (TDD)

- **Organizer** (`Organizer.test.tsx`): for a `setting`/`plugin`/`session` item on Claude, the signpost banner + button render, and clicking calls `onNavigate` with the right `(mode, {category, key})`. On **Codex**, a `plugin` item shows **no** button (guard). The plugin `MetaCard` no longer contains the `/plugin` misdirection string.
- **Settings / Plugins / Sessions** (`*.test.tsx`): given a matching `focus` prop, the mode selects/surfaces the target item (search set / correct tab / row selected) and calls `onFocusConsumed`; given a non-matching or null focus, behavior is unchanged.
- Full `npm test` + `npx tsc --noEmit` green before done. (No Rust changes → `cargo` suites unaffected, but run `cargo check` to be safe.)

## Rollout

Single branch, one commit per task per Ward convention. Verify in `npm run dev:mock` (Chrome) that clicking “Manage in →” from each of the three Organizer categories lands on the right item in the dedicated mode. No native-only surface involved, so mock verification is sufficient; a final hands-on click-through in the real Tauri window is a nice-to-have, not a blocker.
