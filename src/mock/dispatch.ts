// Dev-only command router for the mock Tauri bridge (see ./install.ts).
//
// Maps every `invoke(cmd, args)` the frontend can issue (mirrors ../api.ts)
// onto the stateful mock store. Small artificial delays are added to the
// "heavy" commands so the real loading states ("Scanning…", spinners) are
// visible and testable.

import { MockStore } from './store';

const store = new MockStore();
const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

// Tauri passes command args as a camelCase object; the mock reads them
// directly. `any` here is deliberate — this is the untyped IPC seam.
type Args = Record<string, any>;

export async function mockInvoke(cmd: string, args: Args = {}): Promise<unknown> {
  switch (cmd) {
    // ── Organizer / scan ──
    case 'scan': await delay(140); return store.scan(args.harness ?? 'claude');
    case 'read_file_content': await delay(20); return store.readFile(args.path);
    case 'list_destinations': return store.listDestinations(args.harness ?? 'claude', args.item);
    case 'move_item': await delay(60); return store.moveItem(args.harness ?? 'claude', args.item, args.destScopeId);
    case 'delete_item': await delay(60); return store.deleteItem(args.harness ?? 'claude', args.item);
    case 'restore': await delay(40); store.restore(args.info); return null;
    case 'save_file': await delay(30); store.saveFile(args.path, args.content); return null;
    case 'bulk': await delay(80); return store.bulk(args.harness ?? 'claude', args.items, args.op, args.destScopeId);
    case 'bulk_restore': await delay(60); store.bulkRestore(args.infos); return null;

    // ── MCP controls & policy ──
    case 'mcp_get_disabled': return store.getDisabled(args.projectPath);
    case 'mcp_set_disabled': await delay(40); return store.setDisabled(args.projectPath, args.list);
    case 'mcp_upsert_entry':
      await delay(60);
      return store.upsertMcpEntry(args.harness ?? 'claude', args.scopeId, args.name, args.config, args.targetPath);
    case 'skill_upsert':
      await delay(60);
      return store.skillUpsert(args.harness ?? 'claude', args.scopeId, args.name, args.content);
    case 'mcp_get_policy': return store.getPolicy();
    case 'mcp_set_policy': await delay(40); return store.setPolicy(args.policy);
    case 'mcp_check_policy': return store.checkPolicy(args.serverName, args.serverConfig ?? {}, args.policy);

    // ── Marketplace (Plan 21) ──
    case 'marketplace_search': await delay(260); return store.marketplaceSearch(args.kind ?? 'mcp', args.query ?? '', args.cursor);
    case 'marketplace_build_config': return store.marketplaceBuildConfig(args.entry, args.packageIndex ?? 0, args.envValues ?? {});
    case 'marketplace_install': await delay(120); return store.marketplaceInstall(args.entry, args.packageIndex ?? 0, args.targets ?? [], args.envValues ?? {});
    case 'marketplace_preview_skill': await delay(120); return store.marketplacePreviewSkill(args.entry);

    // ── Security ──
    case 'security_scan': await delay(420); return store.securityScan();
    case 'security_baseline_check': await delay(60); return store.baselineCheck();
    case 'security_baseline_accept': store.baselineAccept(args.server, args.findings); return null;

    // ── Context budget ──
    case 'context_budget': await delay(220); return store.budget(args.scopeId ?? 'global');

    // ── Sessions ──
    case 'session_preview': await delay(120); return store.sessionPreview(args.path);
    case 'session_cost': await delay(120); return store.sessionCost(args.path);
    case 'session_distill': await delay(300); return store.sessionDistill(args.path);
    case 'session_trim': await delay(80); return store.sessionTrim(args.path);

    // ── Backups ──
    case 'backup_status': await delay(60); return store.backupStatus();
    case 'backup_run': await delay(300); return store.backupRun();
    case 'backup_sync': await delay(200); return store.backupSync();
    case 'backup_push': await delay(200); return store.backupPush();
    case 'backup_scheduler_install': store.schedulerInstall(args.intervalSeconds); return null;
    case 'backup_scheduler_remove': store.schedulerRemove(); return null;
    case 'backup_set_remote': store.setRemote(args.url); return null;
    case 'backup_log': await delay(60); return store.backupLog(args.n ?? 20);

    // ── Usage engine + native shell (Plan 14/15/16/17) ──
    case 'usage_snapshot': await delay(120); return store.usageSnapshot(args.harness ?? 'claude');
    // Plan 17 — cache read is instant (no artificial delay): it's the fast path
    // the popover paints from while the slower snapshot above refreshes.
    case 'usage_cached': return store.usageCached(args.harness ?? 'claude');
    case 'usage_snapshot_live': await delay(150); return store.usageSnapshotLive(args.harness ?? 'claude');
    case 'live_usage_enabled': return store.liveUsageEnabled();
    case 'set_live_usage_enabled': store.setLiveUsageEnabled(!!args.enabled); return null;
    case 'autostart_status': return store.autostartStatus();
    case 'autostart_set': store.autostartSet(!!args.enabled); return null;
    case 'native_update_status': store.nativeUpdateStatus(); return null;

    // ── Plugins (Plan 28) ──
    case 'plugins_scan': await delay(140); return store.pluginScan();
    case 'plugins_cli_available': return store.pluginsCliAvailable();
    case 'plugins_set_enabled': await delay(40); return store.setPluginEnabled(args.pluginKey, !!args.enabled);
    case 'plugins_install': await delay(220); return store.installPlugin(args.plugin, args.marketplace, args.scope ?? 'user');
    case 'plugins_uninstall': await delay(220); return store.uninstallPlugin(args.plugin, args.scope ?? 'user');
    case 'plugins_marketplace_add': await delay(180); return store.marketplaceAddPlugin(args.src, args.scope ?? 'user');
    case 'plugins_marketplace_update': await delay(180); return store.marketplaceUpdatePlugins(args.name);

    default:
      throw new Error(`[ward-mock] unhandled command: ${cmd}`);
  }
}
