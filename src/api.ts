import { invoke } from '@tauri-apps/api/core';

/** True when this page is loaded inside a Tauri webview (the native
 *  app). False when running under Vite's bare browser preview — in
 *  that case `invoke` would silently hang. Callers should branch on
 *  this and render an explanatory placeholder instead of waiting
 *  forever on "Scanning…". */
export function isTauri(): boolean {
  return typeof (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== 'undefined';
}

/** Friendly error thrown when an `invoke` call is made outside the
 *  Tauri webview. Surfaced in the UI instead of a generic hang. */
export class TauriUnavailableError extends Error {
  constructor(cmd: string) {
    super(
      `Ward command '${cmd}' requires the Tauri runtime. ` +
      `This browser preview cannot reach the Rust backend. ` +
      `Launch the native app with \`npm run tauri dev\` for full functionality.`,
    );
    this.name = 'TauriUnavailableError';
  }
}

/** Thin wrapper around `@tauri-apps/api/core::invoke` that throws a
 *  descriptive error when the page isn't running inside a Tauri
 *  webview. Use this for any code path the UI must surface
 *  gracefully — Solid's `createResource` will catch the throw and
 *  expose it via `resource.error`. */
function invokeOrThrow<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) return Promise.reject(new TauriUnavailableError(cmd));
  return invoke<T>(cmd, args);
}

export interface Capabilities {
  contextBudget: boolean; mcpControls: boolean; mcpPolicy: boolean;
  mcpSecurity: boolean; sessions: boolean; effective: boolean; backup: boolean;
  /** Plan 18 — true when this harness has a working MCP upsert backend, so
   *  the Organizer renders the editable structured MCP form + "+ Add MCP".
   *  False (e.g. Codex) keeps the MCP pane read-only. */
  mcpEditable: boolean;
  /** Plan 19 — true when this harness can create a new Skill (scaffold
   *  `<skills_dir>/<name>/SKILL.md`), gating the Organizer's "+ Add Skill".
   *  Claude true; Codex false until its write path lands. */
  skillCreatable: boolean;
}
export interface Category { id: string; label: string; count: number; }
export interface Scope { id: string; kind: string; label: string; root: string; }
export interface HarnessItem {
  category: string; scopeId: string; name: string; description?: string; path: string;
  movable: boolean; deletable: boolean; locked: boolean;
  /** "shadowed" | "conflict" | "ancestor" when an item is in the effective
   *  resolution set but not the active winner. Omitted for active items. */
  effective?: 'shadowed' | 'conflict' | 'ancestor';
  /** Plan 04 — server config (command/args/url) for MCP items. Undefined
   *  for non-MCP items. Used by `mcpCheckPolicy` to render badges. */
  mcpConfig?: unknown;
}
export interface ScanResult {
  harnessId: string; categories: Category[]; scopes: Scope[];
  items: HarnessItem[]; capabilities: Capabilities;
}
export interface Destination { scopeId: string; label: string; kind: string; }
export interface RestoreInfo {
  kind: 'file' | 'mcp-entry' | 'mcp-disabled' | 'mcp-policy' | 'mcp-upsert' | 'skill-create';
  originalPath: string;
  currentPath?: string | null;
  backupBytes?: number[] | null;
  mcpEntry?: unknown;
  mcpKey?: string | null;
  mcpParentKey?: string | null;
  mcpScope?: string | null;
}

/** Plan 18 — a single MCP server config, as stored in the shared
 *  config file (`~/.claude.json` mcpServers map / Codex TOML). Fields
 *  cover both stdio (`command`/`args`/`env`) and HTTP/SSE
 *  (`url`/`headers`/`type`) transports. The index signature preserves
 *  any keys Ward doesn't model so an upsert round-trips losslessly. */
export interface McpConfig {
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  headers?: Record<string, string>;
  type?: string;
  enabled?: boolean;
  [key: string]: unknown; // preserve unknown keys round-trip
}

/** Plan 04 — MCP policy allowlist/denylist entries. */
export interface PolicyEntry {
  serverName?: string;
  serverCommand?: string[];
  serverUrl?: string;
}

/** Plan 04 — user-scope MCP policy. */
export interface McpPolicy {
  allowlist: PolicyEntry[];
  denylist: PolicyEntry[];
}

/** Plan 04 — outcome of `mcp_check_policy`. */
export type PolicyVerdict = 'allowed' | 'denied' | 'noPolicy';

