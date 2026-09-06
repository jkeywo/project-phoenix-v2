// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  buildGmStationConsoleInput,
  createGmStationPuppet,
  GM_STATION_PENDING_CAPACITY,
  parseGmStationProjection,
} from '../../gui/gm-station-puppet.js';
import { initConsole } from '../../gui/console-core.js';

function projection({ operators = [], activity = [], results = [] } = {}) {
  return {
    ships: [{
      ship_id: 'ship-player-1',
      name: 'Resolute',
      stations: [{
        station_id: 'captain',
        name: 'Captain',
        console: 'gui/captain-console.html',
        rating: 'Backfill',
        operators,
      }],
      ship_config: {
        station_systems: { captain: ['red-alert'] },
        system_console_families: { 'red-alert': 'captain' },
        system_kinds: { 'red-alert': 'captain' },
        blackboard_console_families: {},
        station_tutorials: {},
        station_assist_gaps: {},
      },
      station_ratings: { captain: 'Backfill' },
      control_sources: { 'red-alert': operators.length ? 'Human' : 'Ai' },
      blackboards: [['red-alert', {
        kind: 'Captain',
        data: { red_alert: false },
      }]],
    }],
    activity,
    results,
  };
}

function mount() {
  document.body.innerHTML = `
    <p id="gm-station-pending"></p>
    <section id="gm-station-controls" hidden>
      <select id="gm-station-select"></select>
      <button id="gm-station-toggle"></button>
      <p id="gm-station-status"></p>
      <iframe id="gm-station-frame"></iframe>
      <ol id="gm-station-activity"></ol>
    </section>`;
}

