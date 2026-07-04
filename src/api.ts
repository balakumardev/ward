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
}
export interface ScanResult {
  harnessId: string; categories: Category[]; scopes: Scope[];
  items: HarnessItem[]; capabilities: Capabilities;
}

export const api = {
  scan: (harness: string) => invoke<ScanResult>('scan', { harness }),
  readFileContent: (path: string) => invoke<string>('read_file_content', { path }),
};
