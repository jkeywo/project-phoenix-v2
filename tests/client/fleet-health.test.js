// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { createFleetHealth } from '../../gui/fleet-health.js';
import { t } from '../../gui/strings.js';
import { sixPeerProtocol } from './fleet-matrix-harness.js';
import { memberOn, settle } from './fleet-session-harness.js';

function panel() {
  document.body.innerHTML = '<section id="fleet"></section>';
  return createFleetHealth({ doc: document, root: () => document.getElementById('fleet'), t });
}

describe('fleet operator health', () => {
  it('shows retries and fallback when real protocol offers remain unanswered', async () => {
    vi.useFakeTimers();
    const view = panel();
    const observed = [];
    let fleet;
    try {
      fleet = await sixPeerProtocol('automatic-fallback', async () => {
        await vi.advanceTimersByTimeAsync(120_000);
        await settle();
      }, { memberDiagnostic(event) {
        view.diagnostic(event);
        observed.push(document.body.textContent);
      } });
      expect(observed.some(text => text.includes('attempt 4'))).toBe(true);
      expect(document.body.textContent).toContain('carried by the join service');
      expect(document.body.textContent).not.toContain('retrying');
    } finally { fleet?.close(); vi.useRealTimers(); }
  });
  it('renders forced relay and a real protocol disconnect while keeping keyboard focus', async () => {
    const view = panel();
    const fleet = await sixPeerProtocol('ws-relay', settle, view);
    try {
      const region = document.querySelector('[data-fleet-health]');
      expect(region.textContent).toContain('carried by the join service');
      region.focus();
      fleet.members[1].member.close();
      await settle();
      expect(region.textContent).toContain('Peer 3');
      expect(region.textContent).toContain('disconnected');
      expect(document.activeElement).toBe(region);
      expect(region.getAttribute('role')).toBe('status');
      expect(region.textContent).not.toMatch(/fixture-capability|conn-\d/);
    } finally { fleet.close(); }
  });

  it('allows a fifth ship and warns from the actual admitted roster', async () => {
    const view = panel();
    // Keep the fleet mutable: the capacity question is admission, not recovery.
    const { makeWorld, makePeerFactory, leadOn } = await import('./fleet-session-harness.js');
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    const lead = await leadOn(world, factories, { onRoster: view.roster });
    const members = [];
    try {
      for (let i = 0; i < 4; i += 1) members.push(await memberOn(world, factories, lead.code.suffix));
      expect(members.at(-1).member.slot).toBe('slot-5');
      expect(members.at(-1).refusals).toEqual([]);
      expect(document.querySelector('[data-fleet-health]').textContent).toContain('Beyond tested capacity: 5 ships');
      for (let i = 0; i < 3; i += 1) members.push(await memberOn(world, factories, lead.code.suffix, { role: 'gm' }));
      expect(members.at(-1).refusals).toEqual([]);
      expect(document.querySelector('[data-fleet-health]').textContent).toContain('5 ships and 3 Game Masters');
    } finally { members.forEach(side => side.member.close()); lead.fleet.close(); }
  });

  it('uses the existing public health states for slow and restoring peers', () => {
    const view = panel();
    const base = { tick: 30, alerts: [], peers: [{ id: 'public-a', ship: { entity_id: 'ship', name: 'Aurora' }, state: 'stale' }] };
    view.health(base);
    expect(document.body.textContent).toContain('Aurora');
    expect(document.body.textContent).toContain(t('server.gm.health.state.stale'));
    view.health({ ...base, peers: [{ ...base.peers[0], state: 'recovering' }] });
    expect(document.body.textContent).toContain(t('server.gm.health.state.recovering'));
    expect(document.body.textContent).not.toContain(t('server.gm.health.state.stale'));
  });

  it('uses public GM names while keeping correlation ids out of the displayed health', () => {
    const view = panel();
    view.health({ tick: 31, alerts: [],
      operators: [{ id: 'gm-internal-correlation', name: 'Morgan', state: 'disconnected' }],
      peers: [{ id: 'public-gm', operators: ['gm-internal-correlation'], state: 'disconnected' }],
    });
    const region = document.querySelector('[data-fleet-health]');
    expect(region.textContent).toContain('Morgan');
    expect(region.textContent).not.toContain('gm-internal-correlation');
    view.health({ tick: 32, alerts: [], peers: [{ id: 'public-gm',
      operators: ['gm-internal-correlation'], state: 'recovering' }] });
    expect(region.textContent).toContain(t('server.fleet.health.link'));
    expect(region.textContent).not.toContain('gm-internal-correlation');
  });
});
