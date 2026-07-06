import { render } from '@solidjs/testing-library';
import { Sidebar, MODES, HARNESSES } from './Sidebar';

test('renders all six modes', () => {
  const { getByText } = render(() => (
    <Sidebar
      active="organizer"
      onSelect={() => {}}
      harness="claude"
      onSelectHarness={() => {}}
    />
  ));
  for (const m of MODES) getByText(m.label);
  expect(MODES.map((m) => m.id)).toEqual(['organizer', 'security', 'budget', 'sessions', 'backups', 'marketplace']);
});

test('renders all registered harnesses in the dropdown', () => {
  const { getByTestId } = render(() => (
    <Sidebar
      active="organizer"
      onSelect={() => {}}
      harness="claude"
      onSelectHarness={() => {}}
    />
  ));
  const select = getByTestId('harness-select') as HTMLSelectElement;
  const optionValues = Array.from(select.options).map((o) => o.value);
  expect(optionValues).toEqual(HARNESSES.map((h) => h.id));
});

test('reflects the current harness in the dropdown selection', () => {
  const { getByTestId } = render(() => (
    <Sidebar
      active="organizer"
      onSelect={() => {}}
      harness="codex"
      onSelectHarness={() => {}}
    />
  ));
  const select = getByTestId('harness-select') as HTMLSelectElement;
  expect(select.value).toBe('codex');
});
