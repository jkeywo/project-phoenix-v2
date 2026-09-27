import { healthStateLabelId, parseGmHealthProjection } from './gm-health-banner.js';
import { has } from './strings.js';

/** Ratified support target (#1090), independent of the transport memory bound. */
export const TESTED_SHIP_HOSTS = 4;
export const TESTED_GM_PEERS = 2;

export function fleetHealthLines({ roster, links = [], health }, t) {
  const label = value => typeof value === 'string' && has(value) ? t(value) : value;
  const lines = [];
  const ships = roster?.slots?.length || 0;
  const gms = roster?.gms?.length || 0;
  if (ships > TESTED_SHIP_HOSTS || gms > TESTED_GM_PEERS) {
    lines.push(t('server.fleet.health.capacity', { ships, gms }));
  }
  for (const link of links) {
    const name = link.name ? label(link.name) : t('server.fleet.health.link');
    if (link.transport === 'ws-relay') lines.push(t('server.fleet.health.relay', { name }));
    if (link.state === 'reconnecting') {
      lines.push(t('server.fleet.health.reconnecting', { name, attempt: link.attempt || 1 }));
    }
    if (link.state === 'disconnected') lines.push(t('server.fleet.health.disconnected', { name }));
    if (link.dropped > 0) lines.push(t('server.fleet.health.shedding', { name }));
  }
  const projection = parseGmHealthProjection(health);
  if (projection) {
    for (const peer of projection.peers) {
      if (peer.state === 'live') continue;
      const name = peer.ship ? label(peer.ship.name) : peer.operators.join(', ') || t('server.fleet.health.link');
      lines.push(t('server.fleet.health.peer_state', {
        name, state: t(healthStateLabelId(peer.state)),
      }));
    }
    for (const alert of projection.alerts) {
      lines.push(t(alert.reason.id, Object.fromEntries(
        Object.entries(alert.reason.params).map(([key, value]) => [key, label(value)]),
      )));
    }
  }
  return [...new Set(lines)];
}

/** Only public labels and transport facts survive; credentials/addresses never do. */
export function createFleetHealth({ doc, root, t }) {
  const links = new Map();
  let roster = null;
  let health = null;
  let region = null;
  let lastText = null;
  function paint() {
    const parent = root();
    if (!parent) return;
    if (!region) {
      region = doc.createElement('div');
      region.dataset.fleetHealth = '';
      region.setAttribute('role', 'status');
      region.setAttribute('aria-live', 'polite');
      region.setAttribute('aria-atomic', 'true');
      region.tabIndex = 0;
      region.style.cssText = 'position:fixed;bottom:1rem;left:1rem;z-index:1100;'
        + 'max-width:min(34rem,calc(100vw - 2rem));max-height:30vh;overflow:auto;'
        + 'white-space:pre-line;background:Canvas;color:CanvasText;'
        + 'border:1px solid currentColor;padding:.5rem;font:inherit;';
      parent.appendChild(region);
    }
    const text = fleetHealthLines({ roster, health, links: [...links.values()] }, t).join('\n');
    region.hidden = !text;
    if (text !== lastText) { region.textContent = text; lastText = text; }
  }
  return {
    roster(value) { roster = value; paint(); },
    health(value) { health = value; paint(); },
    diagnostic(event) {
      const key = event.link || 'owner'; // Private key stays inside this endpoint.
      const link = links.get(key) || {};
      if (event.identity) {
        link.identity = event.identity;
        for (const [previousKey, previous] of links) {
          if (previousKey !== key && previous.identity === link.identity) links.delete(previousKey);
        }
      }
      if (typeof event.name === 'string' && event.name) link.name = event.name;
      if (event.event === 'transport') link.transport = event.transport;
      if (event.event === 'attempt') { link.attempt = event.attempt; link.state = 'reconnecting'; }
      if (event.event === 'open') { link.state = 'live'; link.dropped = 0; }
      if (event.event === 'closed') link.state = 'disconnected';
      if (event.event === 'relay-degraded') link.dropped = event.dropped;
      links.set(key, link);
      // Retain the bounded live fleet plus recent failed admission diagnostics.
      // This is presentation memory, independent of the tested support target.
      if (links.size > 64) {
        const stale = [...links].find(([, value]) => !value.identity);
        links.delete(stale ? stale[0] : links.keys().next().value);
      }
      paint();
    },
    reset() { links.clear(); roster = null; health = null; paint(); },
  };
}

if (typeof window !== 'undefined') window.fleetHealth = { createFleetHealth };
