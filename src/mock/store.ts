// Dev-only stateful mock store backing the Ward UI (see ./install.ts).
//
// Holds an in-memory model of the harness scan so mutations are REAL: move /
// delete / undo / bulk / policy edits actually change the store, and the next
// `scan` reflects the new reality. This is what lets the mutation and undo
// flows be exercised (and fixed) without the Rust backend.

// Imported as a raw string (`?raw`) and parsed at runtime so the typechecker
// doesn't have to infer a literal type for the 579 KB fixture. JSON.parse also
// hands back a fresh, mutable object each construction.
import scanClaudeRaw from './fixtures/scan-claude.json?raw';
import type {
  ScanResult, HarnessItem, Destination, RestoreInfo, McpPolicy, PolicyVerdict,
  ScanResultSec, BaselineDiff, BudgetBreakdown, Conversation, CostBreakdown,
  DistillResult, BackupStatus, ExportReport, CommitInfo, PushResult, GitLogEntry,
} from '../api';
import {
  codexScan, securityScan, budgetFor, conversationFor, costFor, distillFor, initialBackupStatus,
  usageSnapshotFor,
  liveSnapshotFor,
} from './fixtures';

/** A RestoreInfo carrying an opaque handle to the mock's undo closure. The UI
 *  treats RestoreInfo as opaque and passes it straight back to `restore`, so
 *  the extra field survives the round-trip untouched. */
type MockRestore = RestoreInfo & { __undoId: string };

function clone<T>(v: T): T {
  return structuredClone(v);
}

function itemKey(i: HarnessItem): string {
  return `${i.category}::${i.name}::${i.scopeId}::${i.path}`;
}

export class MockStore {
  private claude: ScanResult;
  private codex: ScanResult;
  private policy: McpPolicy = { allowlist: [], denylist: [] };
  private disabled: Record<string, string[]> = {};
  private savedFiles: Record<string, string> = {};
  private acceptedBaselines = new Set<string>();
  private backup: BackupStatus;
  private undoLog = new Map<string, () => void>();
  private undoSeq = 0;
  private autostartEnabled = true;
  // Plan 16 — live usage opt-in. Defaults on in the mock so dev:mock shows the
  // live Claude gauges immediately (the real app starts opted-out).
  private liveEnabled = true;

  constructor() {
    this.claude = JSON.parse(scanClaudeRaw) as ScanResult;
    this.codex = clone(codexScan);
    this.backup = initialBackupStatus();
  }

  private scanFor(harness: string): ScanResult {
    return harness === 'codex' ? this.codex : this.claude;
  }

  /** Fresh shallow snapshot (new object + array identity) so Solid's
   *  createResource re-renders after every mutation. Category counts are
   *  recomputed from the live items so deletes visibly decrement. */
  scan(harness: string): ScanResult {
    const s = this.scanFor(harness);
    const counts = new Map<string, number>();
    for (const i of s.items) counts.set(i.category, (counts.get(i.category) ?? 0) + 1);
    return {
      ...s,
      categories: s.categories.map((c) => ({ ...c, count: counts.get(c.id) ?? 0 })),
      items: [...s.items],
    };
  }

  private newUndo(fn: () => void): string {
    const id = `undo-${++this.undoSeq}`;
    this.undoLog.set(id, fn);
    return id;
  }

  private locate(harness: string, item: HarnessItem): { arr: HarnessItem[]; idx: number } {
    const arr = this.scanFor(harness).items;
    const key = itemKey(item);
    return { arr, idx: arr.findIndex((i) => itemKey(i) === key) };
  }

  private noopRestore(path: string): MockRestore {
    return { kind: 'file', originalPath: path, __undoId: '' };
  }

