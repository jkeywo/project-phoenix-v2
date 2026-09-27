// Reusable six-peer protocol cases. Socket and RTC stand-ins are explicit;
// these cases do not claim browser/native rendering or real-network timings.
import { HOST_ROLE_GM } from '../../gui/host-mesh.js';
import { transportLeversFromLocation } from '../../gui/transport-levers.js';
import { makeWorld, makePeerFactory, leadOn, memberOn, settle } from './fleet-session-harness.js';

export const FLEET_ROUTES = ['direct', 'ws-relay', 'automatic-fallback'];
export const STATIONS = [['captain', 'Std'], ['engineering', 'Std'], ['helm', 'Std']];

export async function sixPeerProtocol(route, advance = settle, observe = {}) {
  if (!FLEET_ROUTES.includes(route)) throw new Error(`Unknown route: ${route}`);
  const world = makeWorld();
  const peer = makePeerFactory();
  const factories = { socket: world.socket, peer: () => {
    const instance = peer();
    if (route === 'automatic-fallback') instance.createOffer = () => new Promise(() => {});
    return instance;
  } };
  const levers = transportLeversFromLocation(
    route === 'automatic-fallback' ? '' : `?transport=${route}`);
  const diagnostics = [];
  const losses = [];
  let credential = 0;
  const lead = await leadOn(world, factories, {
    levers, ship: { template_path: 'probe.toml' },
    credentialFactory: () => `fixture-capability-${++credential}`,
    onHostLost: slot => losses.push(slot),
    onDiag: event => observe.diagnostic?.(event),
    onRoster: roster => { observe.roster?.(roster); },
  });
  const members = [];
  for (let slot = 2; slot <= 6; slot += 1) {
    const side = await memberOn(world, factories, lead.code.suffix, {
      levers, role: slot <= 4 ? 'ship' : HOST_ROLE_GM,
      name: `Peer ${slot}`, ship: slot <= 4 ? { template_path: 'probe.toml' } : null,
      onDiag: event => {
        diagnostics.push({ slot, ...event });
        observe.memberDiagnostic?.({ ...event, link: slot, name: `Peer ${slot}` });
      },
    });
    await advance();
    members.push(side);
  }
  const ships = [lead.fleet, ...members.slice(0, 3).map(side => side.member)];
  for (const ship of ships) {
    ship.update({ ready: true });
    ship.setCrewReadiness({ connected: 3, ready: 3 }, STATIONS);
    ship.setStartValidation(true);
  }
  for (const { member } of members.slice(3)) {
    member.setGmReady(true);
    member.setStartValidation(true);
  }
  await advance();
  return { world, factories, lead, members, diagnostics, losses, levers,
    close() { for (const side of members) side.member.close(); lead.fleet.close(); } };
}
