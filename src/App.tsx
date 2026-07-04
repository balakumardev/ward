import { createResource, createSignal, Show } from 'solid-js';
import { Shell } from './components/Shell';
import { Organizer } from './modes/Organizer';
import { api } from './api';

export default function App() {
  const [mode, setMode] = createSignal('organizer');
  const [scan, { refetch }] = createResource(() => api.scan('claude'));

  // Bridge api → organizer-shape. We re-scan after every mutation so
  // the UI reflects the new on-disk state.
  const organizerApi = {
    listDestinations: (item: Parameters<typeof api.listDestinations>[1]) =>
      api.listDestinations('claude', item),
    moveItem: async (item: Parameters<typeof api.moveItem>[1], destScopeId: string) => {
      const r = await api.moveItem('claude', item, destScopeId);
      await refetch();
      return r;
    },
    deleteItem: async (item: Parameters<typeof api.deleteItem>[1]) => {
      const r = await api.deleteItem('claude', item);
      await refetch();
      return r;
    },
    restore: async (info: Parameters<typeof api.restore>[1]) => {
      await api.restore('claude', info);
      await refetch();
    },
    bulkRestore: async (infos: Parameters<typeof api.bulkRestore>[1]) => {
      await api.bulkRestore('claude', infos);
      await refetch();
    },
    saveFile: async (path: string, content: string) => {
      await api.saveFile(path, content);
      await refetch();
    },
    bulk: async (items: Parameters<typeof api.bulk>[1], op: 'move' | 'delete', destScopeId?: string) => {
      const r = await api.bulk('claude', items, op, destScopeId);
      await refetch();
      return r;
    },
    // restore is used for both single + bulk undo. We dispatch via the
    // backend's bulk_restore when given multiple infos.
    // (handled inline in Organizer.tsx by calling `bulkRestore` via api)
  };

  return (
    <Shell active={mode()} onSelect={setMode}>
      <Show when={scan()} fallback={<div style={{ padding: '16px' }}>Scanning ~/.claude…</div>}>
        {(result) => (
          <Show when={mode() === 'organizer'} fallback={<div style={{ padding: '16px', color: 'var(--text-dim)' }}>Coming in a later plan.</div>}>
            <Organizer scan={result()} loadFile={api.readFileContent} api={organizerApi as never} />
          </Show>
        )}
      </Show>
    </Shell>
  );
}