/** Plan 05 — security scan finding. */
export type Severity = 'critical' | 'high' | 'medium' | 'low';
export interface Finding {
  id: string; ruleId: string; category: string; severity: Severity;
  name: string; description: string; matchedText: string; context: string;
  sourceType: string; sourceName: string;
}
export interface ServerSummary {
  serverName: string; scopeId: string; status: string;
  error?: string | null; toolCount: number;
  tools: Array<{ name: string; description: string; inputSchema: unknown; hash: string }>;
  findings: Finding[];
}
export interface DupFinding {
  kind: 'duplicate'; server: string; serverScope: string;
  duplicateOf: string; winnerScope: string;
  signatureType: string; signature: string;
}
export interface BaselineDiff {
  server: string; tool: string; change: 'added' | 'removed' | 'changed' | 'unchanged';
}
export interface SeverityCounts { critical: number; high: number; medium: number; low: number; }
export interface ScanResultSec {
  timestamp: string;
  servers: ServerSummary[];
  findings: Finding[];
  duplicates: DupFinding[];
  baselineDiffs: BaselineDiff[];
  severityCounts: SeverityCounts;
  totalTools: number; totalServers: number;
  serversConnected: number; serversFailed: number;
  judgeUsed: boolean;
}

/** Plan 06 — Context Budget breakdown for a single scope.
 *
 *  Three tiers of what Claude Code actually loads:
 *  - Always-on FULL: system + CLAUDE.md + unscoped rules + MEMORY.md +
 *    active output style (these + the metadata scalars below sum to `used`).
 *  - Always-on METADATA: the capped skill/command listing, subagent
 *    listing, and MCP tool-names line.
 *  - Deferred / on-invoke: skill/command/agent bodies, MCP tool schemas,
 *    and `paths:`-scoped rules (surfaced via `deferredTotal`, NOT `used`). */
export interface BudgetBreakdown {
  systemLoaded: number;
  /** Active non-default output style, folded into system overhead. */
  outputStyle: number;
  systemDeferred: number;
  /** MCP tool schemas — DEFERRED (part of `deferredTotal`, not `used`). */
  mcpSchemas: number;
  /** Always-on MCP tool-names line (0 when no enabled server). */
  mcpToolNames: number;
  claudemd: number;
  claudeMdFiles: BudgetFile[];
  /** Capped skill+command listing (`"name": description`). */
  skillListing: number;
  /** Uncapped listing total (so the UI can show "capped from N"). */
  skillListingRaw: number;
  /** `<available_skills>` boilerplate (0 when no skill/command). */
  skillBoilerplate: number;
  /** Subagent listing (`name: description`). */
  agentListing: number;
  /** Full-content always-on items: unscoped rules + MEMORY.md. */
  alwaysLoadedItems: BudgetItem[];
  /** Metadata rows behind skillListing/agentListing (skills, commands,
   *  agents) — display only; the capped scalars feed `used`. */
  metadataItems: BudgetItem[];
  /** On-invoke bodies + scoped rules — NOT part of `used`. */
  deferredItems: BudgetItem[];
  /** systemDeferred + mcpSchemas + Σ deferredItems. */
  deferredTotal: number;
  autocompactBuffer: number;
  maxOutput: number;
  warningThreshold: number;
  /** True when a real BPE tokenizer produced the numbers; false when the
   *  bytes/4 heuristic was used. The UI surfaces this as "measured" vs
   *  "estimated". */
  measured: boolean;
  /** Total ALWAYS-ON tokens (system overhead + metadata listings +
   *  full always-on items). What the meter fills toward. */
  used: number;
  /** Model's full context window (200K default; scales for 1M models). */
  contextLimit: number;
}
export interface BudgetFile {
  path: string;
  name: string;
  tokens: number;
  measured: boolean;
}
export interface BudgetItem {
  category: string;
  name: string;
  tokens: number;
  measured: boolean;
}

// ── Plan 07 — Sessions mode ─────────────────────────────────────────────

/** Per-message usage block. Tokens from `message.usage`. */
export interface Usage {
  inputTokens: number;
  outputTokens: number;
  cacheRead?: number;
  cacheWrite?: number;
}

/** A single content block inside a user/assistant message. Mirrors the
 *  Rust `ContentBlock` enum (internally tagged on `type`, camelCase).
 *  Real turns are arrays of these — plain text is only one variant. */
export type ContentBlock =
  | { type: 'text'; text: string }
  | { type: 'thinking'; text: string }
  | { type: 'toolUse'; name: string; inputSummary: string }
  | { type: 'toolResult'; content: string }
  | { type: 'image' };

