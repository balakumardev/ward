import { createResource, createSignal, onCleanup, onMount, For, Show } from 'solid-js';
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
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
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

async function safeUsage(harness: string): Promise<UsageSnapshot> {
  try {
    return await api.usageSnapshot(harness);
  } catch {
    return {
      harness,
      block: emptyWindow(),
      week: emptyWindow(),
      source: 'local',
      available: false,
      generatedAt: '',
    };
  }
}

function emptyWindow(): UsageWindow {
  return { tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 }, costUsd: 0, isActive: false };
}

function HarnessRow(props: { id: string; label: string; icon: string; snap: UsageSnapshot | undefined; nowMs: number }) {
  const block = () => props.snap?.block;
  // Live countdown: derive remaining from the absolute `resetsAt` (ISO) and the
  // ticking `nowMs`. Drift-free — re-fetches update `resetsAt`, never accumulate.
  const remaining = () => secsUntil(block()?.resetsAt, props.nowMs);
  const pct = () => {
    const p = block()?.percent;
    return p == null ? null : Math.round(p * 100);
  };
  const ramp = () => {
    const p = block()?.percent ?? 0;
    return p >= 0.9 ? 'crit' : p >= 0.7 ? 'warn' : 'ok';
  };
  return (
    <div class="pop-harness" data-testid={`pop-harness-${props.id}`}>
      <div class="pop-harness-head">
        <span class="pop-harness-icon">{props.icon}</span>
        <span class="pop-harness-name">{props.label}</span>
        <Show when={block()?.planType}>{(pt) => <span class="pop-plan">{pt()}</span>}</Show>
      </div>
      <Show
        when={props.snap?.available}
        fallback={<div class="pop-empty">No usage found</div>}
      >
        <Show
          when={pct() != null}
          fallback={
            <div class="pop-metric">
              <span class="pop-tokens">{fmtTokens(block()?.tokens.total ?? 0)} tok</span>
              <span class="pop-cost">${(block()?.costUsd ?? 0).toFixed(2)}</span>
            </div>
          }
        >
          <div class={`pop-gauge pop-gauge-${ramp()}`}>
            <div class="pop-gauge-fill" style={{ width: `${pct()}%` }} />
            <span class="pop-gauge-label">{pct()}%</span>
          </div>
        </Show>
        <Show when={remaining() != null}>
          <div class="pop-reset">resets in {fmtCountdown(remaining()!)}</div>
        </Show>
      </Show>
    </div>
  );
}

export default function Popover() {
  const [nowMs, setNowMs] = createSignal(Date.now());
  const [claude, { refetch: refetchClaude }] = createResource(() => safeUsage('claude'));
  const [codex, { refetch: refetchCodex }] = createResource(() => safeUsage('codex'));
  const [autostart, setAutostart] = createSignal<boolean>(false);

  const tick = setInterval(() => setNowMs(Date.now()), 1000);
  onCleanup(() => clearInterval(tick));

  function refetchAll() {
    void refetchClaude();
    void refetchCodex();
  }

  // Focus-gated poll: the tray popover window is hidden (not destroyed) on blur,
  // so an unconditional interval would read the user's full Claude/Codex history
  // off disk forever. Only poll while the window is focused/visible; stop on blur.
  let pollId: ReturnType<typeof setInterval> | undefined;
  const startPoll = () => { if (pollId == null) pollId = setInterval(() => refetchAll(), 20000); };
  const stopPoll = () => { if (pollId != null) { clearInterval(pollId); pollId = undefined; } };
  onCleanup(stopPoll);

  onMount(async () => {
    try {
      setAutostart(await api.autostartStatus());
    } catch {
      setAutostart(false);
    }
    if (isTauri()) {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        const win = getCurrentWindow();
        if (await win.isFocused().catch(() => true)) startPoll();
        const un = await win.onFocusChanged(({ payload }) => {
          if (payload) { refetchAll(); startPoll(); } else { stopPoll(); }
        });
        onCleanup(un);
      } catch { startPoll(); }
    } else {
      startPoll();
    }
  });

  async function toggleAutostart(e: Event) {
    const next = (e.currentTarget as HTMLInputElement).checked;
    try {
      await api.autostartSet(next);
      setAutostart(next);
    } catch {
      setAutostart(!next); // revert on failure
    }
  }

  async function fire(event: string, payload?: unknown) {
    try {
      const { emit } = await import('@tauri-apps/api/event');
      await emit(event, payload);
    } catch {
      /* dev:mock / non-Tauri — no-op */
    }
  }

  return (
    <div class="pop" data-testid="popover">
      <header class="pop-head">
        <span class="pop-title">Ward</span>
        <button class="pop-refresh" title="Refresh" onClick={() => refetchAll()} data-testid="pop-refresh">⟳</button>
      </header>
      <For each={HARNESSES}>
        {(h) => (
          <HarnessRow
            id={h.id}
            label={h.label}
            icon={h.icon}
            snap={h.id === 'claude' ? claude() : codex()}
            nowMs={nowMs()}
          />
        )}
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
}
