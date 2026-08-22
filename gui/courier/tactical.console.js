/**
 * gui/courier/tactical.console.js — the Requiem courier's Tactical seat
 * (issue #1234).
 *
 * The courier's single tactical seat is multi-family: a weapons column (radar +
 * blasters, no phasers or tubes), a sensors column, and a helm column. The
 * shared renderer covers the radar + blaster + station-damage core; a bespoke
 * tail drives the sensors and helm panels, each reading its own system view.
 */
import { makeTacticalRender } from '../stations/tactical-console.js';
import { systemView } from '../console-payload.js';

export const renderStation = makeTacticalRender({
  weaponsView: (s) => systemView(s, 'tactical-radar', 'blaster-fore'),
  ids: {
    radar: 'tactical-radar',
    blasters: 'blasters',
    stationDamage: 'damage',
  },
  tail: (s, w, doc) => {
    const sensors = systemView(s, 'sensors', 'sensor-radar');
    const helm = systemView(s, 'helm-thrust', 'helm-joystick', 'helm-steering');

    const sensorRadar = doc.getElementById('sensor-radar');
    if (sensorRadar) sensorRadar.state = sensors;
    const sensorPanel = doc.getElementById('sensor-panel');
    if (sensorPanel) sensorPanel.state = sensors;

    const helmEl = doc.getElementById('helm');
    if (helmEl) helmEl.state = { auto: !!helm.helm_auto };
    const lateralEl = doc.getElementById('lateral');
    if (lateralEl) lateralEl.state = { auto: !!helm.lateral_auto };
    const impulseEl = doc.getElementById('impulse');
    if (impulseEl) {
      impulseEl.state = {
        state: helm.impulse_charge_progress > 0 ? 'charging' : 'ready',
        charge_pct: (helm.impulse_charge_progress || 0) * 100,
        auto: !!helm.helm_auto,
      };
    }
    const boostEl = doc.getElementById('boost');
    if (boostEl) {
      boostEl.state = {
        available: !!helm.boost_enabled,
        active: !!helm.boost_active,
        recharge_pct: helm.boost_battery != null ? helm.boost_battery * 100 : 100,
        auto: !!helm.helm_auto,
      };
    }
  },
});
