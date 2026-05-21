import { describe, it, expect } from 'vitest';
import { installDom, FakeElement, fireInput } from './slice-5-helpers.js';
import { renderFactionFormView } from '../faction-form-view.js';

function mount(opts) {
  installDom();
  const host = new FakeElement('div');
  renderFactionFormView(host, opts);
  return host;
}

describe('renderFactionFormView', () => {
  it('renders UUID as a read-only label (not an input)', () => {
    const host = mount({
      formState: {
        uuid: 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa',
        name: 'Federation',
        enemies: [],
      },
      enemyOptions: [],
      onNameChange: () => {},
      onEnemiesChange: () => {},
    });

    const ro = host.querySelector('.def-uuid-readonly');
    expect(ro).toBeTruthy();
    expect(ro.tagName).toBe('SPAN');
    expect(ro.textContent).toBe('aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa');

    // There must be no INPUT whose value equals the uuid (would be editable).
    const inputs = host.querySelectorAll('INPUT');
    for (const inp of inputs) {
      expect(inp.value).not.toBe('aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa');
    }
  });

  it('renders name as an editable text input wired to onNameChange', () => {
    let lastName = null;
    const host = mount({
      formState: { uuid: 'x', name: 'Federation', enemies: [] },
      enemyOptions: [],
      onNameChange: (n) => { lastName = n; },
      onEnemiesChange: () => {},
    });

    const input = host.querySelector('.def-name-input');
    expect(input).toBeTruthy();
    expect(input.type).toBe('text');
    expect(input.value).toBe('Federation');

    fireInput(input, 'United Federation');
    expect(lastName).toBe('United Federation');
  });

  it('enemy multi-select shows NAMES as option text and UUIDs as values (AC3)', () => {
    const host = mount({
      formState: { uuid: 'me', name: 'Me', enemies: [] },
      enemyOptions: [
        { uuid: 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa', name: 'Federation', path: 'a.toml' },
        { uuid: 'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb', name: 'Pirate', path: 'b.toml' },
      ],
      onNameChange: () => {},
      onEnemiesChange: () => {},
    });

    const select = host.querySelector('.def-multi-select');
    expect(select).toBeTruthy();
    expect(select.multiple).toBe(true);
    const options = select.querySelectorAll('OPTION');
    expect(options.length).toBe(2);

    // textContent must be the NAME (the human-readable label).
    const texts = options.map((o) => o.textContent).sort();
    expect(texts).toEqual(['Federation', 'Pirate']);

    // value must be the UUID (the canonical wire identity).
    const values = options.map((o) => o.value).sort();
    expect(values).toEqual([
      'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa',
      'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb',
    ]);
  });

  it('change event reports selected UUIDs (not names) via onEnemiesChange', () => {
    let lastEnemies = null;
    const host = mount({
      formState: { uuid: 'me', name: 'Me', enemies: [] },
      enemyOptions: [
        { uuid: 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa', name: 'Federation', path: 'a.toml' },
        { uuid: 'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb', name: 'Pirate', path: 'b.toml' },
      ],
      onNameChange: () => {},
      onEnemiesChange: (uuids) => { lastEnemies = uuids; },
    });

    const select = host.querySelector('.def-multi-select');
    const options = select.querySelectorAll('OPTION');
    // Toggle selection on the second option, then fire change.
    options[1].selected = true;
    select.dispatchEvent({ type: 'change', target: select });
    expect(lastEnemies).toEqual(['bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb']);

    // Pre-selected enemies render `option.selected = true`.
    const host2 = mount({
      formState: { uuid: 'me', name: 'Me', enemies: ['aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa'] },
      enemyOptions: [
        { uuid: 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa', name: 'Federation', path: 'a.toml' },
        { uuid: 'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb', name: 'Pirate', path: 'b.toml' },
      ],
      onNameChange: () => {},
      onEnemiesChange: () => {},
    });
    const opts2 = host2.querySelectorAll('OPTION');
    expect(opts2[0].selected).toBe(true);
    expect(opts2[1].selected).toBe(false);
  });

  it('renders a placeholder when no faction is open', () => {
    const host = mount({
      formState: null,
      enemyOptions: [],
      onNameChange: () => {},
      onEnemiesChange: () => {},
    });
    const placeholder = host.querySelector('.placeholder');
    expect(placeholder).toBeTruthy();
  });
});