/** A single classified JSONL line. The `kind` discriminator mirrors
 *  the Rust `SessionRecord` enum (camelCase). User/Assistant carry the
 *  structured `blocks` plus a derived flattened `content`. */
export type SessionRecord =
  | { kind: 'user'; content: string; blocks: ContentBlock[]; ts?: string }
  | { kind: 'assistant'; content: string; blocks: ContentBlock[]; model?: string; ts?: string; usage?: Usage }
  | { kind: 'system'; subtype: string; summary?: string }
  | { kind: 'aiTitle'; title: string }
  | { kind: 'summary'; text: string; leafUuid?: string }
  | { kind: 'queueOperation'; enqueue: boolean }
  | { kind: 'other'; recordType: string };

/** Parsed conversation returned by `session_preview`. */
export interface Conversation {
  sessionId: string;
  title?: string;
  records: SessionRecord[];
}

/** Per-model cost row. */
export interface ModelCost {
  model: string;
  inputTokens: number;
  outputTokens: number;
  cacheRead: number;
  cacheWrite: number;
  costUsd: number;
}

/** Aggregate cost result returned by `session_cost`. */
export interface CostBreakdown {
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheRead: number;
  totalCacheWrite: number;
  perModel: ModelCost[];
  estimatedCostUsd: number;
  /** Number of assistant records whose model was unknown. The UI
   *  surfaces this as a soft "estimated" badge. */
  estimatedRecords: number;
}

/** Result of `session_distill`. */
export interface DistillResult {
  originalPath: string;
  cleanedPath: string;
  backupPath: string;
  originalBytes: number;
  cleanedBytes: number;
  reductionPct: number;
  indexMd: string;
}

// ── Plan 08 — Backup Center ────────────────────────────────────────────

/** Aggregate snapshot returned by `backup_status`. */
export interface BackupStatus {
  hasRepo: boolean;
  lastCommit: string | null;
  lastCommitAt: string | null;
  schedulerInstalled: boolean;
  /** True when launchd still has the backup label loaded but its plist
   *  file is gone (a recoverable orphan). The UI keeps Remove enabled in
   *  this state so the user can clear the dead job. */
  schedulerOrphaned: boolean;
  schedulerInterval: number | null;
  remoteUrl: string | null;
}

/** Output of `backup_run` — what got mirrored into ~/.ward-backups/. */
export interface ExportReport {
  filesCopied: number;
  bytesCopied: number;
  skipped: string[];
}

/** Result of `backup_sync` — was a commit produced? */
export interface CommitInfo {
  committed: boolean;
  sha: string | null;
  message: string;
  committedAt: string | null;
}

/** Result of `backup_push` — did we actually push? */
export interface PushResult {
  pushed: boolean;
  reason: string;
  remoteUrl: string | null;
}

/** One entry from `git log`. */
export interface GitLogEntry {
  sha: string;
  subject: string;
  author: string;
  committedAt: string;
}

/** `git status --porcelain` rolled into counts. */
export interface GitStatus {
  modified: number;
  untracked: number;
  clean: boolean;
}

// ── Plan 14 — Usage engine ──────────────────────────────────────────────
export interface TokenTotals {
  input: number;
  output: number;
  cacheCreation: number;
  cacheRead: number;
  total: number;
}

export type UsageSource = 'local' | 'rateLimits' | 'live';

export interface UsageWindow {
  tokens: TokenTotals;
  costUsd: number;
  percent?: number;      // 0..1 when known (Codex, or Claude w/ configured limit)
  resetsAt?: string;
  resetsInSecs?: number;
  isActive: boolean;
  startedAt?: string;
  planType?: string;
}

export interface UsageSnapshot {
  harness: string;
  block: UsageWindow;    // current 5-hour window
  week: UsageWindow;     // weekly window
  source: UsageSource;
  available: boolean;
  generatedAt: string;
}

// ── Plan 21 — Marketplace (MCP servers) ─────────────────────────────────

/** One env var (or remote header) a server declares. Ward renders the NAME
 *  only — a secret value is never collected or written. */
export interface EnvVar {
  name: string;
  isRequired: boolean;
  isSecret: boolean;
}

/** One installable package for an MCP server. `transport` is the flattened
 *  `stdio` | `http` | `sse`; `env` lists the declared vars. */
export interface Package {
  registryType: string; // "npm" | "pypi" | "oci"
  identifier: string;
  version: string;
  transport: string;
  env: EnvVar[];
  runtimeHint?: string;
}

