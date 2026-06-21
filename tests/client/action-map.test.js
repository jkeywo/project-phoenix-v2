import { describe, it, expect, vi } from 'vitest';
import { ACTION_MAP, dispatchConsoleAction } from '../../gui/action-map.js';

// ── ACTION_MAP structure ──────────────────────────────────────────────────────

describe('ACTION_MAP', () => {
  it('is frozen', () => {
    expect(Object.isFrozen(ACTION_MAP)).toBe(true);
  });

  it('contains exactly the 27 expected action keys', () => {
    expect(Object.keys(ACTION_MAP).sort()).toEqual([
      'cancel_impulse',
      'clear_comms',
      'clear_navigation_waypoint',
      'decrease_power',
      'dispatch_repair_team',
      'fire_phaser',
      'fire_torpedo',
      'hail',
      'helm_input',
      'increase_power',
      'load_tube',
      'respond_to_message',
      'select_comms_message',
      'set_boost',
      'set_navigation_chart',
      'set_navigation_waypoint',
      'set_phaser_mode',
      'set_radar_view',
      'set_sensors_target',
      'set_shield_focus',
      'set_target',
      'set_view',
      'show_on_screen',
      'start_impulse_charge',
      'toggle_boost',
      'toggle_red_alert',
      'unload_tube',
    ]);
  });
});

// ── Per-action handler tests ──────────────────────────────────────────────────

function mkSend() { return vi.fn(); }
function mkMutate() { return vi.fn(); }

