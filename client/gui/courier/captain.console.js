/**
 * gui/courier/captain.console.js — the courier's Captain seat (issue #1235).
 *
 * The courier is the compact hull: its Captain seat absorbs every system
 * the ship has no dedicated Station for — Shields, Power, Repair,
 * Navigation and Comms all live behind panels here rather than on their own
 * seats (Navigation and Comms are toggled overlays). Only the shared core
 * (camera/red-alert/objectives/station-damage) fits the common renderer; the
 * camera-select view list is filtered to the hull's two authored views, and
 * everything else — shields, power, battery, hull, repair, the Nav/Comms
 * overlays and their footer-adjacent threat-bearing readout — is this
 * hull's bespoke tail. This Captain seat carries no contact-count footer and
 * no AUTO badge, so `variant.footer`/`ids.autoBadge` stay unset.
 */
import { makeCaptainRender } from '../stations/captain-console.js';
import { systemView } from '../console-payload.js';

export const renderStation = makeCaptainRender({
  captainView: (s) => systemView(s, 'captain', 'viewscreen', 'red-alert'),
  ids: {
    camera: 'camera',
    redAlert: 'red-alert',
    objectives: 'objectives',
    stationDamage: 'damage',
  },
  // The Courier's compact bridge exposes only its Fore hull view and the
  // authored Cinematic view, even if the model carries extra markers.
  filterCameraViews: (views) => views.filter((view) => view === 'camera_fore' || view === 'cinematic'),
  tail: (s, view, doc) => {
    const shields = systemView(s, 'shields-system', 'shield-arc-fore', 'shield-arc-aft');
    const power = systemView(s, 'power-reactor', 'power-battery');
    const repair = systemView(s, 'repair');
    const nav = systemView(s, 'navigation');
    const comms = systemView(s, 'comms');

    const shieldsEl = doc.getElementById('shields');
    if (shieldsEl) shieldsEl.state = { facings: shields.facings || [], focused_facing: shields.focused_facing || null, auto: !!shields.shields_auto };

    // Compact threat-bearing readout (issue #926) — same standing fact the
    // battleship's dedicated Shields console shows, condensed to one line
    // for this column's tighter layout. Hidden when Sensors holds no threat.
    const threatRow = doc.getElementById('threat-row');
    const threatBearingEl = doc.getElementById('threat-bearing');
    if (threatRow && threatBearingEl) {
      if (shields.threat_bearing != null) {
        threatRow.classList.add('active');
        threatBearingEl.textContent = Math.round(shields.threat_bearing) + '°M';
      } else {
        threatRow.classList.remove('active');
        threatBearingEl.textContent = '—';
      }
    }

    const powerEl = doc.getElementById('power');
    if (powerEl) powerEl.state = { groups: power.consoles || [], auto: !!power.power_auto };
    const batteryEl = doc.getElementById('battery');
    if (batteryEl) {
      batteryEl.state = {
        level_pct: power.battery_max ? power.battery_charge / power.battery_max * 100 : 0,
        charging: !!power.battery_online && !!power.charging,
        emergency_threshold_pct: 20,
      };
    }

    const hullEl = doc.getElementById('hull');
    if (hullEl) hullEl.state = { total_pct: repair.overall_hull?.pct ?? 1, destroyed_pct: repair.overall_hull?.destroyed_pct };
    const repairEl = doc.getElementById('repair');
    if (repairEl) repairEl.state = { teams: repair.teams || [], auto: !!repair.repair_auto, targets: repair.dispatch_targets || [], damaged: repair.damaged_systems || [] };

    const navEl = doc.getElementById('nav');
    if (navEl) {
      navEl.state = {
        blips: nav.blips || [], regions: nav.regions || [], range: nav.radar_range || 800,
        ship_pos: { x: nav.ship_x || 0, z: nav.ship_z || 0 }, ship_heading: nav.ship_heading || 0,
        waypoint: nav.waypoint || null,
      };
    }

    const contactsEl = doc.getElementById('contacts');
    if (contactsEl) contactsEl.state = { contacts: comms.contacts || [] };
    const messageEl = doc.getElementById('message');
    if (messageEl) {
      const messages = comms.messages || [];
      messageEl.state = { thread: messages.find((m) => !m.is_read) || messages.at(-1) || null, rejection: comms.rejection };
    }
  },
});
