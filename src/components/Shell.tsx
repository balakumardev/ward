import { JSX } from 'solid-js';
import { Sidebar } from './Sidebar';

export function Shell(props: { active: string; onSelect: (id: string) => void; children: JSX.Element }) {
  return (
    <div style={{ display: 'flex', height: '100vh' }}>
      <Sidebar active={props.active} onSelect={props.onSelect} />
      <main style={{ flex: 1, overflow: 'auto' }}>{props.children}</main>
    </div>
  );
}
