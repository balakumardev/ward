# Ward Plan 07 — Sessions: viewer / cost / distill / trim (High-Level)

> High-level plan. Use **superpowers:subagent-driven-development**, TDD, commit per task. CCO reference (read-only): `/Users/balakumar/personal/claude-code-organizer`.

**Goal:** Browse session JSONL as a conversation, show per-model **cost**, **distill** a session (~90% size cut), and **trim** base64 images.

**Builds on:** `session` category (Plan 02), `commands.rs`.

**Files:**
- Create `src-tauri/src/sessions/{parse,cost,distill,trim}.rs`:
  - `parse.rs`: JSONL → structured conversation turns.
  - `cost.rs`: per-model token/cost breakdown.
  - `distill.rs`: back up original → write cleaned session + `index.md` (~90% smaller).
  - `trim.rs`: replace base64 image blocks with `[image redacted]`.
- Modify `src-tauri/src/commands.rs`: `session_preview`, `session_cost`, `session_distill`, `session_trim`.
- Create frontend `src/modes/Sessions.tsx`: session list (from `session` category) → conversation viewer + cost panel + Distill/Trim actions.

**Task checklist:**
- [ ] JSONL parse → conversation model.
- [ ] Per-model cost aggregation.
- [ ] Distill: **back up first**, then clean + emit `index.md`.
- [ ] Trim images → `[image redacted]`.
- [ ] Sessions mode UI.

**CCO parity refs:** `src/session-distiller.mjs`, `src/trim-images.mjs`, session parse in `claude.mjs`; endpoints `/api/session-preview`, `/api/session-cost`, `/api/session-distill`.

**Tests:** parse a fixture JSONL; cost sums per model; distill backs up + shrinks; trim redacts images and preserves structure.

**Gotchas:** distill/trim mutate files → **back up before writing**; large JSONL perf (stream, don't load whole into memory where possible); keep the conversation viewer read-only.
