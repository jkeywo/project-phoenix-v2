// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  buildGmStationConsoleInput,
  createGmStationPuppet,
  GM_STATION_PENDING_CAPACITY,
  parseGmStationProjection,
} from '../../gui/gm-station-puppet.js';
import { initConsole } from '../../gui/console-core.js';
import { createGmConfirmationProfile, createGmConfirmationController } from '../../gui/gm-confirmation.js';

function projection({ operators = [], activity = [], results = [], rating = 'Backfill' } = {}) {
  return {
    ships: [{
      ship_id: 'ship-player-1',
      name: 'Resolute',
      stations: [{
        station_id: 'captain',
        name: 'Captain',
        console: 'gui/captain-console.html',
        rating,
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
      station_ratings: { captain: rating },
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

  it('binds each Ship/Station to a fresh browsing context even when its URL matches', () => {
    const submitStationCommand = vi.fn(() => true);
    const listeners = new Map();
    const hostWindow = { addEventListener: (type, listener) => listeners.set(type, listener) };
    const controller = createGmStationPuppet({
      doc: document, win: hostWindow, getOperator: () => ({ id: 'gm-1' }), submitStationCommand,
    });
    const first = projection({ operators: ['gm-1'] });
    first.ships[0].ship_id = 'npc-a';
    first.ships[0].name = 'NPC A';
    controller.update(first);
    const oldFrame = document.getElementById('gm-station-frame');
    const oldWindow = oldFrame.contentWindow;
    const second = projection({ operators: ['gm-1'] });
    second.ships[0].ship_id = 'npc-b';
    second.ships[0].name = 'NPC B';
    controller.update(second);
    const liveFrame = document.getElementById('gm-station-frame');
    expect(liveFrame).not.toBe(oldFrame);
    expect(liveFrame.contentWindow === oldWindow).toBe(false);
    expect(liveFrame.dataset.ship).toBe('npc-b');
    const data = { type: 'console_action', payload: JSON.stringify({ action: 'set_red_alert', console: 'captain', active: true, correlation: 'npc-frame-command' }) };
    listeners.get('message')({ source: oldWindow, data });
    expect(submitStationCommand).not.toHaveBeenCalled();
    listeners.get('message')({ source: liveFrame.contentWindow, data });
    expect(submitStationCommand).toHaveBeenCalledWith(expect.objectContaining({ ship: 'npc-b' }));
  });

  // Issue #1430: the puppeted document is not "whoever is sitting at it" in
  // the sense `gui/accessibility-profile.js` and `gui/viewscreen-presentation.js`
  // both reserve for a private operator's own seat or the shared Viewscreen —
  // the GM reading THROUGH this frame is the one sitting at it. Without this,
  // the already-corrected Station family contents (#1423-1426) would sit at
  // their own 100% default no matter how far the desk's own text is turned up.
  it('mirrors this endpoint\'s own presentation onto the puppeted document, and again on every later tick', () => {
    const effects = { textScale: 2, contrast: true, reducedMotion: false, shake: 1, flash: 1, decorativeMotion: 1 };
    // `gui/console-state.js` sets the REAL `window.buildConsoleState` as an
    // import-time side effect this same test file relies on later; stand a
    // stub in for this test only and restore the original rather than
    // deleting it, so a later test does not lose it for the rest of the run.
    const previousBuildConsoleState = window.buildConsoleState;
    window.__serverSettings = { presentation: { effects: () => effects } };
    window.buildConsoleState = () => '{}';
    try {
      const controller = createGmStationPuppet({ doc: document, win: window, getOperator: () => ({ id: 'gm-1' }) });
      // First tick: a fresh browsing context, its 'load' has not (and in this
      // harness never will) fire, so nothing has been pushed into it yet.
      controller.update(projection({ operators: ['gm-1'] }));
      const frame = document.getElementById('gm-station-frame');
      // jsdom never actually completes an iframe navigation (no fetch, no
      // 'load'), so `contentDocument` stays a bare, root-less Document under
      // this harness even though `contentWindow` is a stable proxy — a real
      // browser's loaded document always has one. Stand in a real, detached
      // `<html>` element as that root so the assertions below exercise the
      // SAME `applyViewscreenEffectsToRoot(idoc.documentElement, …)` call
      // production code makes, without depending on jsdom's navigation model.
      const puppetRoot = document.createElement('html');
      Object.defineProperty(frame, 'contentDocument', {
        configurable: true, get: () => ({ documentElement: puppetRoot }),
      });
      frame.contentWindow.__updateConsole = vi.fn();
      // Second tick: the SAME Ship/Station selection, exactly like a live
      // `gm_station` projection arriving again — the ordinary path, not the
      // unfired load event, is what actually reaches an open puppet.
      controller.update(projection({ operators: ['gm-1'] }));
      expect(frame.contentWindow.__updateConsole).toHaveBeenCalled();
      expect(puppetRoot.style.getPropertyValue('--a11y-text-scale')).toBe('2');
      expect(puppetRoot.getAttribute('data-contrast')).toBe('more');
      // A later change on the endpoint's own Display tab reaches the ALREADY
      // open puppet on the next ordinary tick — no reload, no second control.
      effects.textScale = 1;
      effects.contrast = false;
      controller.update(projection({ operators: ['gm-1'] }));
      expect(puppetRoot.style.getPropertyValue('--a11y-text-scale')).toBe('1');
      expect(puppetRoot.getAttribute('data-contrast')).toBe('standard');
    } finally {
      delete window.__serverSettings;
      window.buildConsoleState = previousBuildConsoleState;
    }
  });

  it('never throws pushing state when no endpoint presentation is mounted yet', () => {
    const previousBuildConsoleState = window.buildConsoleState;
    window.buildConsoleState = () => '{}';
    try {
      const controller = createGmStationPuppet({ doc: document, win: window, getOperator: () => ({ id: 'gm-1' }) });
      expect(() => controller.update(projection({ operators: ['gm-1'] }))).not.toThrow();
      const frame = document.getElementById('gm-station-frame');
      frame.contentWindow.__updateConsole = vi.fn();
      expect(() => controller.update(projection({ operators: ['gm-1'] }))).not.toThrow();
      expect(frame.contentWindow.__updateConsole).toHaveBeenCalled();
    } finally {
      window.buildConsoleState = previousBuildConsoleState;
    }
  });

  it('retires disappeared-target feedback and unloads the final removed interface', () => {
    const controller = createGmStationPuppet({
      doc: document, win: window, getOperator: () => ({ id: 'gm-1' }),
      submitStationCommand: () => true,
    });
    controller.update(projection({ operators: ['gm-1'] }));
    controller.issueConsoleAction({ action: 'set_red_alert', console: 'captain', active: true, correlation: 'old-mount' });
    expect(controller.state().pendingCommands.size).toBe(1);
    const oldWindow = document.getElementById('gm-station-frame').contentWindow;
    controller.update({ ships: [], activity: [], results: [] });
    expect(controller.state().selectedRow).toBeNull();
    expect(controller.state().pendingCommands.size).toBe(0);
    expect(document.getElementById('gm-station-frame').getAttribute('src')).toBeNull();
    expect(document.getElementById('gm-station-frame').contentWindow === oldWindow).toBe(false);
    controller.update(projection({ operators: ['gm-1'] }));
    const feedback = vi.fn(() => true);
    document.getElementById('gm-station-frame').contentWindow.__updateActionFeedback = feedback;
    expect(controller.settleCommandResults([{
      action_kind: 'station-command', operator_id: 'gm-1', correlation: 'old-mount', outcome: 'applied',
    }])).toBe(0);
    expect(feedback).not.toHaveBeenCalled();
  });

  it.each([
    ['set_helm_thrust', 'value', 0.75, 0, 'SetThrust', 'value'],
    ['set_helm_steering', 'value', -0.5, 0, 'SetSteering', 'value'],
    ['set_helm_lateral', 'value', 0.75, 0, 'LateralThrustInput', 'lateral'],
    ['set_lateral_thrust', 'lateral', -0.5, 0, 'LateralThrustInput', 'lateral'],
    ['set_boost', 'active', true, false, 'SetBoost', 'active'],
  ])('releases %s through the real action map without reviving held input behind a modal',
    (action, field, held, neutral, type, wireField) => {
      const values = new Map();
      const profile = createGmConfirmationProfile({ storage: {
        getItem: (key) => values.get(key) ?? null,
        setItem: (key, value) => values.set(key, value),
      } });
      profile.setMode('station.command', 'confirm');
      const confirmation = createGmConfirmationController({ doc: document, profile });
      const submitStationCommand = vi.fn(() => true);
      const controller = createGmStationPuppet({ doc: document, win: window,
        getOperator: () => ({ id: 'gm-1' }), submitStationCommand,
        confirmAction: confirmation.request,
      });
      controller.update(projection({ operators: ['gm-1'] }));
      const issue = (value) => controller.issueConsoleAction({ action, console: 'helm', [field]: value });
      const payload = (value) => expect.objectContaining({ payload: { type, data: { [wireField]: value } } });

      expect(issue(held)).toBe(true);
      expect(confirmation.isOpen()).toBe(true);
      expect(submitStationCommand).not.toHaveBeenCalled();
      expect(issue(neutral)).toBe(true);
      expect(confirmation.isOpen()).toBe(false);
      document.querySelector('[data-confirmation-accept]').click();
      expect(submitStationCommand).toHaveBeenCalledExactlyOnceWith(payload(neutral));

      issue(held);
      document.querySelector('[data-confirmation-accept]').click();
      expect(submitStationCommand).toHaveBeenLastCalledWith(payload(held));
      const damage = vi.fn();
      confirmation.request({ category: 'effect.damage', description: 'Damage', accept: damage });
      expect(issue(neutral)).toBe(true);
      expect(submitStationCommand).toHaveBeenLastCalledWith(payload(neutral));
      expect(confirmation.isOpen()).toBe(true);
      expect(damage).not.toHaveBeenCalled();
      document.querySelector('[data-confirmation-accept]').click();
      expect(damage).toHaveBeenCalledOnce();
      confirmation.destroy();
    });

  it('confirms a Station command privately and refuses cancellation or a retired selection without sending', () => {
    const values = new Map();
    const profile = createGmConfirmationProfile({ storage: {
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => values.set(key, value),
    } });
    profile.setMode('station.command', 'confirm');
    const confirmation = createGmConfirmationController({ doc: document, profile });
    const submitStationCommand = vi.fn(() => true);
    const feedback = vi.fn();
    const controller = createGmStationPuppet({ doc: document, win: window,
      getOperator: () => ({ id: 'gm-1' }), submitStationCommand,
      confirmAction: confirmation.request,
    });
    controller.update(projection({ operators: ['gm-1'] }));
    document.getElementById('gm-station-frame').contentWindow.__updateActionFeedback = feedback;
    const action = { action: 'set_red_alert', console: 'captain', active: true, correlation: 'private-confirm-1' };
    expect(controller.issueConsoleAction(action)).toBe(true);
    expect(controller.state().pendingCommands.size).toBe(0);
    expect(submitStationCommand).not.toHaveBeenCalled();
    document.querySelector('[data-confirmation-cancel]').click();
    expect(feedback).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      correlation: 'private-confirm-1', state: 'Refused',
    }));
    controller.issueConsoleAction({ ...action, correlation: 'private-confirm-2' });
    controller.update({ ships: [], activity: [], results: [] });
    document.querySelector('[data-confirmation-accept]').click();
    expect(submitStationCommand).not.toHaveBeenCalled();
    expect(controller.state().pendingCommands.size).toBe(0);
    confirmation.destroy();
  });

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

  it('offers a human-held Station to two equal operators and retains the live rating on release', () => {
    const submitStationPuppet = vi.fn(() => true);
    const controller = createGmStationPuppet({
      doc: document, win: window, t: id => id,
      getOperator: () => ({ id: 'gm-2' }), submitStationPuppet,
      correlation: kind => 'human-' + kind,
    });
    const button = document.getElementById('gm-station-toggle');
    controller.update(projection({ rating: 'Manual', operators: ['gm-1'] }));
    expect(button.disabled).toBe(false);
    button.click();
    expect(submitStationPuppet).toHaveBeenLastCalledWith(expect.objectContaining({ active: true }));
    controller.update(projection({ rating: 'Manual', operators: ['gm-1', 'gm-2'] }));
    expect(controller.state().selectedRow.station.rating).toBe('Manual');
    expect(button.dataset.active).toBe('true');
    button.click();
    expect(submitStationPuppet).toHaveBeenLastCalledWith(expect.objectContaining({ active: false }));
    controller.update(projection({ rating: 'Manual', operators: ['gm-1'] }));
    expect(button.disabled).toBe(false);
    expect(button.dataset.active).toBe('false');
    expect(document.getElementById('gm-station-status').textContent)
      .toBe('server.gm.station.operators');
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