describe('GM authentic Station projection', () => {
  beforeEach(mount);

  it('pins the local projection shape and unwraps ordinary tagged blackboards', () => {
    const value = projection({
      operators: ['gm-1'],
      activity: [{
        tick: 44,
        operator_id: 'gm-1',
        ship: 'ship-player-1',
        station: 'captain',
        target: 'red-alert',
        action: 'SetRedAlert',
      }],
    });
    expect(parseGmStationProjection(JSON.stringify(value))).toEqual(value);
    expect(parseGmStationProjection('{bad')).toBeUndefined();
    expect(parseGmStationProjection({ ships: [], activity: null, results: [] })).toBeUndefined();

    const input = buildGmStationConsoleInput(value, value.ships[0]);
    expect(input.blackboards['red-alert']).toEqual({ red_alert: false });
    expect(input.blackboardKinds['red-alert']).toBe('Captain');
    expect(input.stationPuppets.captain.latest_activity.operator_id).toBe('gm-1');
  });

  it('folds absolute world truth through the ordinary reducer for a stateful Helm console', () => {
    const value = projection({ operators: ['gm-1'] });
    const ship = value.ships[0];
    ship.stations = [{
      station_id: 'helm',
      name: 'Helm',
      console: 'gui/cruiser/helm.html',
      rating: 'Backfill',
      operators: ['gm-1'],
    }];
    ship.ship_config = {
      station_systems: {
        helm: ['drive-main'],
        sensors: ['sensor-main'],
        navigation: ['nav-main'],
      },
      system_console_families: {
        'drive-main': 'helm',
        'sensor-main': 'sensors',
        'nav-main': 'navigation',
      },
      system_kinds: {
        'drive-main': 'helm_thrust',
        'sensor-main': 'sensors',
        'nav-main': 'navigation',
      },
      blackboard_console_families: {},
      helm_radar_range: 777,
      helm_radar_shows: ['ship'],
      sensors_radar_range: 1337,
      sensors_radar_shows: ['ship'],
      sensors_radar_selects: ['ship'],
      nav_chart_range: 2048,
      nav_chart_shows: ['ship'],
      nav_chart_selects: ['ship'],
      hostile_arc_color: [0.2, 0.4, 0.6, 0.08],
      phaser_banks: [{
        id: 'fore', facing_deg: 12, fire_arc_deg: 145, cooldown_secs: 3,
      }],
      torpedo_tubes: [{ id: 'aft', facing_deg: 180, fire_arc_deg: 75 }],
      hull_id: 'NX-GM-1299',
      class: 'survey-frigate',
      station_tutorials: {
        helm: [{
          id: 'gm-helm-authored',
          title: 'tutorial.gm_helm.title',
          text: 'tutorial.gm_helm.text',
          trigger: { kind: 'first_visit' },
        }],
      },
      station_assist_gaps: {
        helm: { Backfill: ['helm.heading'] },
      },
    };
    ship.station_ratings = { helm: 'Backfill' };
    ship.control_sources = { 'drive-main': 'Human' };
    ship.blackboards = [];
    ship.entities = [{
      uuid: 'contact-1',
      name: 'contact.name',
      position: [1, 0, 2],
      tags: ['ship'],
      radar_icon: 'ship',
      region_colour: [0.1, 0.2, 0.3],
      radius: 12,
    }];
    ship.entity_states = [{
      uuid: 'contact-1',
      position: [140, 0, -90],
      yaw: 1.25,
      hull_fraction: 0.75,
      flags: [],
    }];
    ship.objectives = [{
      id: 'reach-contact',
      text: 'objective.reach_contact',
      mandatory: true,
      status: 'Active',
      targets: ['contact.name'],
      source: 'Mission',
    }];
    ship.ship_pose = { x: 125, y: 4, z: -75, yaw: 0.75, forward_speed: 18 };
    ship.navigation_waypoint = { x: 240, z: -160 };
    ship.console_hull = [{
      system_id: 'drive-main', display_name: 'Drive', current: 80, max_hp: 100,
      tier: 'Damaged', debuff_magnitude: 0.2,
    }];

    const input = buildGmStationConsoleInput(value, ship);
    expect(input.asteroids[0]).toMatchObject({
      uuid: 'contact-1',
      position: [140, 0, -90],
      yaw: 1.25,
      hull_fraction: 0.75,
    });
    expect(input.objectives[0].id).toBe('reach-contact');
    expect(input.navigationWaypoint).toEqual({ x: 240, z: -160 });
    expect(input).toMatchObject({
      helmRadarRange: 777,
      sensorsRadarRange: 1337,
      navChartRange: 2048,
      hostileArcColor: [0.2, 0.4, 0.6, 0.08],
      hullId: 'NX-GM-1299',
      stationTutorials: { helm: [expect.objectContaining({ id: 'gm-helm-authored' })] },
      stationAssistGaps: { helm: { Backfill: ['helm.heading'] } },
      phaserArcConfigs: [expect.objectContaining({ id: 'fore', fire_arc_deg: 145 })],
      torpedoArcConfigs: [expect.objectContaining({ id: 'aft', fire_arc_deg: 75 })],
    });
    expect(input.regions).toEqual([expect.objectContaining({
      uuid: 'contact-1', x: 140, z: -90, objective_target: true,
    })]);

    const helm = JSON.parse(window.buildConsoleState('helm', input));
    expect(helm).toMatchObject({
      x: 125,
      z: -75,
      yaw: 0.75,
      speed: 18,
      range: 777,
      waypoint: { x: 240, z: -160 },
      thrust_system_id: 'drive-main',
      hostile_arc_color: [0.2, 0.4, 0.6, 0.08],
    });
    expect(helm.blips).toEqual(expect.arrayContaining([
      expect.objectContaining({ uuid: 'contact-1' }),
      expect.objectContaining({ kind: 'waypoint' }),
    ]));

    const sensors = JSON.parse(window.buildConsoleState('sensors', input));
    expect(sensors.scan_range).toBe(1337);
    expect(sensors.blips).toEqual(expect.arrayContaining([
      expect.objectContaining({ uuid: 'contact-1', selectable: true }),
    ]));

    const navigation = JSON.parse(window.buildConsoleState('navigation', input));
    expect(navigation.radar_range).toBe(2048);
    expect(navigation.blips).toEqual(expect.arrayContaining([
      expect.objectContaining({ uuid: 'contact-1', selectable: true }),
    ]));
  });

  it('mounts the exact authored iframe and submits absolute takeover membership', () => {
    const submitStationPuppet = vi.fn(() => true);
    const controller = createGmStationPuppet({
      doc: document,
      win: window,
      t: id => id,
      getOperator: () => ({ id: 'gm-1', name: 'Morgan' }),
      submitStationPuppet,
      correlation: kind => `corr-${kind}`,
    });

    expect(controller.update(projection())).toBe(true);
    expect(document.getElementById('gm-station-controls').hidden).toBe(false);
    expect(document.getElementById('gm-station-frame').getAttribute('src'))
      .toBe('gui/captain-console.html');
    document.getElementById('gm-station-toggle').click();
    expect(submitStationPuppet).toHaveBeenCalledWith({
      ship: 'ship-player-1',
      station: 'captain',
      active: true,
      correlation: 'corr-takeover',
    });

    controller.update(projection({ operators: ['gm-1'] }));
    document.getElementById('gm-station-toggle').click();
    expect(submitStationPuppet).toHaveBeenLastCalledWith(expect.objectContaining({
      active: false,
      correlation: 'corr-release',
    }));
  });

  it('preserves authentic correlations and settles applied/refused feedback on the originating iframe', () => {
    const submitStationCommand = vi.fn(() => true);
    const updateActionFeedback = vi.fn(() => true);
    const activity = [{
      tick: 45,
      operator_id: 'gm-1',
      ship: 'ship-player-1',
      station: 'captain',
      target: 'red-alert',
      action: 'SetRedAlert',
    }];
    const controller = createGmStationPuppet({
      doc: document,
      win: window,
      t: (id, params = {}) => `${id}:${params.operator || ''}`,
      getOperator: () => ({ id: 'gm-1' }),
      submitStationCommand,
      correlation: kind => `corr-${kind}`,
    });
    controller.update(projection({ operators: ['gm-1'], activity }));
    document.getElementById('gm-station-frame').contentWindow.__updateActionFeedback = updateActionFeedback;

    expect(controller.issueConsoleAction(JSON.stringify({
      action: 'set_red_alert',
      console: 'captain',
      active: true,
      correlation: 'iframe-feedback-1',
    }))).toBe(true);
    expect(submitStationCommand).toHaveBeenCalledWith({
      ship: 'ship-player-1',
      station: 'captain',
      target: 'red-alert',
      payload: { type: 'SetRedAlert', data: { active: true } },
      correlation: 'iframe-feedback-1',
    });
    expect(controller.state().pendingCommands.size).toBe(1);
    controller.update(projection({ operators: ['gm-1'], activity, results: [] }));
    expect(updateActionFeedback).not.toHaveBeenCalled();
    expect(controller.state().pendingCommands.size).toBe(1);

    const appliedResult = {
      operator_id: 'gm-1',
      correlation: 'iframe-feedback-1',
      action_kind: 'station-command',
      requested_active: true,
      outcome: 'applied',
      tick: 46,
    };
    controller.update(projection({
      operators: ['gm-1'],
      activity,
      results: [appliedResult],
    }));
    expect(updateActionFeedback).toHaveBeenCalledWith({
      correlation: 'iframe-feedback-1',
      state: 'Applied',
    });
    expect(controller.state().pendingCommands.size).toBe(0);
    controller.update(projection({
      operators: ['gm-1'],
      activity,
      results: [appliedResult],
    }));
    expect(updateActionFeedback).toHaveBeenCalledTimes(1);

    expect(controller.issueConsoleAction(JSON.stringify({
      action: 'set_red_alert',
      console: 'captain',
      active: false,
      correlation: 'iframe-feedback-2',
    }))).toBe(true);
    controller.update(projection({
      operators: ['gm-1'],
      activity,
      results: [{
        operator_id: 'gm-1',
        correlation: 'iframe-feedback-2',
        action_kind: 'station-command',
        requested_active: true,
        outcome: 'refused',
        tick: 47,
        reason: 'station-not-puppeted',
      }],
    }));
    expect(updateActionFeedback).toHaveBeenLastCalledWith({
      correlation: 'iframe-feedback-2',
      state: 'Refused',
    });
    expect(updateActionFeedback).toHaveBeenCalledTimes(2);
    expect(controller.state().pendingCommands.size).toBe(0);
    const item = document.querySelector('#gm-station-activity li');
    expect(item.dataset.operator).toBe('gm-1');
    expect(item.dataset.target).toBe('red-alert');
  });

  it('immediately refuses an exact iframe correlation when GM ingress rejects it', () => {
    const submitStationCommand = vi.fn(() => false);
    const updateActionFeedback = vi.fn(() => true);
    const controller = createGmStationPuppet({
      doc: document,
      win: window,
      getOperator: () => ({ id: 'gm-1' }),
      submitStationCommand,
    });
    controller.update(projection({ operators: ['gm-1'] }));
    document.getElementById('gm-station-frame').contentWindow.__updateActionFeedback = updateActionFeedback;

    expect(controller.issueConsoleAction(JSON.stringify({
      action: 'set_red_alert',
      console: 'captain',
      active: true,
      correlation: 'iframe-ingress-refusal',
    }))).toBe(false);
    expect(updateActionFeedback).toHaveBeenCalledWith({
      correlation: 'iframe-ingress-refusal',
      state: 'Refused',
      reason: 'ingress-rejected',
    });
    expect(controller.state().pendingCommands.size).toBe(0);

    controller.update(projection({
      operators: ['gm-1'],
      results: [{
        operator_id: 'gm-1',
        correlation: 'iframe-ingress-refusal',
        action_kind: 'station-command',
        requested_active: true,
        outcome: 'refused',
        tick: 48,
        reason: 'station-not-puppeted',
      }],
    }));
    expect(updateActionFeedback).toHaveBeenCalledTimes(1);
  });

  it('bounds pending feedback, visibly expires the oldest, and ignores its late result', () => {
    const timers = [];
    const schedule = vi.fn((callback) => {
      timers.push(callback);
      return timers.length;
    });
    const cancelSchedule = vi.fn();
    const updateActionFeedback = vi.fn(() => true);
    const controller = createGmStationPuppet({
      doc: document,
      win: window,
      getOperator: () => ({ id: 'gm-1' }),
      submitStationCommand: () => true,
      pendingCapacity: 2,
      feedbackTimeoutMs: 25,
      schedule,
      cancelSchedule,
    });
    controller.update(projection({ operators: ['gm-1'] }));
    document.getElementById('gm-station-frame').contentWindow.__updateActionFeedback = updateActionFeedback;

    const issue = (correlation, active) => controller.issueConsoleAction(JSON.stringify({
      action: 'set_red_alert', console: 'captain', active, correlation,
    }));
    expect(issue('iframe-bounded-1', true)).toBe(true);
    expect(issue('iframe-bounded-2', false)).toBe(true);
    expect(issue('iframe-bounded-3', true)).toBe(true);

    expect(GM_STATION_PENDING_CAPACITY).toBeGreaterThanOrEqual(2);
    expect(controller.state().pendingCommands.size).toBe(2);
    expect([...controller.state().pendingCommands.values()].map(entry => entry.correlation))
      .toEqual(['iframe-bounded-2', 'iframe-bounded-3']);
    expect(updateActionFeedback).toHaveBeenCalledWith({
      correlation: 'iframe-bounded-1',
      state: 'TimedOut',
      reason: 'feedback-capacity',
    });
    expect(cancelSchedule).toHaveBeenCalledWith(1);

    controller.update(projection({
      operators: ['gm-1'],
      results: [{
        operator_id: 'gm-1',
        correlation: 'iframe-bounded-1',
        action_kind: 'station-command',
        requested_active: true,
        outcome: 'applied',
        tick: 49,
      }],
    }));
    expect(updateActionFeedback).toHaveBeenCalledTimes(1);

    timers[1]();
    expect(updateActionFeedback).toHaveBeenLastCalledWith({
      correlation: 'iframe-bounded-2',
      state: 'TimedOut',
      reason: 'feedback-timeout',
    });
    expect(controller.state().pendingCommands.size).toBe(1);
    controller.update(projection({
      operators: ['gm-1'],
      results: [{
        operator_id: 'gm-1',
        correlation: 'iframe-bounded-2',
        action_kind: 'station-command',
        requested_active: true,
        outcome: 'refused',
        tick: 50,
        reason: 'system-refused',
      }],
    }));
    expect(updateActionFeedback).toHaveBeenCalledTimes(2);
  });
});

describe('crew-visible takeover banner', () => {
  it('appears in the shared authentic console runtime and carries admission attribution', () => {
    document.body.innerHTML = '<main class="frame"></main>';
    initConsole({ name: 'captain', render: vi.fn() });
    window.__updateConsole('captain', JSON.stringify({
      gm_takeover: {
        station: 'captain',
        operators: ['gm-1'],
        latest_activity: {
          tick: 45,
          operator_id: 'gm-1',
          target: 'red-alert',
          action: 'SetRedAlert',
        },
      },
    }));
    const banner = document.getElementById('gm-takeover-banner');
    expect(banner.hidden).toBe(false);
    expect(banner.dataset.station).toBe('captain');
    expect(banner.dataset.latestOperator).toBe('gm-1');

    window.__updateConsole('captain', '{"gm_takeover":null}');
    expect(banner.hidden).toBe(true);
  });
});
