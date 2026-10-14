// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { parse } from 'smol-toml';
import { resolveTemplate } from '../../editor/entity-includes.js';

describe('Dynasty player-cruiser console delivery', () => {
  beforeEach(() => {
    document.head.innerHTML = '';
    vi.resetModules();
  });

  it('loads the authored Dynasty theme inside the Station iframe realm', async () => {
    await import('../../gui/dynasty-cruiser/console.js');
    const theme = document.querySelector('link[data-dynasty-cruiser-theme]');
    expect(theme).not.toBeNull();
    expect(theme.getAttribute('rel')).toBe('stylesheet');
    expect(theme.getAttribute('href')).toBe('../themes/dynasty-cruiser.css');
  });
  it('rehosts inherited logistics on the actual six-seat roster with complete physical machinery', () => {
    const root = path.resolve(import.meta.dirname, '../..');
    const resolved = resolveTemplate('assets/entities/dynasty_player_cruiser.toml', {
      read: file => fs.existsSync(path.join(root, file)) ? fs.readFileSync(path.join(root, file), 'utf8') : null,
    }, parse);
    expect(resolved.ok).toBe(true); const value = resolved.resolved.value;
    expect(value.station.map(row => row.id)).toEqual(['command', 'helm', 'gunnery', 'sensors', 'power', 'damage-control']);
    expect(value.console.some(row => row.id === 'engineering')).toBe(false);
    const handling = value.system.find(row => row.id === 'logistics');
    expect(handling.console).toBe('damage-control'); expect(handling.kind).toBe('logistics');
    expect(value.console.some(row => row.id === handling.console)).toBe(true);
    expect(value.console.find(row => row.id === handling.console).feed).toEqual(expect.arrayContaining([
      { source: 'logistics', kind: 'Logistics' }, { source: 'torpedo-magazine', kind: 'TorpedoMagazine' },
    ]));
    expect(value.station.find(row => row.id === 'damage-control').rating[0].automated_systems).toContain('logistics');
    expect(value.interior.placements.logistics).toBe('engineering');
    expect(handling.damage_component.map(row => row.binding)).toEqual(['operation', 'intake', 'cooling']);
    expect(value.energy.consumer.filter(row => row.system === 'logistics')).toHaveLength(1);
    expect(value.energy.group.filter(row => row.id === handling.power_group)).toHaveLength(1);
    expect(value.torpedoes.count).toBe(6); expect(value.torpedoes.initial_count).toBe(2);
    expect(value.infrastructure.capacity.find(row => row.id === 'hold-ammunition').amount + value.torpedoes.initial_count).toBe(6);
    expect(fs.readFileSync(path.join(root, 'gui/dynasty-cruiser/damage-control.html'), 'utf8')).toContain('<ph-logistics-controls id="logistics-panel"');
  });
});
