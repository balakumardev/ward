import { createResource, createSignal, onCleanup, onMount, For, Show, Switch, Match } from 'solid-js';
import { api, isTauri, type UsageSnapshot, type UsageWindow } from '../api';
import '../styles/popover.css';

const HARNESSES = [
  { id: 'claude', label: 'Claude Code', icon: '◆' },
  { id: 'codex', label: 'Codex CLI', icon: '◇' },
] as const;

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return `${n}`;
}

function fmtCountdown(secs: number): string {
  if (secs <= 0) return 'resetting…';
  const d = Math.floor(secs / 86_400);
  const h = Math.floor((secs % 86_400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m`;
  return `${secs}s`;
}

/** Seconds until `resetsAt` (ISO) relative to `nowMs`; null if absent/unparseable. Drift-free. */
export function secsUntil(resetsAt: string | undefined, nowMs: number): number | null {
  if (!resetsAt) return null;
  const ms = Date.parse(resetsAt);
  if (Number.isNaN(ms)) return null;
  return Math.max(0, Math.floor((ms - nowMs) / 1000));
}

/** Popover window sizing bounds (logical px). Width is fixed at 320. */
export const POPOVER_MIN_H = 120;
export const POPOVER_MAX_H = 600;

/** Clamp a measured content height into the window's allowed range.
 *  Non-finite / non-positive (e.g. jsdom `scrollHeight === 0`) → MIN. */
export function clampPopoverHeight(raw: number): number {
  if (!Number.isFinite(raw) || raw <= 0) return POPOVER_MIN_H;
  return Math.min(POPOVER_MAX_H, Math.max(POPOVER_MIN_H, Math.ceil(raw)));
}

/** Extract a displayable message from a rejected invoke (Tauri serializes
 *  WardError to `{kind, message}`); strip the internal prefix. */
function errMsg(e: unknown): string {
  const m = (e as { message?: string })?.message ?? String(e);
  return m.replace(/^live usage error:\s*/i, '');
}

function emptyWindow(): UsageWindow {
  return { tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 }, costUsd: 0, isActive: false };
}

function emptySnap(harness: string): UsageSnapshot {
  return { harness, block: emptyWindow(), week: emptyWindow(), source: 'local', available: false, generatedAt: '' };
}

/** Local snapshot, never throws (browser preview / disk error → empty). */
async function safeLocal(harness: string): Promise<UsageSnapshot> {
  try {
    return await api.usageSnapshot(harness);
  } catch {
    return emptySnap(harness);
  }
}

/** One labeled window row (5h / 7d): a percent gauge when the limit % is known,
 *  else tokens/cost when `tokensFallback` (local Claude), plus a reset line. */
function WindowRow(props: { label: string; w: UsageWindow; nowMs: number; tokensFallback?: boolean }) {
  const pct = () => (props.w.percent == null ? null : Math.round(props.w.percent * 100));
  const ramp = () => {
    const p = props.w.percent ?? 0;
    return p >= 0.9 ? 'crit' : p >= 0.7 ? 'warn' : 'ok';
  };
  const remaining = () => secsUntil(props.w.resetsAt, props.nowMs);
  return (
    <Switch>
      <Match when={pct() != null}>
        <div class="pop-window">
          <span class="pop-window-label">{props.label}</span>
          <div class={`pop-gauge pop-gauge-${ramp()}`}>
            <div class="pop-gauge-fill" style={{ width: `${pct()}%` }} />
            <span class="pop-gauge-label">{pct()}%</span>
          </div>
        </div>
        <Show when={remaining() != null}>
          <div class="pop-reset">{props.label} resets in {fmtCountdown(remaining()!)}</div>
        </Show>
      </Match>
      <Match when={props.tokensFallback}>
        <div class="pop-metric">
          <span class="pop-tokens">{fmtTokens(props.w.tokens.total)} tok</span>
          <span class="pop-cost">${props.w.costUsd.toFixed(2)}</span>
        </div>
        <Show when={remaining() != null}>
          <div class="pop-reset">resets in {fmtCountdown(remaining()!)}</div>
        </Show>
      </Match>
    </Switch>
  );
}

function HarnessRow(props: {
  id: string;
  label: string;
  icon: string;
  snap: UsageSnapshot | undefined;
  nowMs: number;
  error?: string;
  optInMode?: boolean;
  onEnable?: () => void;
  onRetry?: () => void;
}) {
  const block = () => props.snap?.block;
  const week = () => props.snap?.week;
  return (
    <div class="pop-harness" data-testid={`pop-harness-${props.id}`}>
      <div class="pop-harness-head">
        <span class="pop-harness-icon">{props.icon}</span>
        <span class="pop-harness-name">{props.label}</span>
        <Show when={block()?.planType}>{(pt) => <span class="pop-plan">{pt()}</span>}</Show>
      </div>
      <Switch fallback={<div class="pop-empty">No usage found</div>}>
        <Match when={props.error}>
          <div class="pop-error" data-testid="pop-live-error">
            <span>⚠ {props.error}</span>
          </div>
          <button class="pop-btn pop-btn-sm" onClick={() => props.onRetry?.()} data-testid="pop-live-retry">
            Retry
          </button>
        </Match>
        <Match when={props.optInMode}>
          <div class="pop-blurb">See your real 5-hour &amp; weekly limits — one call using your Claude login.</div>
          <button class="pop-btn" onClick={() => props.onEnable?.()} data-testid="pop-enable-live">
            Enable live usage
          </button>
        </Match>
        <Match when={props.snap?.available}>
          <WindowRow label="5h" w={block()!} nowMs={props.nowMs} tokensFallback />
          <Show when={week()?.percent != null}>
            <WindowRow label="7d" w={week()!} nowMs={props.nowMs} />
          </Show>
          <Show when={props.snap?.source === 'live'}>
            <div class="pop-source">live · from your Claude login</div>
          </Show>
        </Match>
      </Switch>
    </div>
  );
}

export default function Popover() {
  const [nowMs, setNowMs] = createSignal(Date.now());
  const [liveEnabled, setLiveEnabled] = createSignal(false);
  const [claudeError, setClaudeError] = createSignal<string | null>(null);
  const [autostart, setAutostart] = createSignal<boolean>(false);
  // Plan 17 — last-known snapshot from the local usage cache, painted instantly
  // on mount so the popover never opens with empty gauges while the (possibly
  // slow) live/local refresh runs in the background (stale-while-revalidate).
  const [cachedClaude, setCachedClaude] = createSignal<UsageSnapshot | undefined>(undefined);
  const [cachedCodex, setCachedCodex] = createSignal<UsageSnapshot | undefined>(undefined);

  // Size the tray popover window to its content so nothing scrolls (native
  // menu-bar behavior). Measure-then-resize: read the rendered content height
  // and set the window height, clamped to [MIN, MAX]. Beyond MAX the .pop
  // container scrolls internally (CSS safety net). No-ops outside Tauri.
  let popEl: HTMLDivElement | undefined;
  let ro: ResizeObserver | undefined;
  let lastFitH = 0;
  async function fitWindow() {
    if (!isTauri() || !popEl) return;
    const h = clampPopoverHeight(popEl.scrollHeight);
    if (h === lastFitH) return; // avoid redundant setSize churn
    lastFitH = h;
    try {
      const [{ getCurrentWindow }, { LogicalSize }] = await Promise.all([
        import('@tauri-apps/api/window'),
        import('@tauri-apps/api/dpi'),
      ]);
      await getCurrentWindow().setSize(new LogicalSize(320, h));
    } catch {
      /* non-Tauri window API — ignore */
    }
  }

  // Claude: live (gated network) when opted in under Tauri, else local/preview.
  // The resource is keyed on `liveEnabled` so opting in refetches immediately.
  async function fetchClaude(enabled: boolean): Promise<UsageSnapshot | undefined> {
    if (!isTauri()) return safeLocal('claude'); // plain browser preview
    if (!enabled) return undefined; // not opted in → show the opt-in affordance
    setClaudeError(null);
    try {
      return await api.usageSnapshotLive('claude');
    } catch (e) {
      setClaudeError(errMsg(e));
      return undefined;
    }
  }

  // Source is an always-truthy object (never a bare `false`, which Solid treats
  // as "not ready" and would skip the fetcher); re-runs whenever opt-in changes.
  const [claude, { refetch: refetchClaude }] = createResource(
    () => ({ enabled: liveEnabled() }),
    (src) => fetchClaude(src.enabled),
  );
  const [codex, { refetch: refetchCodex }] = createResource(() => safeLocal('codex'));

  // 1-second local tick drives the drift-free countdown (no fetching).
  const tick = setInterval(() => setNowMs(Date.now()), 1000);
  onCleanup(() => clearInterval(tick));

  function refetchAll() {
    void refetchClaude();
    void refetchCodex();
  }

  async function enableLive() {
    try {
      await api.setLiveUsageEnabled(true);
      setClaudeError(null);
      setLiveEnabled(true); // triggers the claude resource to refetch (live)
    } catch (e) {
      setClaudeError(errMsg(e));
    }
  }

  function retryLive() {
    setClaudeError(null);
    void refetchClaude();
  }

  onMount(async () => {
    // Keep the window fitted to content as usage data / opt-in state render in.
    // ResizeObserver fires once on observe, so the hidden window is already the
    // right height before the first tray-click shows it.
    if (popEl && typeof ResizeObserver !== 'undefined') {
      ro = new ResizeObserver(() => void fitWindow());
      ro.observe(popEl);
      onCleanup(() => ro?.disconnect());
    }
    // Cache-first paint (Plan 17): show the last-known gauges immediately from
    // the local usage cache while the resources below refresh in the background.
    // This is a cache READ only — it never triggers the gated live network call,
    // so the opt-in and no-silent-poll invariants are untouched.
    void api.usageCached('claude').then((s) => { if (s) setCachedClaude(s); }).catch(() => {});
    void api.usageCached('codex').then((s) => { if (s) setCachedCodex(s); }).catch(() => {});
    try {
      setAutostart(await api.autostartStatus());
    } catch {
      setAutostart(false);
    }
    // Live opt-in state (Tauri only; the browser preview has no Keychain).
    if (isTauri()) {
      try {
        setLiveEnabled(await api.liveUsageEnabled());
      } catch {
        setLiveEnabled(false);
      }
      // Refetch when the popover regains focus (i.e. is re-opened from the tray).
      // This is the ONLY automatic refresh — there is no silent background poll,
      // so the gated live call never fires unless the user opens the popover or
      // hits Refresh. The countdown stays live via the local 1s tick.
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        const win = getCurrentWindow();
        const un = await win.onFocusChanged(({ payload }) => {
          if (payload) refetchAll();
        });
        onCleanup(un);
      } catch {
        /* non-Tauri window API — ignore */
      }
    }
  });

  async function fire(event: string, payload?: unknown) {
    try {
      const { emit } = await import('@tauri-apps/api/event');
      await emit(event, payload);
    } catch {
      /* dev:mock / non-Tauri — no-op */
    }
  }

  const claudeOptIn = () => isTauri() && !liveEnabled();

  return (
    <div class="pop" data-testid="popover" ref={popEl}>
      <header class="pop-head">
        <span class="pop-title">Ward</span>
        <button class="pop-refresh" title="Refresh" onClick={() => refetchAll()} data-testid="pop-refresh">⟳</button>
      </header>
      <For each={HARNESSES}>
        {(h) =>
          h.id === 'claude' ? (
            <HarnessRow
              id={h.id}
              label={h.label}
              icon={h.icon}
              snap={claude() ?? cachedClaude()}
              nowMs={nowMs()}
              error={claudeError() ?? undefined}
              optInMode={claudeOptIn()}
              onEnable={enableLive}
              onRetry={retryLive}
            />
          ) : (
            <HarnessRow id={h.id} label={h.label} icon={h.icon} snap={codex() ?? cachedCodex()} nowMs={nowMs()} />
          )
        }
      </For>
      <footer class="pop-foot">
        <div class="pop-actions">
          <button class="pop-btn" onClick={() => fire('scan-now')} data-testid="pop-scan">Scan now</button>
          <button class="pop-btn" onClick={() => fire('tray_action', 'open')} data-testid="pop-open">Open</button>
        </div>
        <label class="pop-toggle">
          <input type="checkbox" checked={autostart()} onChange={toggleAutostart} data-testid="pop-autostart" />
          <span>Launch at login</span>
        </label>
      </footer>
    </div>
  );

  async function toggleAutostart(e: Event) {
    const next = (e.currentTarget as HTMLInputElement).checked;
    try {
      await api.autostartSet(next);
      setAutostart(next);
    } catch {
      setAutostart(!next); // revert on failure
    }
  }
}