/** A hosted remote transport for an MCP server. */
export interface Remote {
  transport: string;
  url: string;
  headers: EnvVar[];
}

/** Unified marketplace card model (mirrors the Rust `MarketEntry`).
 *  `kind: "skill"` + `repoUrl`/`skillPath` are the Plan 22 seam. */
export interface MarketEntry {
  kind: string; // "mcp" | "skill"
  name: string;
  displayName: string;
  description: string;
  source: string; // "registry" | "github" | "marketplace"
  version?: string;
  verified: boolean;
  packages: Package[];
  remotes: Remote[];
  repoUrl?: string;
  skillPath?: string;
}

/** One install destination — a harness × scope pair. Kept as data so a
 *  future `harness: "claude-desktop"` slots in without a rewrite. */
export interface InstallTarget {
  harness: string;
  scopeId: string;
}

/** The exact server object that will land on disk, plus the flattened
 *  command/url preview and the declared env metadata. */
export interface BuiltConfig {
  name: string;
  config: McpConfig;
  commandPreview: string[];
  env: EnvVar[];
}

/** Per-target install outcome. The batch never aborts on one failure, so
 *  each target reports independently (and carries its own undoable
 *  `restore` on success). */
export interface InstallResult {
  target: InstallTarget;
  ok: boolean;
  error?: string;
  restore?: RestoreInfo;
}

/** One page of marketplace results + the cursor for the next page. */
export interface MarketPage {
  entries: MarketEntry[];
  nextCursor?: string;
}

/** The fetched-and-parsed `SKILL.md` shown BEFORE a skill install so approval
 *  is bound to the actual content, not just the name (mirrors Rust
 *  `SkillPreview`). Frontmatter wins; the catalog entry is the fallback. */
export interface SkillPreview {
  name: string;
  description: string;
  body: string;
}

