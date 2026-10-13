import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync } from 'node:fs';
import { parse } from 'smol-toml';
import { resolveTemplate } from '../../editor/entity-includes.js';

const read = path => readFileSync(path, 'utf8');
const hull = name => {
  const result = resolveTemplate(`assets/entities/${name}.toml`, read, parse);
  expect(result.ok).toBe(true);
  return result.resolved.value;
};
const feeds = (ship, id) => ship.console.find(console => console.id === id).feed;

describe('shipped explicit Console composition', () => {
  it.each([
    ['alliance_courier', 2, 4], ['alliance_destroyer', 4, 7], ['alliance_cruiser', 6, 8],
    ['alliance_battleship', 9, 9], ['dynasty_player_cruiser', 6, 7],
  ])('retains %s seats and separate control surfaces', (name, seats, consoles) => {
    const ship = hull(name);
    expect(ship.station).toHaveLength(seats);
    expect(ship.console).toHaveLength(consoles);
    expect(ship.station.every(station => station.console == null && !station.auxiliary)).toBe(true);
    for (const system of ship.system) {
      expect(system.station).toBeUndefined();
      expect(ship.console.some(console => console.id === system.console)).toBe(true);
    }
  });
  it('preserves detached Navigation automation independently from Tactical ratings', () => {
    const ship = hull('alliance_destroyer');
    const navigation = ship.console.find(console => console.id === 'navigation');
    expect(navigation.home_station).toBeUndefined();
    expect(navigation.visiting_rating).toBe('Simplified');
    expect(navigation.rating).toEqual([{ name: 'Std', automated_systems: [] }, { name: 'Simplified', automated_systems: ['navigation'] }]);
    expect(navigation.host_order).toEqual(['tactical', 'engineering', 'captain']);
    expect(ship.station.find(station => station.id === 'tactical').rating[1].automated_systems).toEqual(['phaser-omni']);
  });
  it.each(['alliance_cruiser', 'alliance_destroyer'])('preserves %s existing radar markers and Command advice feeds', name => {
    const ship = hull(name);
    expect(feeds(ship, 'helm')).toEqual(expect.arrayContaining([
      { source: 'navigation', kind: 'Navigation' }, { source: 'sensors', kind: 'Sensors' },
      { source: 'tactical', kind: 'Weapons' }, { source: 'tactical-radar', kind: 'TacticalRadar' },
      { source: 'tractor', kind: 'Tractor' }, { source: 'captain', kind: 'Captain' },
      { source: 'dossiers', kind: 'Dossiers' },
    ]));
    expect(feeds(ship, 'tactical')).toEqual(expect.arrayContaining([
      { source: 'sensors', kind: 'Sensors' }, { source: 'command', kind: 'Command' },
    ]));
  });
  it('keeps independently seeking services separate from fixed primary controls', () => {
    for (const [name, primary, services] of [['alliance_courier', 'captain', ['navigation', 'comms']], ['dynasty_player_cruiser', 'command', ['comms']]]) {
      const ship = hull(name);
      expect(ship.console.find(console => console.id === primary).human_seeking).not.toBe(true);
      for (const id of services) {
        const system = ship.system.find(system => system.id === id);
        expect(system.console).toBe(`${id}-service`);
        expect(ship.console.find(console => console.id === system.console)).toMatchObject({ home_station: primary, human_seeking: true });
      }
    }
  });
  it('reuses the authored Dock tree library only on the five complete Dock hulls', () => {
    for (const name of ['alliance_cruiser', 'alliance_destroyer', 'alliance_tender', 'dock_probe', 'umbilical_probe']) {
      const ship = hull(name);
      const dock = ship.system.find(system => system.kind === 'dock');
      expect(dock.config.ai.root, name).toBe('dock-standard');
      expect(dock.config.ai.tree.find(tree => tree.id === 'dock-standard').extends, name).toBe('dock-base');
      expect(dock.config.ai.tree.find(tree => tree.id === 'dock-base').state[0].task.some(task => task.action.kind === 'call'), name).toBe(true);
      expect(ship.system.filter(system => system.kind !== 'dock').every(system => !system.config?.ai), name).toBe(true);
    }
  });
  it('loads every authored entity with explicit ownership and no auxiliary Station records', () => {
    // Fragments are partial definitions; complete entity templates compose them before validation.
    for (const file of readdirSync('assets/entities', { recursive: true }).filter(file => file.endsWith('.toml') && !file.replaceAll('\\', '/').startsWith('fragments/'))) {
      const ship = hull(file.replaceAll('\\', '/').slice(0, -5));
      for (const system of ship.system || []) expect(ship.console?.some(console => console.id === system.console), `${file}:${system.id}`).toBe(true);
      expect((ship.station || []).some(station => station.auxiliary), file).toBe(false);
    }
  });
});