  // ── Files ──
  readFile(path: string): string {
    if (path in this.savedFiles) return this.savedFiles[path];
    const name = path.split('/').pop() ?? path;
    return `# ${name}\n\n_Ward mock preview_\n\nSynthetic content served by the dev mock harness for\n\`${path}\`.\n\nReal file contents are only available in the native app (\`npm run tauri dev\`).\nEdit freely — Save / Revert are wired to the mock store so the editor UI is\nfully exercisable.\n\n${'lorem ipsum dolor sit amet consectetur adipiscing elit. '.repeat(16)}\n`;
  }

  saveFile(path: string, content: string): void {
    this.savedFiles[path] = content;
  }

  // ── Move / delete / undo ──
  listDestinations(harness: string, item: HarnessItem): Destination[] {
    const out: Destination[] = [];
    for (const s of this.scanFor(harness).scopes) {
      if (s.id === item.scopeId) continue;
      out.push({ scopeId: s.id, label: s.label, kind: s.kind });
    }
    // Global first, matching the Organizer's own ordering.
    out.sort((a, b) => (a.scopeId === 'global' ? -1 : b.scopeId === 'global' ? 1 : 0));
    return out;
  }

  moveItem(harness: string, item: HarnessItem, destScopeId: string): MockRestore {
    const { arr, idx } = this.locate(harness, item);
    if (idx < 0) return this.noopRestore(item.path);
    const prevScope = arr[idx].scopeId;
    const moved = { ...arr[idx], scopeId: destScopeId };
    arr[idx] = moved;
    const undoId = this.newUndo(() => {
      const j = arr.indexOf(moved);
      if (j >= 0) arr[j] = { ...arr[j], scopeId: prevScope };
    });
    return { kind: 'file', originalPath: item.path, currentPath: item.path, __undoId: undoId };
  }

  deleteItem(harness: string, item: HarnessItem): MockRestore {
    const { arr, idx } = this.locate(harness, item);
    if (idx < 0) return this.noopRestore(item.path);
    const removed = arr.splice(idx, 1)[0];
    const undoId = this.newUndo(() => { arr.splice(Math.min(idx, arr.length), 0, removed); });
    return { kind: 'file', originalPath: item.path, currentPath: null, __undoId: undoId };
  }

  bulk(harness: string, items: HarnessItem[], op: 'move' | 'delete', destScopeId?: string): MockRestore[] {
    return items.map((it) =>
      op === 'move' ? this.moveItem(harness, it, destScopeId ?? 'global') : this.deleteItem(harness, it),
    );
  }

  restore(info: RestoreInfo): void {
    const id = (info as MockRestore).__undoId;
    const fn = id ? this.undoLog.get(id) : undefined;
    if (fn) { fn(); this.undoLog.delete(id); }
  }

  bulkRestore(infos: RestoreInfo[]): void {
    // Reverse order so index-based re-inserts land at their original slots.
    for (const info of [...infos].reverse()) this.restore(info);
  }

  // ── MCP controls & policy ──
  getDisabled(projectPath: string): string[] {
    return this.disabled[projectPath] ?? [];
  }

  setDisabled(projectPath: string, list: string[]): MockRestore {
    const prev = this.disabled[projectPath] ?? [];
    this.disabled[projectPath] = [...list];
    const undoId = this.newUndo(() => { this.disabled[projectPath] = prev; });
    return { kind: 'mcp-disabled', originalPath: projectPath, __undoId: undoId };
  }

  upsertMcpEntry(harness: string, scopeId: string, name: string, config: unknown, targetPath?: string): MockRestore {
    const s = this.scanFor(harness);
    const idx = s.items.findIndex((i) => i.category === 'mcp' && i.name === name && i.scopeId === scopeId);
    if (idx >= 0) {
      const prev = s.items[idx].mcpConfig;
      s.items[idx] = { ...s.items[idx], mcpConfig: config };
      const undoId = this.newUndo(() => { s.items[idx] = { ...s.items[idx], mcpConfig: prev }; });
      return { kind: 'mcp-upsert', originalPath: s.items[idx].path, __undoId: undoId };
    }
    const newItem = {
      category: 'mcp', scopeId, name,
      path: targetPath ?? `${scopeId}/.mcp.json`,
      movable: false, deletable: true, locked: false, mcpConfig: config,
    } as (typeof s.items)[number];
    s.items.push(newItem);
    const undoId = this.newUndo(() => {
      const j = s.items.indexOf(newItem);
      if (j >= 0) s.items.splice(j, 1);
    });
    return { kind: 'mcp-upsert', originalPath: newItem.path, __undoId: undoId };
  }

