import { afterEach, describe, expect, it, vi } from 'vitest';
import { FLEET_ROUTES, sixPeerProtocol, STATIONS } from './fleet-matrix-harness.js';
import { settle } from './fleet-session-harness.js';

afterEach(() => vi.useRealTimers());

describe('six peers over the Phoenix protocol seam (#1530)', () => {
  for (const route of FLEET_ROUTES) it(route, async () => {
    vi.useFakeTimers();
    const fleet = await sixPeerProtocol(route, async () => {
      await vi.advanceTimersByTimeAsync(route === 'automatic-fallback' ? 120_000 : 1);
      await settle();
    });
    try {
      expect(fleet.members.map(side => side.refusals)).toEqual([[], [], [], [], []]);
      expect(fleet.lead.grants).toEqual([{ id: 'start-1', mode: 'automatic', operator_id: null }]);
      for (const side of [fleet.lead, ...fleet.members]) {
        const roster = side.simulationRosters.at(-1);
        expect(roster.participants).toEqual([1, 2, 3, 4, 5, 6]);
        expect(roster.ships).toHaveLength(4);
        expect(roster.gms).toHaveLength(2);
        expect(roster.ships.every(ship => JSON.stringify(ship.crew) === JSON.stringify(STATIONS))).toBe(true);
      }
      for (let slot = 2; slot <= 6; slot += 1) {
        const events = fleet.diagnostics.filter(event => event.slot === slot);
        expect(events.some(event => event.event === 'open')).toBe(true);
        expect(events.filter(event => event.event === 'transport').at(-1).transport)
          .toBe(route === 'direct' ? 'direct' : 'ws-relay');
        if (route === 'automatic-fallback') {
          expect(events.some(event => event.reason === 'direct-exhausted')).toBe(true);
          expect(events.filter(event => event.event === 'timeout')).toHaveLength(4);
        }
      }
    } finally { fleet.close(); }
  });
});
