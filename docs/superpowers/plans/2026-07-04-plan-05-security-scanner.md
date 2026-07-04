# Ward Plan 05 — Security Scanner (High-Level) ★ the moat

> High-level plan. Use **superpowers:subagent-driven-development**, TDD, commit per task. **This is the differentiator — port CCO's logic faithfully and use its tests as the parity spec.** CCO reference (read-only): `/Users/balakumar/personal/claude-code-organizer`.

**Goal:** Introspect MCP servers, run the **4-layer** scan, maintain a **hash baseline**, present the **in-place master-detail** Security mode, and offer an optional `claude -p` **LLM judge**.

**Builds on:** `mcp` category (Plan 02), mutation ops (Plan 03), MCP controls (Plan 04), `commands.rs`, design tokens.

**Files:**
- Create `src-tauri/src/mcp/introspect.rs`: JSON-RPC over **stdio** (+ streamable HTTP) — spawn server, `initialize`, `tools/list`; hash each tool definition. (Port CCO `src/mcp-introspector.mjs`, ~200 lines. Evaluate `rmcp` crate vs. hand-roll.)
- Create `src-tauri/src/security/{mod,deobfuscate,rules,baseline,judge}.rs`:
  - `deobfuscate.rs`: 8 techniques (base64, homoglyph, zero-width, etc.).
  - `rules.rs`: ~40+ regex rules with **verbatim IDs + severities** grouped `prompt_injection` (PI-*), `tool_poisoning` (TP-*), `tool_shadowing` (TS-*), `sensitive_access` (SF-*), `data_exfiltration` (DE-*), `credential_harvest` (CH-*), `code_execution` (CE-*), `command_injection` (CI-*), `suspicious_hook` (HK-*), `exfil_params` (EP-*).
  - `baseline.rs`: store/compare tool-def hashes at **`~/.ward/security/baselines.json`** (detects a server silently changing tools).
  - `judge.rs`: optional, user-triggered `claude -p` (feature-detect via `checkClaudeAvailable` equivalent; never blocks layers 1–3).
- Modify `src-tauri/src/commands.rs`: `security_status`, `security_scan`, `security_rescan` (judge), `security_baseline_check`, `security_cache` get/set.
- Create frontend `src/modes/Security.tsx`: master-detail — severity-colored findings list + detail (rule, **deobfuscated + highlighted** poisoned snippet, server config, actions **Delete / Disable / Move / Re-judge / Accept-baseline** wired to Plan 03/04 ops).

**Task checklist:**
- [ ] MCP introspection client (stdio); tool-def hashing.
- [ ] Layer 1 deobfuscation (8 techniques).
- [ ] Layer 2 pattern rules (all IDs/severities ported verbatim).
- [ ] Layer 3 hash baseline store/compare under `~/.ward`.
- [ ] Layer 4 optional `claude -p` judge (feature-detected).
- [ ] `security_scan` orchestration + progress events.
- [ ] Security mode UI (master-detail) + action wiring.

**CCO parity refs:** `src/security-scanner.mjs` (entire 4-layer pipeline + rule set — credited to AgentSeal + Cisco YARA), `src/mcp-introspector.mjs`; endpoints `/api/security-*`. **Port golden (critical):** `tests/unit/test-security-features.mjs`.

**Tests:** each rule category hits + misses; deobfuscation reveals hidden instruction; baseline detects a tool-def change; judge is skipped gracefully when `claude` CLI absent.

**Gotchas:** introspection **spawns untrusted MCP servers** to read tool defs — same trust model as CCO; run user-triggered, least-privilege, document the caveat. Judge is optional and must never block layers 1–3. Store baselines under `~/.ward`, never `~/.claude`. Menu-bar/background scanning is **Plan 10**, not here.
