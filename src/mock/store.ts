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
  McpConfig, EnvVar, MarketEntry, MarketPage, BuiltConfig, InstallResult, InstallTarget,
  SkillPreview, PluginScan, PluginEntry, MarketplaceRef,
  SettingRow, SchemaDiff, EnvVarDef,
} from '../api';
import {
  codexScan, securityScan, budgetFor, conversationFor, costFor, distillFor, initialBackupStatus,
  usageSnapshotFor,
  liveSnapshotFor,
  MARKET_ENTRIES,
  MARKET_SKILLS,
  MARKET_SKILL_BODIES,
  PLUGIN_SCAN,
  SETTINGS_CATALOG,
  SETTINGS_SCHEMA_DIFF,
  SETTINGS_ENV_LIST,
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

// ── Marketplace helpers (mirror src-tauri/src/marketplace/install.rs) ──
const UNPINNED = 'refusing to install an unpinned version';

function deriveLocalName(registryName: string): string {
  const seg = registryName.split('/').pop()?.trim() ?? '';
  return seg || registryName.trim();
}

/** Non-secret vars with a provided value only; secrets are NEVER written. */
function buildSecretSafe(vars: EnvVar[], envValues: Record<string, string>): Record<string, string> {
  const obj: Record<string, string> = {};
  for (const v of vars) {
    if (v.isSecret) continue;
    const val = envValues[v.name];
    if (val) obj[v.name] = val;
  }
  return obj;
}

function mapRemoteType(t: string): string {
  return t.toLowerCase() === 'sse' ? 'sse' : 'http';
}

/** Minimal leading-`---` frontmatter reader (mirrors the Rust
 *  `parse_frontmatter`) — top-level `key: value` pairs only, quotes stripped. */
function parseFrontmatter(body: string): Record<string, string> {
  const out: Record<string, string> = {};
  const m = /^﻿?---\r?\n([\s\S]*?)\r?\n---/.exec(body);
  if (!m) return out;
  for (const raw of m[1].split(/\r?\n/)) {
    const line = raw.trimEnd();
    if (!line || line.startsWith(' ') || line.startsWith('\t')) continue;
    const i = line.indexOf(':');
    if (i < 0) continue;
    const key = line.slice(0, i).trim();
    let val = line.slice(i + 1).trim();
    if (
      (val.startsWith('"') && val.endsWith('"') && val.length >= 2) ||
      (val.startsWith("'") && val.endsWith("'") && val.length >= 2)
    ) {
      val = val.slice(1, -1);
    }
    if (key) out[key] = val;
  }
  return out;
}

/** Faithful mirror of the Rust `build_mcp_config` so the dev:mock preview,
 *  policy verdict, and install all behave like the native app (version-pin
 *  enforced, secrets omitted, npm→npx / pypi→uvx / remote→url+type). */
function buildMcpConfig(entry: MarketEntry, packageIndex: number, envValues: Record<string, string>): BuiltConfig {
  if (entry.packages.length > 0) {
    const pkg = entry.packages[packageIndex];
    if (!pkg) throw new Error(`package index ${packageIndex} out of range for '${entry.name}'`);
    const version = (pkg.version ?? '').trim();
    if (!version || version.toLowerCase() === 'latest') throw new Error(UNPINNED);
    let command: string;
    let args: string[];
    if (pkg.registryType === 'npm') { command = 'npx'; args = ['-y', `${pkg.identifier}@${version}`]; }
    else if (pkg.registryType === 'pypi') { command = 'uvx'; args = [`${pkg.identifier}==${version}`]; }
    else if (pkg.registryType === 'oci') {
      throw new Error(`'${entry.name}' ships an OCI/container image; install it with your container runtime — Ward will not run docker for you`);
    } else throw new Error(`unsupported package registry type '${pkg.registryType}' for '${entry.name}'`);
    const env = buildSecretSafe(pkg.env, envValues);
    const config: McpConfig = { command, args };
    if (Object.keys(env).length) config.env = env;
    return { name: deriveLocalName(entry.name), config, commandPreview: [command, ...args], env: pkg.env };
  }
  if (entry.remotes.length > 0) {
    const remote = entry.remotes[packageIndex];
    if (!remote) throw new Error(`remote index ${packageIndex} out of range for '${entry.name}'`);
    if (!remote.url.trim()) throw new Error(`remote for '${entry.name}' has no url`);
    const headers = buildSecretSafe(remote.headers, envValues);
    const config: McpConfig = { url: remote.url, type: mapRemoteType(remote.transport) };
    if (Object.keys(headers).length) config.headers = headers;
    return { name: deriveLocalName(entry.name), config, commandPreview: [remote.url], env: remote.headers };
  }
  throw new Error(`entry '${entry.name}' has no packages or remotes to install`);
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
  // Plan 28 — Plugins mode. Cloned from the fixture so enable/install/uninstall
  // and marketplace-add mutate real in-memory state (the next scan reflects it).
  private pluginMarketplaces: MarketplaceRef[];
  private pluginEntries: PluginEntry[];
  // Plan 29 — Settings mode. Cloned from the fixture so set/unset mutate real
  // in-memory rows (the next catalog read reflects the change; undo reverts it).
  private settings: SettingRow[];

  constructor() {
    this.claude = JSON.parse(scanClaudeRaw) as ScanResult;
    this.codex = clone(codexScan);
    this.backup = initialBackupStatus();
    this.pluginMarketplaces = clone(PLUGIN_SCAN.marketplaces);
    this.pluginEntries = clone(PLUGIN_SCAN.plugins);
    this.settings = clone(SETTINGS_CATALOG);
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

  // ── Marketplace (Plan 21 MCP, Plan 22 Skills) ──
  /** Search the synthetic catalog for a `kind` of unit (`"mcp"` → registry
   *  servers, `"skill"` → curated skills), filtered by substring over name /
   *  display name / description. Any other kind → empty page. */
  marketplaceSearch(kind: string, query: string, _cursor?: string): MarketPage {
    const list = kind === 'mcp' ? MARKET_ENTRIES : kind === 'skill' ? MARKET_SKILLS : [];
    const q = query.trim().toLowerCase();
    const entries = (q
      ? list.filter((e) =>
          e.name.toLowerCase().includes(q) ||
          e.displayName.toLowerCase().includes(q) ||
          e.description.toLowerCase().includes(q))
      : list
    ).map(clone);
    return { entries };
  }

  /** Pre-install preview — mirror the Rust `marketplace_preview_skill`: return
   *  the synthetic SKILL.md body with its frontmatter name/description (the
   *  catalog entry is the fallback). Binds approval to the actual content. */
  marketplacePreviewSkill(entry: MarketEntry): SkillPreview {
    const body =
      MARKET_SKILL_BODIES[entry.name] ??
      `---\nname: ${entry.name}\ndescription: ${entry.description}\n---\n\n# ${entry.name}\n`;
    const fm = parseFrontmatter(body);
    return {
      name: fm.name || entry.name,
      description: fm.description || entry.description,
      body,
    };
  }

  /** Pre-install preview — mirrors the Rust `build_mcp_config`. */
  marketplaceBuildConfig(entry: MarketEntry, packageIndex: number, envValues: Record<string, string>): BuiltConfig {
    return buildMcpConfig(entry, packageIndex, envValues);
  }

  /** Fan an install out to every target. Builds per target so an unpinned /
   *  OCI entry fails that target without aborting the batch; a success upserts
   *  a new MCP item into the target's scan so the Organizer reflects it. */
  marketplaceInstall(entry: MarketEntry, packageIndex: number, targets: InstallTarget[], envValues: Record<string, string>): InstallResult[] {
    return targets.map((target) => {
      try {
        if (entry.kind === 'skill') {
          // Create-only, mirroring the Rust `skill_upsert`: a target that
          // already has this skill fails that target without aborting the batch.
          const dup = this.scanFor(target.harness).items.some(
            (i) => i.category === 'skill' && i.name === entry.name && i.scopeId === target.scopeId,
          );
          if (dup) return { target, ok: false, error: `Skill '${entry.name}' already exists` };
          const body = MARKET_SKILL_BODIES[entry.name] ?? '';
          const restore = this.skillUpsert(target.harness, target.scopeId, entry.name, body);
          return { target, ok: true, restore };
        }
        const built = buildMcpConfig(entry, packageIndex, envValues);
        const restore = this.upsertMcpEntry(target.harness, target.scopeId, built.name, built.config);
        return { target, ok: true, restore };
      } catch (e) {
        return { target, ok: false, error: e instanceof Error ? e.message : String(e) };
      }
    });
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

  // ── Plugins (Plan 28) ──
  /** Fresh snapshot (new object + array/entry identity) so Solid re-renders
   *  after every plugin mutation. The mock CLI is always "available". */
  pluginScan(): PluginScan {
    return {
      marketplaces: this.pluginMarketplaces.map(clone),
      plugins: this.pluginEntries.map(clone),
      cliAvailable: true,
    };
  }

  pluginsCliAvailable(): boolean {
    return true;
  }

  private locatePlugin(pluginKey: string): number {
    return this.pluginEntries.findIndex((p) => `${p.name}@${p.marketplace}` === pluginKey);
  }

  /** Surgical enable/disable flip on the matching entry (mirrors the real
   *  single-key `enabledPlugins[...]` write). Undo restores the prior flag. */
  setPluginEnabled(pluginKey: string, enabled: boolean): MockRestore {
    const path = '~/.claude/settings.json';
    const idx = this.locatePlugin(pluginKey);
    if (idx < 0) return { kind: 'plugin-enable', originalPath: path, __undoId: '' };
    const prev = this.pluginEntries[idx].enabled;
    this.pluginEntries[idx] = { ...this.pluginEntries[idx], enabled };
    const undoId = this.newUndo(() => {
      const j = this.locatePlugin(pluginKey);
      if (j >= 0) this.pluginEntries[j] = { ...this.pluginEntries[j], enabled: prev };
    });
    return { kind: 'plugin-enable', originalPath: path, __undoId: undoId };
  }

  /** Install a plugin at `scope`: mark an existing catalog entry installed +
   *  enabled, or append a fresh entry when it's not in the catalog. Returns a
   *  fresh scan (mirrors the CLI-backed command re-scanning after install). */
  installPlugin(plugin: string, marketplace: string, scope: string): PluginScan {
    const idx = this.pluginEntries.findIndex((p) => p.name === plugin && p.marketplace === marketplace);
    if (idx >= 0) {
      this.pluginEntries[idx] = { ...this.pluginEntries[idx], installed: true, enabled: true, scope };
    } else {
      const src = this.pluginMarketplaces.find((m) => m.name === marketplace)?.source ?? { source: 'github', repo: marketplace };
      this.pluginEntries.push({
        kind: 'plugin', name: plugin, marketplace, displayName: plugin,
        description: '', source: src, tags: [], installed: true, enabled: true, scope,
      });
    }
    return this.pluginScan();
  }

  /** Uninstall a plugin (by bare name or `name@marketplace` key): mark it
   *  not-installed + disabled. Returns a fresh scan. `scope` is accepted for
   *  parity with the real command but not needed to locate the mock entry. */
  uninstallPlugin(plugin: string, _scope: string): PluginScan {
    const idx = this.pluginEntries.findIndex(
      (p) => p.installed && (p.name === plugin || `${p.name}@${p.marketplace}` === plugin),
    );
    if (idx >= 0) {
      this.pluginEntries[idx] = { ...this.pluginEntries[idx], installed: false, enabled: false, scope: undefined };
    }
    return this.pluginScan();
  }

  /** Add a marketplace derived from `src` (a `owner/repo`, URL, or path) if
   *  not already known, then return a fresh scan. */
  marketplaceAddPlugin(src: string, _scope: string): PluginScan {
    const name = src.split('/').pop()?.trim() || src.trim();
    if (name && !this.pluginMarketplaces.some((m) => m.name === name)) {
      this.pluginMarketplaces.push({
        name,
        source: { source: 'github', repo: src },
        installLocation: `~/.claude/plugins/marketplaces/${name}`,
        lastUpdated: new Date().toISOString(),
      });
    }
    return this.pluginScan();
  }

  /** Update one (`name`) or every (`undefined`) marketplace. A no-op mutation
   *  in the mock — just returns a fresh scan. */
  marketplaceUpdatePlugins(_name?: string): PluginScan {
    return this.pluginScan();
  }

  // ── Settings (Plan 29) ──
  /** The curated catalog joined with live effective values. Cloned per read so
   *  Solid's createResource sees fresh identity after every set/unset. */
  settingsCatalog(): SettingRow[] {
    return this.settings.map(clone);
  }

  private locateSetting(key: string): number {
    return this.settings.findIndex((r) => r.def.key === key);
  }

  private settingsPath(targetFile: string): string {
    return targetFile === 'claudeJson' ? '~/.claude.json' : '~/.claude/settings.json';
  }

  /** Surgical single-key write (mirrors the real `set_setting`): mark the row
   *  set with its new effective value and source `user`. Undo restores the row
   *  verbatim. `_scope` is accepted for parity with the real command; the mock
   *  always reflects the write as the winning `user`-scope value. */
  settingsSet(_scope: string, key: string, targetFile: string, value: unknown): MockRestore {
    const path = this.settingsPath(targetFile);
    const idx = this.locateSetting(key);
    if (idx < 0) return { kind: 'setting-write', originalPath: path, __undoId: '' };
    const prev = clone(this.settings[idx]);
    this.settings[idx] = { ...this.settings[idx], effective: value, sourceScope: 'user', isSet: true };
    const undoId = this.newUndo(() => { this.settings[idx] = prev; });
    return { kind: 'setting-write', originalPath: path, __undoId: undoId };
  }

  /** Reset a key to its default (mirrors the real `unset_setting` — remove the
   *  key, never write null/[]): effective falls back to `def.default`, source
   *  becomes `default`, `isSet` false. Undo restores the prior row verbatim. */
  settingsUnset(_scope: string, key: string, targetFile: string): MockRestore {
    const path = this.settingsPath(targetFile);
    const idx = this.locateSetting(key);
    if (idx < 0) return { kind: 'setting-write', originalPath: path, __undoId: '' };
    const prev = clone(this.settings[idx]);
    this.settings[idx] = {
      ...this.settings[idx],
      effective: this.settings[idx].def.default,
      sourceScope: 'default',
      isSet: false,
    };
    const undoId = this.newUndo(() => { this.settings[idx] = prev; });
    return { kind: 'setting-write', originalPath: path, __undoId: undoId };
  }

  settingsSchemaDiff(): SchemaDiff {
    return clone(SETTINGS_SCHEMA_DIFF);
  }

  settingsEnvList(): EnvVarDef[] {
    return clone(SETTINGS_ENV_LIST);
  }
}
