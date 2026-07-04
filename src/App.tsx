import { createResource, createSignal, Show } from 'solid-js';
import { Shell } from './components/Shell';
import { Organizer } from './modes/Organizer';
import { api } from './api';

export default function App() {
  const [mode, setMode] = createSignal('organizer');
  const [scan] = createResource(() => api.scan('claude'));

  return (
    <Shell active={mode()} onSelect={setMode}>
      <Show when={scan()} fallback={<div style={{ padding: '16px' }}>Scanning ~/.claude…</div>}>
        {(result) => (
          <Show when={mode() === 'organizer'} fallback={<div style={{ padding: '16px', color: 'var(--text-dim)' }}>Coming in a later plan.</div>}>
            <Organizer scan={result()} loadFile={api.readFileContent} />
          </Show>
        )}
      </Show>
    </Shell>
  );
}
