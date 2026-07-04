import { render } from '@solidjs/testing-library';
import { Sidebar, MODES } from './Sidebar';

test('renders all five modes', () => {
  const { getByText } = render(() => <Sidebar active="organizer" onSelect={() => {}} />);
  for (const m of MODES) getByText(m.label);
  expect(MODES.map((m) => m.id)).toEqual(['organizer', 'security', 'budget', 'sessions', 'backups']);
});
