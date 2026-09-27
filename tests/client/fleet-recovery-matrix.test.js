// Protocol controls for #1534. These stand-ins do not prove runtime digest,
// snapshot/history recovery or star-owner loss; those need the runtime matrix.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FLEET_ROUTES, sixPeerProtocol } from './fleet-matrix-harness.js';
import { memberOn, settle } from './fleet-session-harness.js';

afterEach(() => vi.useRealTimers());

describe('six-peer protocol failure controls (#1534)', () => {
  for (const route of FLEET_ROUTES) {
    const advance = async () => {
      await vi.advanceTimersByTimeAsync(route === 'automatic-fallback' ? 120_000 : 1);
      await settle();
    };
    for (const departed of [2, 5]) it(`${route}: departed ${departed <= 4 ? 'ship' : 'GM'}`, async () => {
      vi.useFakeTimers();
      const fleet = await sixPeerProtocol(route, advance);
      try {
        fleet.members[departed - 2].member.close();
        await settle();
        expect(fleet.losses).toEqual([departed]);
        const roster = fleet.lead.fleet.roster();
        const row = departed <= 4 ? roster.slots.find(s => s.id === `slot-${departed}`)
          : roster.gms.find(g => g.id === fleet.members[departed - 2].member.operatorId);
        expect(row?.connected).toBe(false);
        expect(fleet.lead.simulationRosters).toHaveLength(1);
        for (const side of fleet.members.filter((_, index) => index !== departed - 2)) {
          expect(side.simulationRosters).toHaveLength(1);
        }
      } finally { fleet.close(); }
    });

    it(`${route}: replacement race admits one and protects the live holder`, async () => {
      vi.useFakeTimers();
      const fleet = await sixPeerProtocol(route, advance);
      const replacements = [];
      try {
        fleet.members[0].member.close();
        await settle();
        replacements.push(...await Promise.all([0, 1].map(() => memberOn(
          fleet.world, fleet.factories, fleet.lead.code.suffix,
          { claim: 'slot-2', levers: fleet.levers },
        ))));
        await advance();
        expect(replacements.filter(side => side.member.slot === 'slot-2')).toHaveLength(1);
        expect(replacements.flatMap(side => side.refusals.map(item => item.reason))).toEqual(['slot-taken']);
        const late = await memberOn(fleet.world, fleet.factories, fleet.lead.code.suffix,
          { claim: 'slot-2', levers: fleet.levers });
        replacements.push(late);
        await advance();
        expect(late.member.slot).toBeNull();
        expect(late.refusals.map(item => item.reason)).toEqual(['slot-taken']);
        expect(fleet.losses).toEqual([2]);
        expect(fleet.lead.fleet.roster().slots.find(s => s.id === 'slot-2').ship.template_path)
          .toBe('probe.toml');
      } finally {
        for (const side of replacements) side.member.close();
        fleet.close();
      }
    });
  }
});
