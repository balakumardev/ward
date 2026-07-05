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
    <div class="shell">
      <Sidebar
        active={props.active}
        onSelect={props.onSelect}
        harness={props.harness}
        onSelectHarness={props.onSelectHarness}
      />
      <main class="shell-main">{props.children}</main>
    </div>
  );
}