  // Plan 19 — creatable skills: scaffold a new skill item (create-only in the
  // real backend; the mock just inserts the row so the Organizer's Add Skill
  // flow can be exercised). Undo splices the new item back out.
  skillUpsert(harness: string, scopeId: string, name: string, _content: string): MockRestore {
    const s = this.scanFor(harness);
    const newItem = {
      category: 'skill', scopeId, name,
      path: `${scopeId}/skills/${name}/SKILL.md`,
      movable: true, deletable: true, locked: false,
    } as (typeof s.items)[number];
    s.items.push(newItem);
    const undoId = this.newUndo(() => {
      const j = s.items.indexOf(newItem);
      if (j >= 0) s.items.splice(j, 1);
    });
    return { kind: 'skill-create', originalPath: newItem.path, __undoId: undoId };
  }

  getPolicy(): McpPolicy {
    return clone(this.policy);
  }

  setPolicy(policy: McpPolicy): MockRestore {
    const prev = this.policy;
    this.policy = clone(policy);
    const undoId = this.newUndo(() => { this.policy = prev; });
    return { kind: 'mcp-policy', originalPath: '~/.claude/settings.json', __undoId: undoId };
  }

  checkPolicy(serverName: string, cfg: { command?: string; args?: string[]; url?: string }, policy: McpPolicy): PolicyVerdict {
    for (const e of policy.denylist) if (this.matchesEntry(e, serverName, cfg)) return 'denied';
    if (policy.allowlist.length === 0) return 'noPolicy';
    for (const e of policy.allowlist) if (this.matchesEntry(e, serverName, cfg)) return 'allowed';
    return 'denied';
  }

  private matchesEntry(
    e: { serverName?: string; serverCommand?: string[]; serverUrl?: string },
    name: string,
    cfg: { command?: string; args?: string[]; url?: string },
  ): boolean {
    if (e.serverName && e.serverName === name) return true;
    if (e.serverCommand && cfg.command) {
      const cmd = [cfg.command, ...(cfg.args ?? [])];
      if (e.serverCommand.length === cmd.length && e.serverCommand.every((c, i) => c === cmd[i])) return true;
    }
    if (e.serverUrl && cfg.url && e.serverUrl === cfg.url) return true;
    return false;
  }

  // ── Security ──
  securityScan(): ScanResultSec {
    return clone(securityScan);
  }

  baselineCheck(): BaselineDiff[] {
    return clone(securityScan.baselineDiffs).filter((d) => !this.acceptedBaselines.has(`${d.server}::${d.tool}`));
  }

  baselineAccept(server: string, tools: string[]): void {
    for (const t of tools) this.acceptedBaselines.add(`${server}::${t}`);
  }

  // ── Context budget ──
  budget(scopeId: string): BudgetBreakdown {
    return budgetFor(scopeId);
  }

  // ── Sessions ──
  sessionPreview(path: string): Conversation { return conversationFor(path); }
  sessionCost(path: string): CostBreakdown { return costFor(path); }
  sessionDistill(path: string): DistillResult { return distillFor(path); }
  sessionTrim(path: string): MockRestore {
    const undoId = this.newUndo(() => { /* trim is a no-op to reverse in the mock */ });
    return { kind: 'file', originalPath: path, currentPath: path, __undoId: undoId };
  }

  // ── Backups (stateful) ──
  backupStatus(): BackupStatus { return clone(this.backup); }

  backupRun(): ExportReport {
    this.backup.hasRepo = true;
    return { filesCopied: 1811, bytesCopied: 4_200_000, skipped: ['projects/*/sessions (445 files) — excluded'] };
  }

