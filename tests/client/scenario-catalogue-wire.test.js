import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { LobbyState, activePacksView, scenarioOriginBadge } from '../../gui/lobby-state.js';

const fixture = JSON.parse(readFileSync(new URL('../fixtures/scenario-catalogue-wire.json', import.meta.url), 'utf8'));
const old = JSON.parse(readFileSync(new URL('../fixtures/scenario-catalogue-v3.json', import.meta.url), 'utf8'));

describe('the native and browser catalogue wire in the real phone reducer', () => {
  it('replaces no-pack, one-pack and two-pack state with the correct badges and rows', () => {
    const phone = new LobbyState();
    for (const { message } of fixture.snapshots.slice(0, 4)) {
      phone.apply(message);
      expect(phone.selectionLocked).toEqual({
        scenario_id: message.data.locked_scenario,
        template_path: message.data.locked_ship,
      });
      expect(phone.scenarioCatalog).toEqual(message.data.scenarios);
      expect(activePacksView(phone.activePacks)).toEqual(message.data.active_packs.map(({ name, version }) => ({ name, version })));
      expect(scenarioOriginBadge(phone.scenarioCatalog[0], phone.activePacks)).toBeNull();
      for (const scenario of phone.scenarioCatalog.slice(1)) {
        const pack = message.data.active_packs.find(p => p.id === scenario.source);
        expect(scenarioOriginBadge(scenario, phone.activePacks)).toEqual({ name: pack.name });
      }
      // The shared fixture's base manifest curates one of two authored hulls.
      expect(phone.scenarioCatalog[0].ships.map(s => s.template_path)).toEqual([
        'assets/entities/__catalogue_fixture/cruiser.toml',
      ]);
    }
  });

  it('retains rows after lock and Welcome and restores a fresh or reset phone', () => {
    const locked = fixture.snapshots.find(s => s.name === 'locked').message;
    const welcome = { type: 'Welcome', data: {
      state: { phase: 'Lobby', players: [], world: null },
      ship_stations: { stations: [] }, ship_config: {},
    } };
    for (const fresh of [true, false]) {
      const phone = new LobbyState();
      if (!fresh) { phone.apply(fixture.snapshots[2].message); phone.reset(); }
      phone.apply(welcome);
      phone.apply(locked);
      expect(phone.scenarioCatalog).toBeNull();
      expect(phone.activePacks).toEqual(locked.data.active_packs);
      phone.apply(welcome);
      expect(activePacksView(phone.activePacks)).toEqual([
        { name: 'Aurora Skirmish', version: '1.0.0' },
        { name: 'Script Valid', version: '1.0.0' },
      ]);
    }
  });

  it('still reads the pre-addition v3 message as base content with no packs', () => {
    const phone = new LobbyState();
    phone.apply(fixture.snapshots[2].message);
    phone.apply(old);
    expect(phone.activePacks).toEqual([]);
    expect(scenarioOriginBadge(phone.scenarioCatalog[0], phone.activePacks)).toBeNull();
  });

  it('the browser host delegates the complete wire envelope and sends it to late arrivals', () => {
    const html = readFileSync(new URL('../../server.html', import.meta.url), 'utf8');
    const body = html.match(/function scenarioCatalogMessage\(\) \{([\s\S]*?)\n    \}/)[1];
    const message = fixture.snapshots[2].message;
    const calls = [];
    const bindings = { wasm_scenario_catalog_message: (...args) => {
      calls.push(args);
      return JSON.stringify(message);
    } };
    const make = new Function('wasmBindings', '_scenarioCatalog', '_preSelection', body);
    expect(make(bindings, message.data.scenarios, { scenario_id: 'base', template_path: null })).toEqual(message);
    expect(calls).toEqual([[JSON.stringify(message.data.scenarios), 'base', null]]);
    const sendBody = html.match(/function sendCatalogTo\(conn\) \{([\s\S]*?)\n    \}/)[1];
    const received = [];
    const send = new Function('_catalogBuilt', '_worldLoadStarted', 'scenarioCatalogMessage', 'conn', sendBody);
    send(true, true, () => message, { send: json => received.push(JSON.parse(json)) });
    expect(received).toEqual([message]);
  });
});
