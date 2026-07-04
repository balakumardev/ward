import { invoke } from '@tauri-apps/api/core';

export interface Capabilities {
  contextBudget: boolean; mcpControls: boolean; mcpPolicy: boolean;
  mcpSecurity: boolean; sessions: boolean; effective: boolean; backup: boolean;
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
  kind: 'file' | 'mcp-entry' | 'mcp-disabled' | 'mcp-policy';
  originalPath: string;
  currentPath?: string | null;
  backupBytes?: number[] | null;
  mcpEntry?: unknown;
  mcpKey?: string | null;
  mcpParentKey?: string | null;
  mcpScope?: string | null;
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

/** Plan 06 — Context Budget breakdown for a single scope. */
export interface BudgetBreakdown {
  systemLoaded: number;
  systemDeferred: number;
  mcpSchemas: number;
  claudemd: number;
  claudeMdFiles: BudgetFile[];
  alwaysLoadedItems: BudgetItem[];
  autocompactBuffer: number;
  maxOutput: number;
  warningThreshold: number;
  /** True when a real BPE tokenizer produced the numbers; false when the
   *  bytes/4 heuristic was used. The UI surfaces this as "measured" vs
   *  "estimated". */
  measured: boolean;
  /** Total tokens used by always-loaded + system overhead (what the meter
   *  fills toward). */
  used: number;
  /** Model's full context window (200K for Claude Sonnet/Opus). */
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

/** A single classified JSONL line. The `kind` discriminator mirrors
 *  the Rust `SessionRecord` enum (camelCase). */
export type SessionRecord =
  | { kind: 'user'; content: string; ts?: string }
  | { kind: 'assistant'; content: string; model?: string; ts?: string; usage?: Usage }
  | { kind: 'system'; subtype: string; summary?: string }
  | { kind: 'aiTitle'; title: string }
  | { kind: 'queueOperation'; enqueue: boolean }
  | { kind: 'other'; recordType: string };

/** Parsed conversation returned by `session_preview`. */
export interface Conversation {
  sessionId: string;
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

export const api = {
  scan: (harness: string) => invoke<ScanResult>('scan', { harness }),
  readFileContent: (path: string) => invoke<string>('read_file_content', { path }),
  listDestinations: (harness: string, item: HarnessItem) =>
    invoke<Destination[]>('list_destinations', { harness, item }),
  moveItem: (harness: string, item: HarnessItem, destScopeId: string) =>
    invoke<RestoreInfo>('move_item', { harness, item, destScopeId }),
  deleteItem: (harness: string, item: HarnessItem) =>
    invoke<RestoreInfo>('delete_item', { harness, item }),
  restore: (harness: string, info: RestoreInfo) =>
    invoke<void>('restore', { harness, info }),
  saveFile: (path: string, content: string) =>
    invoke<void>('save_file', { path, content }),
  bulk: (harness: string, items: HarnessItem[], op: string, destScopeId?: string) =>
    invoke<RestoreInfo[]>('bulk', { harness, items, op, destScopeId }),
  bulkRestore: (harness: string, infos: RestoreInfo[]) =>
    invoke<void>('bulk_restore', { harness, infos }),

  // Plan 04 — MCP controls.
  mcpGetDisabled: (projectPath: string) =>
    invoke<string[]>('mcp_get_disabled', { projectPath }),
  mcpSetDisabled: (projectPath: string, list: string[]) =>
    invoke<RestoreInfo>('mcp_set_disabled', { projectPath, list }),
  mcpGetPolicy: () => invoke<McpPolicy>('mcp_get_policy'),
  mcpSetPolicy: (policy: McpPolicy) =>
    invoke<RestoreInfo>('mcp_set_policy', { policy }),
  mcpCheckPolicy: (serverName: string, serverConfig: unknown, policy: McpPolicy) =>
    invoke<PolicyVerdict>('mcp_check_policy', { serverName, serverConfig, policy }),

  // Plan 05 — Security scanner.
  securityScan: (harness: string, items: HarnessItem[], runJudge?: boolean) =>
    invoke<ScanResultSec>('security_scan', { harness, items, runJudge }),
  securityBaselineCheck: (scan: ScanResultSec) =>
    invoke<BaselineDiff[]>('security_baseline_check', { scan }),
  securityBaselineAccept: (server: string, findings: string[]) =>
    invoke<void>('security_baseline_accept', { server, findings }),

  // Plan 06 — Context Budget.
  contextBudget: (harness: string, scopeId: string) =>
    invoke<BudgetBreakdown>('context_budget', { harness, scopeId }),

  // Plan 07 — Sessions mode.
  sessionPreview: (path: string) => invoke<Conversation>('session_preview', { path }),
  sessionCost: (path: string) => invoke<CostBreakdown>('session_cost', { path }),
  sessionDistill: (path: string) => invoke<DistillResult>('session_distill', { path }),
  sessionTrim: (path: string) => invoke<RestoreInfo>('session_trim', { path }),
};
