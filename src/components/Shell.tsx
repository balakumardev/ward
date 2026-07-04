import { JSX } from 'solid-js';
import { Sidebar } from './Sidebar';
import type { HarnessId } from './Sidebar';

export function Shell(props: {
  active: string;
  onSelect: (id: string) => void;
  harness: HarnessId;
  onSelectHarness: (id: HarnessId) => void;
  children: JSX.Element;
}) {
  return (
    <div style={{ display: 'flex', height: '100vh' }}>
      <Sidebar
        active={props.active}
        onSelect={props.onSelect}
        harness={props.harness}
        onSelectHarness={props.onSelectHarness}
      />
      <main style={{ flex: 1, overflow: 'auto' }}>{props.children}</main>
    </div>
  );
}