describe('fire_phaser', () => {
  it('calls send FirePhaser with bank when bank is provided', () => {
    const send = mkSend();
    ACTION_MAP.fire_phaser({ action: 'fire_phaser', bank: 1 }, send);
    expect(send).toHaveBeenCalledWith('FirePhaser', { bank: 1 });
  });

  it('does nothing when bank is absent', () => {
    const send = mkSend();
    ACTION_MAP.fire_phaser({ action: 'fire_phaser' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('fire_torpedo', () => {
  it('calls send FireTorpedo with tube and target_uuid', () => {
    const send = mkSend();
    ACTION_MAP.fire_torpedo({ action: 'fire_torpedo', tube: 'port', target_uuid: 'u1' }, send);
    expect(send).toHaveBeenCalledWith('FireTorpedo', { tube: 'port', target_uuid: 'u1' });
  });

  it('defaults tube to fore and target_uuid to null', () => {
    const send = mkSend();
    ACTION_MAP.fire_torpedo({ action: 'fire_torpedo' }, send);
    expect(send).toHaveBeenCalledWith('FireTorpedo', { tube: 'fore', target_uuid: null });
  });
});

describe('load_tube', () => {
  it('calls send LoadTube with tube', () => {
    const send = mkSend();
    ACTION_MAP.load_tube({ action: 'load_tube', tube: 'fore_port' }, send);
    expect(send).toHaveBeenCalledWith('LoadTube', { tube: 'fore_port' });
  });

  it('does nothing when tube is absent', () => {
    const send = mkSend();
    ACTION_MAP.load_tube({ action: 'load_tube' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('unload_tube', () => {
  it('calls send UnloadTube with tube', () => {
    const send = mkSend();
    ACTION_MAP.unload_tube({ action: 'unload_tube', tube: 'aft' }, send);
    expect(send).toHaveBeenCalledWith('UnloadTube', { tube: 'aft' });
  });

  it('does nothing when tube is absent', () => {
    const send = mkSend();
    ACTION_MAP.unload_tube({ action: 'unload_tube' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_target', () => {
  it('calls send SetTarget with uuid', () => {
    const send = mkSend();
    ACTION_MAP.set_target({ action: 'set_target', uuid: 'abc' }, send);
    expect(send).toHaveBeenCalledWith('SetTarget', { uuid: 'abc' });
  });

  it('does nothing when uuid is absent', () => {
    const send = mkSend();
    ACTION_MAP.set_target({ action: 'set_target' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_phaser_mode', () => {
  it('calls send SetPhaserMode with mode', () => {
    const send = mkSend();
    ACTION_MAP.set_phaser_mode({ action: 'set_phaser_mode', mode: 'Manual' }, send);
    expect(send).toHaveBeenCalledWith('SetPhaserMode', { mode: 'Manual' });
  });

  it('does nothing when mode is absent', () => {
    const send = mkSend();
    ACTION_MAP.set_phaser_mode({ action: 'set_phaser_mode' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_view', () => {
  it('calls send SetView with Camera kind and direction', () => {
    const send = mkSend();
    ACTION_MAP.set_view({ action: 'set_view', direction: 'Aft' }, send);
    expect(send).toHaveBeenCalledWith('SetView', { mode: { kind: 'Camera', data: 'Aft' } });
  });

  it('does nothing when direction is absent', () => {
    const send = mkSend();
    ACTION_MAP.set_view({ action: 'set_view' }, send);
    expect(send).not.toHaveBeenCalled();
  });

  it('sends non-camera view modes by kind', () => {
    const send = mkSend();
    ACTION_MAP.set_view({ action: 'set_view', direction: 'SensorsRadar' }, send);
    expect(send).toHaveBeenCalledWith('SetView', { mode: { kind: 'SensorsRadar' } });
  });
});

describe('toggle_red_alert', () => {
  it('calls send ToggleRedAlert with no data', () => {
    const send = mkSend();
    ACTION_MAP.toggle_red_alert({ action: 'toggle_red_alert' }, send);
    expect(send).toHaveBeenCalledWith('ToggleRedAlert');
    expect(send).toHaveBeenCalledTimes(1);
  });
});

describe('helm_input', () => {
  it('calls send HelmInput with thrust and steering', () => {
    const send = mkSend();
    ACTION_MAP.helm_input({ action: 'helm_input', thrust: 0.5, steering: -0.3 }, send);
    expect(send).toHaveBeenCalledWith('HelmInput', { thrust: 0.5, steering: -0.3 });
  });

  it('defaults thrust and steering to 0', () => {
    const send = mkSend();
    ACTION_MAP.helm_input({ action: 'helm_input' }, send);
    expect(send).toHaveBeenCalledWith('HelmInput', { thrust: 0, steering: 0 });
  });
});

describe('start_impulse_charge', () => {
  it('calls send StartImpulseCharge', () => {
    const send = mkSend();
    ACTION_MAP.start_impulse_charge({}, send);
    expect(send).toHaveBeenCalledWith('StartImpulseCharge');
  });
});

describe('toggle_boost', () => {
  it('calls send ToggleBoost', () => {
    const send = mkSend();
    ACTION_MAP.toggle_boost({ action: 'toggle_boost' }, send);
    expect(send).toHaveBeenCalledWith('ToggleBoost');
  });
});

describe('set_boost', () => {
  it('sends SetBoost with active true', () => {
    const send = mkSend();
    ACTION_MAP.set_boost({ active: true }, send);
    expect(send).toHaveBeenCalledWith('SetBoost', { active: true });
  });
  it('sends SetBoost with active false', () => {
    const send = mkSend();
    ACTION_MAP.set_boost({ active: false }, send);
    expect(send).toHaveBeenCalledWith('SetBoost', { active: false });
  });
});

describe('cancel_impulse', () => {
  it('calls send CancelImpulse', () => {
    const send = mkSend();
    ACTION_MAP.cancel_impulse({}, send);
    expect(send).toHaveBeenCalledWith('CancelImpulse');
  });
});

describe('set_radar_view', () => {
  it('calls send SetView with Radar kind', () => {
    const send = mkSend();
    ACTION_MAP.set_radar_view({}, send);
    expect(send).toHaveBeenCalledWith('SetView', { mode: { kind: 'Radar' } });
  });
});

describe('dispatch_repair_team', () => {
  it('calls send DispatchRepairTeam with team_idx and console', () => {
    const send = mkSend();
    ACTION_MAP.dispatch_repair_team({ action: 'dispatch_repair_team', team_idx: 0, target: 'Helm' }, send);
    expect(send).toHaveBeenCalledWith('DispatchRepairTeam', { team_idx: 0, console: 'Helm' });
  });
});

describe('increase_power', () => {
  it('calls send IncreasePower with console target', () => {
    const send = mkSend();
    ACTION_MAP.increase_power({ action: 'increase_power', target: 'Weapons' }, send);
    expect(send).toHaveBeenCalledWith('IncreasePower', { console: 'Weapons' });
  });

  it('does nothing when target is absent', () => {
    const send = mkSend();
    ACTION_MAP.increase_power({ action: 'increase_power' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('decrease_power', () => {
  it('calls send DecreasePower with console target', () => {
    const send = mkSend();
    ACTION_MAP.decrease_power({ action: 'decrease_power', target: 'Sensors' }, send);
    expect(send).toHaveBeenCalledWith('DecreasePower', { console: 'Sensors' });
  });

  it('does nothing when target is absent', () => {
    const send = mkSend();
    ACTION_MAP.decrease_power({ action: 'decrease_power' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_shield_focus', () => {
  it('calls send SetShieldFocus with facing', () => {
    const send = mkSend();
    ACTION_MAP.set_shield_focus({ action: 'set_shield_focus', facing: 'fore' }, send);
    expect(send).toHaveBeenCalledWith('SetShieldFocus', { facing: 'fore' });
  });

  it('defaults facing to null when absent', () => {
    const send = mkSend();
    ACTION_MAP.set_shield_focus({ action: 'set_shield_focus' }, send);
    expect(send).toHaveBeenCalledWith('SetShieldFocus', { facing: null });
  });
});

describe('set_sensors_target', () => {
  it('calls mutate with sensorsTarget and send SetScienceTarget', () => {
    const send = mkSend();
    const mutate = mkMutate();
    ACTION_MAP.set_sensors_target({ action: 'set_sensors_target', uuid: 'tgt-42' }, send, mutate);
    expect(mutate).toHaveBeenCalledWith({ sensorsTarget: 'tgt-42' });
    expect(send).toHaveBeenCalledWith('SetScienceTarget', { uuid: 'tgt-42' });
  });

  it('does nothing when uuid is absent', () => {
    const send = mkSend();
    const mutate = mkMutate();
    ACTION_MAP.set_sensors_target({ action: 'set_sensors_target' }, send, mutate);
    expect(send).not.toHaveBeenCalled();
    expect(mutate).not.toHaveBeenCalled();
  });
});

describe('hail', () => {
  it('calls send Hail with target_uuid', () => {
    const send = mkSend();
    ACTION_MAP.hail({ action: 'hail', target_uuid: 'npc-1' }, send);
    expect(send).toHaveBeenCalledWith('Hail', { target_uuid: 'npc-1' });
  });

  it('does nothing when target_uuid is absent', () => {
    const send = mkSend();
    ACTION_MAP.hail({ action: 'hail' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('select_comms_message', () => {
  it('calls send SelectCommsMessage with message_id', () => {
    const send = mkSend();
    ACTION_MAP.select_comms_message({ action: 'select_comms_message', message_id: 'msg-42' }, send);
    expect(send).toHaveBeenCalledWith('SelectCommsMessage', { message_id: 'msg-42' });
  });

  it('does nothing when message_id is absent', () => {
    const send = mkSend();
    ACTION_MAP.select_comms_message({ action: 'select_comms_message' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('respond_to_message', () => {
  it('calls send RespondToMessage with message_id and response_index', () => {
    const send = mkSend();
    ACTION_MAP.respond_to_message({ action: 'respond_to_message', message_id: 'msg-1', response_index: 2 }, send);
    expect(send).toHaveBeenCalledWith('RespondToMessage', { message_id: 'msg-1', response_index: 2 });
  });

  it('does nothing when message_id is absent', () => {
    const send = mkSend();
    ACTION_MAP.respond_to_message({ action: 'respond_to_message', response_index: 0 }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('clear_comms', () => {
  it('calls send ClearComms with no payload', () => {
    const send = mkSend();
    ACTION_MAP.clear_comms({}, send);
    expect(send).toHaveBeenCalledWith('ClearComms');
    expect(send).toHaveBeenCalledTimes(1);
  });
});

describe('show_on_screen', () => {
  it('calls send ShowOnScreen with message_id', () => {
    const send = mkSend();
    ACTION_MAP.show_on_screen({ action: 'show_on_screen', message_id: 'msg-7' }, send);
    expect(send).toHaveBeenCalledWith('ShowOnScreen', { message_id: 'msg-7' });
  });

  it('does nothing when message_id is absent', () => {
    const send = mkSend();
    ACTION_MAP.show_on_screen({ action: 'show_on_screen' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_navigation_chart', () => {
  it('calls send SetView with NavigationChart kind', () => {
    const send = mkSend();
    ACTION_MAP.set_navigation_chart({ action: 'set_navigation_chart' }, send);
    expect(send).toHaveBeenCalledWith('SetView', { mode: { kind: 'NavigationChart' } });
  });
});

describe('set_navigation_waypoint', () => {
  it('calls send SetNavigationWaypoint with coordinates', () => {
    const send = mkSend();
    ACTION_MAP.set_navigation_waypoint({ action: 'set_navigation_waypoint', x: 12.5, z: -8 }, send);
    expect(send).toHaveBeenCalledWith('SetNavigationWaypoint', { x: 12.5, z: -8 });
  });

  it('does nothing for invalid coordinates', () => {
    const send = mkSend();
    ACTION_MAP.set_navigation_waypoint({ action: 'set_navigation_waypoint', x: Number.NaN, z: -8 }, send);
    expect(send).not.toHaveBeenCalled();
  });

  it('forwards source_uuid when present and non-empty (entity-anchored waypoint)', () => {
    const send = mkSend();
    ACTION_MAP.set_navigation_waypoint(
      { action: 'set_navigation_waypoint', x: 50, z: -100, source_uuid: 'station-alpha' },
      send,
    );
    expect(send).toHaveBeenCalledWith('SetNavigationWaypoint', {
      x: 50,
      z: -100,
      source_uuid: 'station-alpha',
    });
  });

  it('omits source_uuid when empty string (treated as free waypoint)', () => {
    const send = mkSend();
    ACTION_MAP.set_navigation_waypoint(
      { action: 'set_navigation_waypoint', x: 1, z: 2, source_uuid: '' },
      send,
    );
    expect(send).toHaveBeenCalledWith('SetNavigationWaypoint', { x: 1, z: 2 });
  });

  it('omits source_uuid when null (legacy free-waypoint path)', () => {
    const send = mkSend();
    ACTION_MAP.set_navigation_waypoint(
      { action: 'set_navigation_waypoint', x: 3, z: 4, source_uuid: null },
      send,
    );
    expect(send).toHaveBeenCalledWith('SetNavigationWaypoint', { x: 3, z: 4 });
  });
});

describe('clear_navigation_waypoint', () => {
  it('calls send ClearNavigationWaypoint', () => {
    const send = mkSend();
    ACTION_MAP.clear_navigation_waypoint({}, send);
    expect(send).toHaveBeenCalledWith('ClearNavigationWaypoint');
  });
});

// ── dispatchConsoleAction ─────────────────────────────────────────────────────

describe('dispatchConsoleAction', () => {
  it('routes a known action to its handler', () => {
    const send = mkSend();
    dispatchConsoleAction({ action: 'toggle_red_alert' }, send);
    expect(send).toHaveBeenCalledWith('ToggleRedAlert');
  });

  it('ignores unknown actions without throwing', () => {
    const send = mkSend();
    expect(() => dispatchConsoleAction({ action: 'unknown_xyz' }, send)).not.toThrow();
    expect(send).not.toHaveBeenCalled();
  });

  it('ignores null action without throwing', () => {
    const send = mkSend();
    expect(() => dispatchConsoleAction(null, send)).not.toThrow();
  });

  it('ignores action with missing action field', () => {
    const send = mkSend();
    expect(() => dispatchConsoleAction({ data: 'no action key' }, send)).not.toThrow();
    expect(send).not.toHaveBeenCalled();
  });

  it('provides a no-op mutate when none is given', () => {
    const send = mkSend();
    // set_sensors_target needs mutate; should not throw even if not provided
    expect(() => dispatchConsoleAction({ action: 'set_sensors_target', uuid: 'x' }, send)).not.toThrow();
    expect(send).toHaveBeenCalledWith('SetScienceTarget', { uuid: 'x' });
  });
});
