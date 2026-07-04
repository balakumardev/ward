# Plan 01 — End-to-End Smoke Checklist

## Build verification (already done by SDD controller)

- [x] `cd src-tauri && cargo test` — 15/15 passing
- [x] `cd src-tauri && cargo check` — compiles clean (3 dead-code warnings, all expected pre-Plan-02)
- [x] `npm test` — 4/4 passing (Sidebar, api × 2, Organizer)
- [x] `npx tsc --noEmit` — clean

## GUI smoke (HANDS-ON — please run)

Run: `cd /Users/balakumar/personal/ward && npm run tauri dev`

A native window titled **Ward** should open. After a brief "Scanning ~/.claude…" it should show the Organizer.

### Manual checklist

- [ ] Sidebar shows all 5 modes; **Organizer** is active/highlighted
- [ ] **Skills** category shows a count > 0; clicking it lists skill names under "Global (~/.claude)"
- [ ] **Memories** category lists `CLAUDE.md` (with 🔒) plus any `~/.claude/memory/*.md`
- [ ] Clicking an item loads its file content in the detail pane (monospace)
- [ ] Clicking Security / Context Budget / Sessions / Backups shows the "Coming in a later plan" placeholder (proves mode switching)

### Safety boundary sanity

In the running app's devtools console (right-click → Inspect, or `Cmd+Option+I`), run:

```js
await window.__TAURI__.core.invoke('read_file_content', { path: '/etc/passwd' })
```

- [ ] Rejects with a `pathEscaped` error object (NOT the file contents)

## When done

If everything is green, the tag `plan-01-foundation` is already in place. If something fails, report back and we'll fix.
