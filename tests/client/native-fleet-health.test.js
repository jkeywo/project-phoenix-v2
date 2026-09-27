// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { createFleetHealth } from '../../gui/fleet-health.js';
import { createNativeFleetPeer } from '../../gui/native-fleet-peer.js';
import { setBaseCatalogue, t } from '../../gui/strings.js';

beforeEach(() => {
  setBaseCatalogue(readFileSync('assets/strings/strings.csv', 'utf8'));
  document.body.replaceChildren();
});

describe('native fleet surface health', () => {
  it('renders native bridge transport, capacity and simulation health in one focus-stable status region', () => {
    let owner;
    const view = createFleetHealth({ doc: document, root: () => document.body, t });
    const peer = createNativeFleetPeer({ send: vi.fn(),
      createOwner(options) {
        owner = options;
        return { update() {}, setCrewReadiness() {}, setGmReady() {},
          setStartValidation() {}, broadcast() {}, close() {} };
      },
      onRoster: view.roster, onDiag: view.diagnostic, onHealth: view.health,
    });
    expect(peer.configure({ base: 'https://fleet.test', credentials: ['private-reconnect-proof'] })).toBe(true);
    owner.onRoster({ slots: Array.from({ length: 5 }, (_, index) => ({ id: `slot-${index + 1}` })),
      gms: Array.from({ length: 3 }, (_, index) => ({ id: `gm-${index + 1}` })) });
    const region = document.querySelector('[data-fleet-health]');
    expect(region.textContent).toContain('Beyond tested capacity: 5 ships and 3 Game Masters');
    region.focus();
    owner.onDiag({ event: 'transport', link: 'wire-2', identity: 'slot-2', name: 'Aurora', transport: 'ws-relay' });
    expect(region.textContent).toContain('Aurora: carried by the join service');
    peer.update({ health: { tick: 17, alerts: [], peers: [{ id: 'public-peer',
      ship: { entity_id: 'ship-2', name: 'Aurora' }, state: 'stale' }] } });
    expect(region.textContent).toContain(t('server.gm.health.state.stale'));
    owner.onDiag({ event: 'relay-degraded', link: 'wire-2', dropped: 3 });
    expect(region.textContent).toContain('Aurora: the relay cannot keep up with snapshots');
    expect(document.activeElement).toBe(region);
    expect(region.getAttribute('role')).toBe('status');
    expect(region.getAttribute('aria-live')).toBe('polite');
    expect(region.textContent).not.toContain('private-reconnect-proof');
    peer.close();
  });
});
