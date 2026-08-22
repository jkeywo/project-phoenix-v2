// @vitest-environment jsdom
//
// Issue #1150 — the debug dock's four migrated legacy overlays.
//
// The renderers live in gui/debug-overlays.js precisely so they can be driven
// here without a browser or a WASM bundle: each is a pure function of the
// structured payload its Rust surface now publishes. These tests pin that they
// parse defensively and render the facts the retired text overlays used to
// print — now as DOM the dock drops in.

import { describe, it, expect } from 'vitest';
import {
  parseDebugPayload,
  buildModifierDebug,
  buildDamageDebug,
  buildEntityBehaviorDebug,
  buildEntityInspectorDebug,
  renderModifierDebug,
  renderDamageDebug,
  renderEntityBehaviorDebug,
  renderEntityInspectorDebug,
} from '../../gui/debug-overlays.js';

describe('parseDebugPayload', () => {
  it('parses a well-formed payload', () => {
    expect(parseDebugPayload('{"schema_version":1}')).toEqual({ schema_version: 1 });
  });
  it('returns null for the empty string (before the first publish)', () => {
    expect(parseDebugPayload('')).toBeNull();
  });
  it('returns null for malformed JSON rather than throwing', () => {
    expect(parseDebugPayload('{not json')).toBeNull();
  });
  it('returns null for a non-object payload', () => {
    expect(parseDebugPayload('42')).toBeNull();
  });
});

describe('buildModifierDebug', () => {
  const payload = {
    schema_version: 1,
    flags: [{ flag: 'CommsJammed', sources: ['ImpulseDrive'] }],
    float_modifiers: [
      { slot: 'MaxSpeed', multiplier: 1.5, contributions: [{ source: 'ImpulseDrive', bonus: 0.5 }] },
    ],
    int_modifiers: [
      { slot: 'RepairTeams', sum: 2, contributions: [{ source: 'ImpulseDrive', bonus: 2 }] },
    ],
  };

  it('renders the active flag with its source', () => {
    const el = buildModifierDebug(payload, { doc: document });
    const row = el.querySelector('.dbg-row[data-flag="CommsJammed"]');
    expect(row).not.toBeNull();
    expect(row.textContent).toContain('ImpulseDrive');
  });

  it('renders a float modifier with its multiplier and contribution', () => {
    const el = buildModifierDebug(payload, { doc: document });
    const row = el.querySelector('.dbg-row[data-slot="MaxSpeed"]');
    expect(row.textContent).toContain('×1.50');
    expect(row.textContent).toContain('ImpulseDrive');
    expect(row.textContent).toContain('+0.50');
  });

  it('renders an int modifier with its sum', () => {
    const el = buildModifierDebug(payload, { doc: document });
    const row = el.querySelector('.dbg-row[data-slot="RepairTeams"]');
    expect(row.textContent).toContain('+2');
  });

  it('shows (none) for an empty section', () => {
    const el = buildModifierDebug(
      { schema_version: 1, flags: [], float_modifiers: [], int_modifiers: [] },
      { doc: document },
    );
    expect(el.querySelectorAll('.dbg-empty')).toHaveLength(3);
  });
});

describe('buildDamageDebug', () => {
  it('renders each event newest-first with source, arc and amount', () => {
    const el = buildDamageDebug(
      {
        schema_version: 1,
        entries: [
          { source: 'region-zone', shield_arc: null, amount: 3.0 },
          { source: 'asteroid-42', shield_arc: 'Fore', amount: 12.5 },
        ],
      },
      { doc: document },
    );
    const rows = el.querySelectorAll('.dbg-row');
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain('region-zone');
    expect(rows[0].textContent).toContain('—'); // null arc → em-dash
    expect(rows[1].textContent).toContain('asteroid-42');
    expect(rows[1].textContent).toContain('Fore');
    expect(rows[1].textContent).toContain('12.5');
  });

  it('shows the empty placeholder when there is no damage', () => {
    const el = buildDamageDebug({ schema_version: 1, entries: [] }, { doc: document });
    expect(el.querySelector('.dbg-empty')).not.toBeNull();
  });
});

describe('buildEntityBehaviorDebug', () => {
  it('renders a row per entity with position and target, and a count', () => {
    const el = buildEntityBehaviorDebug(
      {
        schema_version: 1,
        entries: [{ name: 'Raider', x: 1, y: 2, z: 3, target: 'player' }],
      },
      { doc: document },
    );
    expect(el.querySelector('.dbg-section-title').textContent).toContain('(1)');
    const row = el.querySelector('.dbg-row[data-name="Raider"]');
    expect(row.textContent).toContain('target=player');
  });
});

describe('buildEntityInspectorDebug', () => {
  const payload = {
    schema_version: 1,
    player: {
      x: 10,
      z: 20,
      hull: [{ system: 'core', current: 50, max: 100 }],
      shields: [{ label: 'Fore', hp: 20, max_hp: 40, offline: false, focused: true }],
    },
    entities: [
      {
        name: 'Scout',
        tags: ['ship'],
        x: 13,
        z: 24,
        distance: 5,
        faction: 'Hostiles',
        hull_current: 30,
        hull_max: 60,
        comms_hailable: true,
        comms_in_range: true,
        comms_range: 500,
        ai_target: 'none',
      },
    ],
  };

  it('renders the player block with hull and shields', () => {
    const el = buildEntityInspectorDebug(payload, { doc: document });
    const player = el.querySelector('.dbg-player');
    expect(player).not.toBeNull();
    expect(player.querySelector('[data-field="hull"]').textContent).toContain('core 50/100');
    expect(player.querySelector('[data-field="shields"]').textContent).toContain('*Fore 20/40');
  });

  it('renders each entity with faction, hull, comms and ai lines', () => {
    const el = buildEntityInspectorDebug(payload, { doc: document });
    const block = el.querySelector('.dbg-entity[data-name="Scout"]');
    expect(block.textContent).toContain('[ship]');
    expect(block.textContent).toContain('faction: Hostiles');
    expect(block.textContent).toContain('hull: 30/60');
    expect(block.textContent).toContain('comms: hailable (in range)');
    expect(block.textContent).toContain('ai: target=none');
  });

  it('omits the player block when there is no player', () => {
    const el = buildEntityInspectorDebug(
      { schema_version: 1, player: null, entities: [] },
      { doc: document },
    );
    expect(el.querySelector('.dbg-player')).toBeNull();
  });
});

describe('render wrappers', () => {
  it('render into a container from raw JSON, clearing prior content', () => {
    const container = document.createElement('div');
    renderDamageDebug(container, JSON.stringify({ schema_version: 1, entries: [] }));
    expect(container.querySelector('.dbg-damage')).not.toBeNull();
    // Re-rendering with an empty string shows the placeholder, not appended.
    renderDamageDebug(container, '');
    expect(container.querySelectorAll('.dbg-damage')).toHaveLength(0);
    expect(container.querySelector('.dbg-empty')).not.toBeNull();
  });

  it('every renderer shows a placeholder before any data arrives', () => {
    for (const render of [
      renderModifierDebug,
      renderDamageDebug,
      renderEntityBehaviorDebug,
      renderEntityInspectorDebug,
    ]) {
      const container = document.createElement('div');
      render(container, '');
      expect(container.querySelector('.dbg-empty')).not.toBeNull();
    }
  });
});