export const api = {
  scan: (harness: string) => invokeOrThrow<ScanResult>('scan', { harness }),
  readFileContent: (path: string) => invokeOrThrow<string>('read_file_content', { path }),
  listDestinations: (harness: string, item: HarnessItem) =>
    invokeOrThrow<Destination[]>('list_destinations', { harness, item }),
  moveItem: (harness: string, item: HarnessItem, destScopeId: string) =>
    invokeOrThrow<RestoreInfo>('move_item', { harness, item, destScopeId }),
  deleteItem: (harness: string, item: HarnessItem) =>
    invokeOrThrow<RestoreInfo>('delete_item', { harness, item }),
  restore: (harness: string, info: RestoreInfo) =>
    invokeOrThrow<void>('restore', { harness, info }),
  saveFile: (path: string, content: string) =>
    invokeOrThrow<void>('save_file', { path, content }),
  bulk: (harness: string, items: HarnessItem[], op: string, destScopeId?: string) =>
    invokeOrThrow<RestoreInfo[]>('bulk', { harness, items, op, destScopeId }),
  bulkRestore: (harness: string, infos: RestoreInfo[]) =>
    invokeOrThrow<void>('bulk_restore', { harness, infos }),

  // Plan 04 — MCP controls.
  mcpGetDisabled: (projectPath: string) =>
    invokeOrThrow<string[]>('mcp_get_disabled', { projectPath }),
  mcpSetDisabled: (projectPath: string, list: string[]) =>
    invokeOrThrow<RestoreInfo>('mcp_set_disabled', { projectPath, list }),
  mcpGetPolicy: () => invokeOrThrow<McpPolicy>('mcp_get_policy'),
  mcpSetPolicy: (policy: McpPolicy) =>
    invokeOrThrow<RestoreInfo>('mcp_set_policy', { policy }),
  mcpCheckPolicy: (serverName: string, serverConfig: unknown, policy: McpPolicy) =>
    invokeOrThrow<PolicyVerdict>('mcp_check_policy', { serverName, serverConfig, policy }),

  // Plan 18 — MCP marketplace: install/update a server entry in a scope's
  // shared config file (upsert by name).
  mcpUpsertEntry: (harness: string, scopeId: string, name: string, config: McpConfig, targetPath?: string) =>
    invokeOrThrow<RestoreInfo>('mcp_upsert_entry', { harness, scopeId, name, config, targetPath }),

  // Plan 19 — creatable skills: scaffold a new `<skills_dir>/<name>/SKILL.md`
  // (create-only). Returns a `skill-create` RestoreInfo for Undo.
  skillUpsert: (harness: string, scopeId: string, name: string, content: string) =>
    invokeOrThrow<RestoreInfo>('skill_upsert', { harness, scopeId, name, content }),

  // Plan 21 — Marketplace (MCP servers). Search is a user-triggered network
  // call; build_config is a pure pre-install preview; install fans out to the
  // shared upsert engine (one Install = N targets).
  marketplaceSearch: (kind: string, query: string, cursor?: string) =>
    invokeOrThrow<MarketPage>('marketplace_search', { kind, query, cursor }),
  marketplaceBuildConfig: (entry: MarketEntry, packageIndex: number, envValues: Record<string, string>) =>
    invokeOrThrow<BuiltConfig>('marketplace_build_config', { entry, packageIndex, envValues }),
  marketplaceInstall: (entry: MarketEntry, packageIndex: number, targets: InstallTarget[], envValues: Record<string, string>) =>
    invokeOrThrow<InstallResult[]>('marketplace_install', { entry, packageIndex, targets, envValues }),

  // Plan 22 — fetch + parse a skill's SKILL.md for the pre-install preview
  // (bind approval to content). User-triggered on card select.
  marketplacePreviewSkill: (entry: MarketEntry) =>
    invokeOrThrow<SkillPreview>('marketplace_preview_skill', { entry }),

  // Plan 05 — Security scanner.
  securityScan: (harness: string, items: HarnessItem[], runJudge?: boolean) =>
    invokeOrThrow<ScanResultSec>('security_scan', { harness, items, runJudge }),
  securityBaselineCheck: (scan: ScanResultSec) =>
    invokeOrThrow<BaselineDiff[]>('security_baseline_check', { scan }),
  securityBaselineAccept: (server: string, findings: string[]) =>
    invokeOrThrow<void>('security_baseline_accept', { server, findings }),

  // Plan 06 — Context Budget.
  contextBudget: (harness: string, scopeId: string) =>
    invokeOrThrow<BudgetBreakdown>('context_budget', { harness, scopeId }),

  // Plan 07 — Sessions mode.
  sessionPreview: (path: string) => invokeOrThrow<Conversation>('session_preview', { path }),
  sessionCost: (path: string) => invokeOrThrow<CostBreakdown>('session_cost', { path }),
  sessionDistill: (path: string) => invokeOrThrow<DistillResult>('session_distill', { path }),
  sessionTrim: (path: string) => invokeOrThrow<RestoreInfo>('session_trim', { path }),

  // Plan 08 — Backup Center.
  backupStatus: () => invokeOrThrow<BackupStatus>('backup_status'),
  backupRun: (scan: ScanResult, remoteUrl?: string | null) =>
    invokeOrThrow<ExportReport>('backup_run', { scan, remoteUrl: remoteUrl ?? null }),
  backupSync: () => invokeOrThrow<CommitInfo>('backup_sync'),
  backupPush: () => invokeOrThrow<PushResult>('backup_push'),
  backupSchedulerInstall: (intervalSeconds: number) =>
    invokeOrThrow<void>('backup_scheduler_install', { intervalSeconds }),
  backupSchedulerRemove: () => invokeOrThrow<void>('backup_scheduler_remove'),
  backupSetRemote: (url: string) => invokeOrThrow<void>('backup_set_remote', { url }),
  backupLog: (n: number) => invokeOrThrow<GitLogEntry[]>('backup_log', { n }),

  // Plan 14/15 — usage engine + native shell.
  usageSnapshot: (harness: string) => invokeOrThrow<UsageSnapshot>('usage_snapshot', { harness }),
  // Plan 17 — last-known cached snapshot for instant cache-first popover paint.
  usageCached: (harness: string) => invokeOrThrow<UsageSnapshot | null>('usage_cached', { harness }),
  // Plan 16 — live Claude usage (gated network call; Claude only).
  usageSnapshotLive: (harness: string) => invokeOrThrow<UsageSnapshot>('usage_snapshot_live', { harness }),
  liveUsageEnabled: () => invokeOrThrow<boolean>('live_usage_enabled'),
  setLiveUsageEnabled: (enabled: boolean) => invokeOrThrow<void>('set_live_usage_enabled', { enabled }),
  autostartStatus: () => invokeOrThrow<boolean>('autostart_status'),
  autostartSet: (enabled: boolean) => invokeOrThrow<void>('autostart_set', { enabled }),
  nativeUpdateStatus: (critical: number, lastScanAt?: string) =>
    invokeOrThrow<void>('native_update_status', { critical, lastScanAt }),
};
