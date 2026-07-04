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
};
