/** Renderers for the six-seat T5 Dynasty cruiser (issue #1521).
 *
 * These compose the same family renderers used by the shipped Alliance hulls.
 * Station ownership changes presentation and admission routing, never the
 * underlying system contract.
 */
import { familyView } from '../console-payload.js';
import { makeCaptainRender } from '../stations/captain-console.js';
import { makeHelmRender } from '../stations/helm-console.js';
import { makeTacticalRender } from '../stations/tactical-console.js';
import { makeScienceRender } from '../stations/science-console.js';
import { makeEngineeringRender } from '../stations/engineering-console.js';

// Ship CSS is installed on the client shell, while each Station is an iframe
// realm. Load the same theme here so the authored Dynasty identity reaches the
// actual controls rather than stopping at their parent document.
if (typeof document !== 'undefined' && !document.querySelector('link[data-dynasty-cruiser-theme]')) {
  const theme = document.createElement('link');
  theme.rel = 'stylesheet';
  theme.href = '../themes/dynasty-cruiser.css';
  theme.dataset.dynastyCruiserTheme = '';
  document.head.append(theme);
}

export const renderCommand = makeCaptainRender({
  captainView: (s) => familyView(s, 'captain'),
  ids: { camera: 'camera', redAlert: 'red-alert', objectives: 'objectives' },
  tail: (s, _view, doc) => {
    const comms = familyView(s, 'comms');
    const contacts = doc.getElementById('contacts');
    if (contacts) contacts.state = { contacts: comms.contacts || [] };
    const message = doc.getElementById('message');
    if (message) {
      const messages = comms.messages || [];
      message.state = {
        thread: messages.find((item) => !item.is_read) || messages.at(-1) || null,
        rejection: comms.rejection,
      };
    }
  },
});

const renderHelmFamily = makeHelmRender({
  ids: {
    radar: 'helm-radar', joystick: 'helm-joystick', lateral: 'lateral-thrust',
    impulse: 'impulse', boost: 'boost', autoBadge: 'auto-badge',
  },
});
export function renderHelm(s, doc = document) {
  renderHelmFamily(familyView(s, 'helm'), doc);
  const nav = familyView(s, 'navigation');
  const map = doc.getElementById('navigation-map');
  if (map) {
    map.state = {
      blips: nav.blips || [], regions: nav.regions || [], range: nav.radar_range || 800,
      ship_pos: { x: nav.ship_x || 0, z: nav.ship_z || 0 },
      ship_heading: nav.ship_heading || 0, waypoint: nav.waypoint || null,
      auto: !!nav.navigation_auto,
    };
  }
}

export const renderGunnery = makeTacticalRender({
  weaponsView: (s) => familyView(s, 'tactical'),
  ids: {
    radar: 'tactical-radar', phasers: 'phasers', torpedo: 'torpedoes',
    autoBadge: 'auto-badge', targetCard: 'target-card',
  },
  torpedoMaxDefault: 20,
});

export const renderSensors = makeScienceRender({
  ids: {
    sensorRadar: 'sensor-radar', sensorPanel: 'sensor-panel',
    footer: 'sensor-target', autoBadge: 'auto-badge',
  },
});

export const renderPower = makeEngineeringRender({
  ids: { power: 'power-controls', battery: 'battery-bar' },
});

export const renderDamageControl = makeEngineeringRender({
  ids: {
    shieldFacings: 'shield-facings', threatRow: 'threat-row',
    threatBearing: 'threat-bearing', hullIntegrity: 'hull-integrity',
    coreDamage: 'core-damage', repairTeams: 'repair-teams',
  },
});