  backupSync(): CommitInfo {
    this.backup.hasRepo = true;
    const sha = `${(this.backup.lastCommit ? 'b1c2d3e' : '9a8b7c6')}`.slice(0, 7);
    this.backup.lastCommit = sha;
    this.backup.lastCommitAt = '2026-07-05T09:20:00Z';
    return { committed: true, sha, message: 'ward: snapshot of ~/.claude', committedAt: this.backup.lastCommitAt };
  }

  backupPush(): PushResult {
    if (!this.backup.remoteUrl) return { pushed: false, reason: 'no remote configured', remoteUrl: null };
    return { pushed: true, reason: 'ok', remoteUrl: this.backup.remoteUrl };
  }

  schedulerInstall(intervalSeconds: number): void {
    this.backup.schedulerInstalled = true;
    this.backup.schedulerInterval = intervalSeconds;
  }

  schedulerRemove(): void {
    this.backup.schedulerInstalled = false;
    this.backup.schedulerInterval = null;
  }

  setRemote(url: string): void {
    this.backup.remoteUrl = url;
  }

  // A few realistic sample commits so the Backups history section is
  // populated in dev:mock and in vitest. Newest first, mirroring the real
  // `git log` order.
  backupLog(n = 20): GitLogEntry[] {
    const samples: GitLogEntry[] = [
      { sha: '9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d3e2f1a0b', subject: 'backup: ward (claude) 2026-07-05T09:20:00Z', author: 'ward', committedAt: '2026-07-05T09:20:00Z' },
      { sha: '1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d', subject: 'backup: ward sync 2026-07-05T04:00:00Z', author: 'ward', committedAt: '2026-07-05T04:00:00Z' },
      { sha: 'f0e1d2c3b4a596877869504b3c2d1e0f9a8b7c6d', subject: 'backup: ward (claude) 2026-07-04T22:10:00Z', author: 'ward', committedAt: '2026-07-04T22:10:00Z' },
      { sha: '7c6b5a4938271605f4e3d2c1b0a9f8e7d6c5b4a3', subject: 'backup: ward sync 2026-07-04T16:00:00Z', author: 'ward', committedAt: '2026-07-04T16:00:00Z' },
      { sha: '3e2d1c0b9a8f7e6d5c4b3a2918070f6e5d4c3b2a', subject: 'backup: ward (claude) 2026-07-04T09:00:00Z', author: 'ward', committedAt: '2026-07-04T09:00:00Z' },
    ];
    return samples.slice(0, Math.max(0, n));
  }

  // ── Usage engine + native shell (Plan 14/15) ──
  usageSnapshot(harness: string) {
    return clone(usageSnapshotFor(harness));
  }

  // Plan 16 — live Claude usage (Claude only, mirrors the backend command).
  usageSnapshotLive(harness: string) {
    if (harness !== 'claude') {
      throw { kind: 'harnessUnavailable', message: `live usage unsupported for ${harness}` };
    }
    return clone(liveSnapshotFor(harness));
  }

  // Plan 17 — last-known cached snapshot for instant cache-first paint. Seeded
  // so dev:mock opens the popover with gauges already visible: Claude shows the
  // live snapshot (dev:mock defaults live-enabled), Codex the local one.
  usageCached(harness: string) {
    return clone(harness === 'claude' ? liveSnapshotFor(harness) : usageSnapshotFor(harness));
  }

  liveUsageEnabled(): boolean {
    return this.liveEnabled;
  }

  setLiveUsageEnabled(enabled: boolean): void {
    this.liveEnabled = enabled;
  }

  autostartStatus(): boolean {
    return this.autostartEnabled;
  }

  autostartSet(enabled: boolean): void {
    this.autostartEnabled = enabled;
  }

  nativeUpdateStatus(): void {
    // no-op in the mock (native badge/tooltip has no browser surface)
  }
}
