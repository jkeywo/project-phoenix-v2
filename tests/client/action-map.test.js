import { describe, it, expect, vi } from 'vitest';
import { ACTION_MAP, dispatchConsoleAction } from '../../gui/action-map.js';

// ── ACTION_MAP structure ──────────────────────────────────────────────────────

describe('ACTION_MAP', () => {
  it('is frozen', () => {
    expect(Object.isFrozen(ACTION_MAP)).toBe(true);
  });

  it('contains exactly the 41 expected action keys', () => {
    expect(Object.keys(ACTION_MAP).sort()).toEqual([
      'abort_operation',
      'cancel_impulse',
      'charge_blaster_cancel',
      'charge_blaster_start',
      'clear_comms',
      'clear_navigation_waypoint',
      'dispatch_repair_team',
      'fire_blaster',
      'fire_phaser',
      'fire_torpedo',
      'hail',
      'helm_input',
      'load_tube',
      'respond_to_message',
      'return_to_lobby',
      'select_comms_message',
      'select_player_ship',
      'select_scenario',
      'set_boost',
      'set_helm',
      'set_lateral_thrust',
      'set_navigation_chart',
      'set_navigation_waypoint',
      'set_objective_priority',
      'set_phaser_frequency',
      'set_phaser_mode',
      'set_power',
      'set_radar_view',
      'set_red_alert',
      'set_repair_priority',
      'set_repair_target_priority',
      'set_sensors_target',
      'set_shield_focus',
      'set_target',
      'set_torpedo_volley_target',
      'set_view',
      'show_on_screen',
      'start_impulse_charge',
      'start_operation',
      'toggle_boost',
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
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'phaser-1',
      payload: { type: 'FirePhaser' },
    });
  });

  it('does nothing when bank is absent', () => {
    const send = mkSend();
    ACTION_MAP.fire_phaser({ action: 'fire_phaser' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('fire_blaster', () => {
  it('sends ControlSystem FireBlaster with blaster-bank target when bank provided', () => {
    const send = mkSend();
    ACTION_MAP.fire_blaster({ action: 'fire_blaster', bank: 'fore' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'blaster-fore',
      payload: { type: 'FireBlaster' },
    });
  });

  it('does nothing when bank is absent', () => {
    const send = mkSend();
    ACTION_MAP.fire_blaster({ action: 'fire_blaster' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('charge_blaster_start', () => {
  it('sends ControlSystem ChargeBlasterStart with blaster-bank target when bank provided', () => {
    const send = mkSend();
    ACTION_MAP.charge_blaster_start({ action: 'charge_blaster_start', bank: 'heavy' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'blaster-heavy',
      payload: { type: 'ChargeBlasterStart' },
    });
  });

  it('does nothing when bank is absent', () => {
    const send = mkSend();
    ACTION_MAP.charge_blaster_start({ action: 'charge_blaster_start' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('charge_blaster_cancel', () => {
  it('sends ControlSystem ChargeBlasterCancel with blaster-bank target when bank provided', () => {
    const send = mkSend();
    ACTION_MAP.charge_blaster_cancel({ action: 'charge_blaster_cancel', bank: 'heavy' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'blaster-heavy',
      payload: { type: 'ChargeBlasterCancel' },
    });
  });

  it('does nothing when bank is absent', () => {
    const send = mkSend();
    ACTION_MAP.charge_blaster_cancel({ action: 'charge_blaster_cancel' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('fire_torpedo', () => {
  it('calls send FireTorpedo with tube and target_uuid', () => {
    const send = mkSend();
    ACTION_MAP.fire_torpedo({ action: 'fire_torpedo', tube: 'port', target_uuid: 'u1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'torpedo-tube-port',
      payload: { type: 'FireTorpedo', data: { target_uuid: 'u1' } },
    });
  });

  it('defaults tube to fore and target_uuid to null', () => {
    const send = mkSend();
    ACTION_MAP.fire_torpedo({ action: 'fire_torpedo' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'torpedo-tube-fore',
      payload: { type: 'FireTorpedo', data: { target_uuid: null } },
    });
  });
});

describe('load_tube', () => {
  it('calls send LoadTube with tube', () => {
    const send = mkSend();
    ACTION_MAP.load_tube({ action: 'load_tube', tube: 'fore_port' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'torpedo-tube-fore-port',
      payload: { type: 'LoadTube' },
    });
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
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'torpedo-tube-aft',
      payload: { type: 'UnloadTube' },
    });
  });

  it('does nothing when tube is absent', () => {
    const send = mkSend();
    ACTION_MAP.unload_tube({ action: 'unload_tube' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_target', () => {
  it('calls mutate with weaponsTarget and send SetTarget with uuid', () => {
    const send = mkSend();
    const mutate = mkMutate();
    ACTION_MAP.set_target({ action: 'set_target', uuid: 'abc' }, send, mutate);
    expect(mutate).toHaveBeenCalledWith({ weaponsTarget: 'abc' });
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'tactical-radar',
      payload: { type: 'SetTarget', data: { uuid: 'abc' } },
    });
  });

  it('does nothing when uuid is absent', () => {
    const send = mkSend();
    const mutate = mkMutate();
    ACTION_MAP.set_target({ action: 'set_target' }, send, mutate);
    expect(send).not.toHaveBeenCalled();
    expect(mutate).not.toHaveBeenCalled();
  });
});

describe('set_phaser_mode', () => {
  it('calls send SetPhaserMode with mode', () => {
    const send = mkSend();
    ACTION_MAP.set_phaser_mode({ action: 'set_phaser_mode', mode: 'Manual' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'phaser-control',
      payload: { type: 'SetPhaserMode', data: { mode: 'Manual' } },
    });
  });

  it('does nothing when mode is absent', () => {
    const send = mkSend();
    ACTION_MAP.set_phaser_mode({ action: 'set_phaser_mode' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_view', () => {
  it('calls send ControlSystem viewscreen SetView with Camera kind and direction', () => {
    const send = mkSend();
    ACTION_MAP.set_view({ action: 'set_view', direction: 'Aft' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'viewscreen',
      payload: { type: 'SetView', data: { mode: { kind: 'Camera', data: 'Aft' } } },
    });
  });

  it('does nothing when direction is absent', () => {
    const send = mkSend();
    ACTION_MAP.set_view({ action: 'set_view' }, send);
    expect(send).not.toHaveBeenCalled();
  });

  it('sends non-camera view modes by kind', () => {
    const send = mkSend();
    ACTION_MAP.set_view({ action: 'set_view', direction: 'SensorsRadar' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'viewscreen',
      payload: { type: 'SetView', data: { mode: { kind: 'SensorsRadar' } } },
    });
  });
});

describe('set_red_alert', () => {
  it('sends ControlSystem with the explicit desired active=true state', () => {
    const send = mkSend();
    ACTION_MAP.set_red_alert({ action: 'set_red_alert', active: true }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'red-alert',
      payload: { type: 'SetRedAlert', data: { active: true } },
    });
    expect(send).toHaveBeenCalledTimes(1);
  });

  it('sends the explicit desired active=false state', () => {
    const send = mkSend();
    ACTION_MAP.set_red_alert({ action: 'set_red_alert', active: false }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'red-alert',
      payload: { type: 'SetRedAlert', data: { active: false } },
    });
  });

  it('coerces a missing active flag to false (never inverts)', () => {
    const send = mkSend();
    ACTION_MAP.set_red_alert({ action: 'set_red_alert' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'red-alert',
      payload: { type: 'SetRedAlert', data: { active: false } },
    });
  });
});

describe('helm_input', () => {
  // Issue #801: one joystick action fans out to the two per-axis payloads,
  // so admission gates each axis on its own declared system.
  it('sends SetThrust to helm-thrust and SetSteering to helm-steering', () => {
    const send = mkSend();
    ACTION_MAP.helm_input({ action: 'helm_input', thrust: 0.5, steering: -0.3 }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-thrust',
      payload: { type: 'SetThrust', data: { value: 0.5 } },
    });
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-steering',
      payload: { type: 'SetSteering', data: { value: -0.3 } },
    });
    expect(send).toHaveBeenCalledTimes(2);
  });

  it('defaults thrust and steering to 0', () => {
    const send = mkSend();
    ACTION_MAP.helm_input({ action: 'helm_input' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-thrust',
      payload: { type: 'SetThrust', data: { value: 0 } },
    });
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-steering',
      payload: { type: 'SetSteering', data: { value: 0 } },
    });
  });
});

describe('set_helm', () => {
  it('maps joystick yaw onto the steering axis', () => {
    const send = mkSend();
    ACTION_MAP.set_helm({ action: 'set_helm', thrust: 0.8, yaw: 0.2 }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-thrust',
      payload: { type: 'SetThrust', data: { value: 0.8 } },
    });
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-steering',
      payload: { type: 'SetSteering', data: { value: 0.2 } },
    });
  });
});

describe('start_impulse_charge', () => {
  it('calls send StartImpulseCharge', () => {
    const send = mkSend();
    ACTION_MAP.start_impulse_charge({}, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-impulse',
      payload: { type: 'StartImpulseCharge' },
    });
  });
});

describe('toggle_boost', () => {
  it('calls send ToggleBoost', () => {
    const send = mkSend();
    ACTION_MAP.toggle_boost({ action: 'toggle_boost' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-boost',
      payload: { type: 'ToggleBoost' },
    });
  });
});

describe('set_boost', () => {
  it('sends SetBoost with active true', () => {
    const send = mkSend();
    ACTION_MAP.set_boost({ active: true }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-boost',
      payload: { type: 'SetBoost', data: { active: true } },
    });
  });
  it('sends SetBoost with active false', () => {
    const send = mkSend();
    ACTION_MAP.set_boost({ active: false }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-boost',
      payload: { type: 'SetBoost', data: { active: false } },
    });
  });
});

describe('cancel_impulse', () => {
  it('calls send CancelImpulse', () => {
    const send = mkSend();
    ACTION_MAP.cancel_impulse({}, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-impulse',
      payload: { type: 'CancelImpulse' },
    });
  });
});

describe('set_radar_view', () => {
  it('calls send ControlSystem SetView with Radar kind', () => {
    const send = mkSend();
    ACTION_MAP.set_radar_view({}, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'viewscreen',
      payload: { type: 'SetView', data: { mode: { kind: 'Radar' } } },
    });
  });
});

describe('dispatch_repair_team', () => {
  it('sends ControlSystem envelope for a Station target (post issue #618)', () => {
    const send = mkSend();
    ACTION_MAP.dispatch_repair_team(
      { action: 'dispatch_repair_team', team_idx: 0, target: 'helm' },
      send,
    );
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'repair',
      payload: {
        type: 'DispatchRepairTeam',
        data: { team_idx: 0, target: { type: 'Station', data: 'helm' } },
      },
    });
  });

  it('sends ControlSystem envelope for the Core bucket', () => {
    const send = mkSend();
    ACTION_MAP.dispatch_repair_team(
      { action: 'dispatch_repair_team', team_idx: 1, target: 'core' },
      send,
    );
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'repair',
      payload: {
        type: 'DispatchRepairTeam',
        data: { team_idx: 1, target: { type: 'Core' } },
      },
    });
  });
});

describe('set_repair_priority', () => {
  it('sends ControlSystem SetRepairPriority with team_idx and priority', () => {
    const send = mkSend();
    ACTION_MAP.set_repair_priority(
      { action: 'set_repair_priority', team_idx: 1, priority: 2 },
      send,
    );
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'repair',
      payload: {
        type: 'SetRepairPriority',
        data: { team_idx: 1, priority: 2 },
      },
    });
  });

  it('does nothing when team_idx is missing', () => {
    const send = mkSend();
    ACTION_MAP.set_repair_priority(
      { action: 'set_repair_priority', priority: 2 },
      send,
    );
    expect(send).not.toHaveBeenCalled();
  });

  it('does nothing when priority is missing', () => {
    const send = mkSend();
    ACTION_MAP.set_repair_priority(
      { action: 'set_repair_priority', team_idx: 0 },
      send,
    );
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_repair_target_priority', () => {
  it('sends ControlSystem SetRepairTargetPriority with only the system id', () => {
    const send = mkSend();
    ACTION_MAP.set_repair_target_priority(
      { action: 'set_repair_target_priority', system_id: 'hull-plating' },
      send,
    );
    // No team_idx and no ordinal: the host resolves which team and pins the
    // system; the ordinal is untouched. See gui/repair-dispatch.js for why
    // the console cannot compute it.
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'repair',
      payload: {
        type: 'SetRepairTargetPriority',
        data: { system_id: 'hull-plating' },
      },
    });
  });

  it('does nothing when system_id is missing', () => {
    const send = mkSend();
    ACTION_MAP.set_repair_target_priority({ action: 'set_repair_target_priority' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_power', () => {
  it('calls send ControlSystem SetPowerGroupAllocation with group and level', () => {
    const send = mkSend();
    ACTION_MAP.set_power({ action: 'set_power', target: 'helm', level: 3 }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'power-reactor',
      payload: { type: 'SetPowerGroupAllocation', data: { group: 'helm', level: 3 } },
    });
  });

  it('does nothing when target is absent', () => {
    const send = mkSend();
    ACTION_MAP.set_power({ action: 'set_power', level: 3 }, send);
    expect(send).not.toHaveBeenCalled();
  });

  it('does nothing when level is absent', () => {
    const send = mkSend();
    ACTION_MAP.set_power({ action: 'set_power', target: 'Helm' }, send);
    expect(send).not.toHaveBeenCalled();
  });

  it('sends level 1 to decrease power', () => {
    const send = mkSend();
    ACTION_MAP.set_power({ action: 'set_power', target: 'weapons', level: 1 }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'power-reactor',
      payload: { type: 'SetPowerGroupAllocation', data: { group: 'weapons', level: 1 } },
    });
  });
});

describe('set_shield_focus', () => {
  it('sends SetShieldArcFocus targeted at shield-arc-<arc_id> (issue #514)', () => {
    const send = mkSend();
    ACTION_MAP.set_shield_focus({ action: 'set_shield_focus', arc_id: 'fore', focused: true }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'shield-arc-fore',
      payload: { type: 'SetShieldArcFocus', data: { focused: true } },
    });
  });

  it('defaults focused to true when the field is omitted', () => {
    const send = mkSend();
    ACTION_MAP.set_shield_focus({ action: 'set_shield_focus', arc_id: 'starboard' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'shield-arc-starboard',
      payload: { type: 'SetShieldArcFocus', data: { focused: true } },
    });
  });

  it('sends focused=false to clear focus on the currently-focused arc', () => {
    const send = mkSend();
    ACTION_MAP.set_shield_focus({ action: 'set_shield_focus', arc_id: 'aft', focused: false }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'shield-arc-aft',
      payload: { type: 'SetShieldArcFocus', data: { focused: false } },
    });
  });

  it('is a no-op when arc_id is missing', () => {
    const send = mkSend();
    ACTION_MAP.set_shield_focus({ action: 'set_shield_focus' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_sensors_target', () => {
  it('calls mutate with sensorsTarget and send ControlSystem SetScienceTarget', () => {
    const send = mkSend();
    const mutate = mkMutate();
    ACTION_MAP.set_sensors_target({ action: 'set_sensors_target', uuid: 'tgt-42' }, send, mutate);
    expect(mutate).toHaveBeenCalledWith({ sensorsTarget: 'tgt-42' });
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'sensors',
      payload: { type: 'SetScienceTarget', data: { uuid: 'tgt-42' } },
    });
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
  it('sends ControlSystem Hail targeting comms (issue #822)', () => {
    const send = mkSend();
    ACTION_MAP.hail({ action: 'hail', target_uuid: 'npc-1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'comms',
      payload: { type: 'Hail', data: { target_uuid: 'npc-1' } },
    });
  });

  it('does nothing when target_uuid is absent', () => {
    const send = mkSend();
    ACTION_MAP.hail({ action: 'hail' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('select_comms_message', () => {
  it('sends ControlSystem SelectCommsMessage targeting comms (issue #822)', () => {
    const send = mkSend();
    ACTION_MAP.select_comms_message({ action: 'select_comms_message', message_id: 'msg-42' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'comms',
      payload: { type: 'SelectCommsMessage', data: { message_id: 'msg-42' } },
    });
  });

  it('does nothing when message_id is absent', () => {
    const send = mkSend();
    ACTION_MAP.select_comms_message({ action: 'select_comms_message' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('respond_to_message', () => {
  it('sends ControlSystem RespondToMessage targeting comms (issue #822)', () => {
    const send = mkSend();
    ACTION_MAP.respond_to_message({ action: 'respond_to_message', message_id: 'msg-1', response_index: 2 }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'comms',
      payload: { type: 'RespondToMessage', data: { message_id: 'msg-1', response_index: 2 } },
    });
  });

  it('does nothing when message_id is absent', () => {
    const send = mkSend();
    ACTION_MAP.respond_to_message({ action: 'respond_to_message', response_index: 0 }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('clear_comms', () => {
  it('sends ControlSystem ClearComms targeting comms (issue #822)', () => {
    const send = mkSend();
    ACTION_MAP.clear_comms({}, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'comms',
      payload: { type: 'ClearComms' },
    });
    expect(send).toHaveBeenCalledTimes(1);
  });
});

describe('show_on_screen', () => {
  it('sends ControlSystem ShowOnScreen targeting comms (issue #822)', () => {
    const send = mkSend();
    ACTION_MAP.show_on_screen({ action: 'show_on_screen', message_id: 'msg-7' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'comms',
      payload: { type: 'ShowOnScreen', data: { message_id: 'msg-7' } },
    });
  });

  it('does nothing when message_id is absent', () => {
    const send = mkSend();
    ACTION_MAP.show_on_screen({ action: 'show_on_screen' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_navigation_chart', () => {
  it('calls send ControlSystem viewscreen SetView with NavigationChart kind', () => {
    const send = mkSend();
    ACTION_MAP.set_navigation_chart({ action: 'set_navigation_chart' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'viewscreen',
      payload: { type: 'SetView', data: { mode: { kind: 'NavigationChart' } } },
    });
  });
});

describe('set_navigation_waypoint', () => {
  it('sends ControlSystem SetNavigationWaypoint targeting navigation (issue #822)', () => {
    const send = mkSend();
    ACTION_MAP.set_navigation_waypoint({ action: 'set_navigation_waypoint', x: 12.5, z: -8 }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'navigation',
      payload: { type: 'SetNavigationWaypoint', data: { x: 12.5, z: -8 } },
    });
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
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'navigation',
      payload: {
        type: 'SetNavigationWaypoint',
        data: { x: 50, z: -100, source_uuid: 'station-alpha' },
      },
    });
  });

  it('omits source_uuid when empty string (treated as free waypoint)', () => {
    const send = mkSend();
    ACTION_MAP.set_navigation_waypoint(
      { action: 'set_navigation_waypoint', x: 1, z: 2, source_uuid: '' },
      send,
    );
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'navigation',
      payload: { type: 'SetNavigationWaypoint', data: { x: 1, z: 2 } },
    });
  });

  it('omits source_uuid when null (legacy free-waypoint path)', () => {
    const send = mkSend();
    ACTION_MAP.set_navigation_waypoint(
      { action: 'set_navigation_waypoint', x: 3, z: 4, source_uuid: null },
      send,
    );
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'navigation',
      payload: { type: 'SetNavigationWaypoint', data: { x: 3, z: 4 } },
    });
  });
});

describe('clear_navigation_waypoint', () => {
  it('sends ControlSystem ClearNavigationWaypoint targeting navigation (issue #822)', () => {
    const send = mkSend();
    ACTION_MAP.clear_navigation_waypoint({}, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'navigation',
      payload: { type: 'ClearNavigationWaypoint' },
    });
  });
});

// ── return_to_lobby (issue #822 / #756) ───────────────────────────────────────
// Host-page lobby actions route through the same action map as everything else;
// each maps to its bare ClientMessage variant.

describe('return_to_lobby', () => {
  it('sends the bare ReturnToLobby client message', () => {
    const send = mkSend();
    ACTION_MAP.return_to_lobby({ action: 'return_to_lobby' }, send);
    expect(send).toHaveBeenCalledWith('ReturnToLobby');
    expect(send).toHaveBeenCalledTimes(1);
  });
});

// ── select_scenario / select_player_ship (issue #755) ─────────────────────────
// QR-first pre-scenario selection: both the host page and phones emit these via
// the same action map (two transports), fed to the host-runtime arbiter.

describe('select_scenario', () => {
  it('sends SelectScenario with the scenario id', () => {
    const send = mkSend();
    ACTION_MAP.select_scenario({ action: 'select_scenario', scenario_id: 'combat_test' }, send);
    expect(send).toHaveBeenCalledWith('SelectScenario', { scenario_id: 'combat_test' });
    expect(send).toHaveBeenCalledTimes(1);
  });
  it('ignores a request with no scenario id', () => {
    const send = mkSend();
    ACTION_MAP.select_scenario({ action: 'select_scenario' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('select_player_ship', () => {
  it('sends SelectPlayerShip with the template path', () => {
    const send = mkSend();
    ACTION_MAP.select_player_ship(
      { action: 'select_player_ship', template_path: 'assets/entities/alliance_cruiser.toml' },
      send,
    );
    expect(send).toHaveBeenCalledWith('SelectPlayerShip', {
      template_path: 'assets/entities/alliance_cruiser.toml',
    });
    expect(send).toHaveBeenCalledTimes(1);
  });
  it('ignores a request with no template path', () => {
    const send = mkSend();
    ACTION_MAP.select_player_ship({ action: 'select_player_ship' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

// ── External operations (issue #1026) ─────────────────────────────────────────

describe('start_operation / abort_operation', () => {
  it('sends StartOperation at the captain system with the verb and target', () => {
    // The captain system, not an operations one: an operation is something the
    // ship does, so it rides the captain's ordinary station-tenure admission.
    const send = mkSend();
    ACTION_MAP.start_operation(
      { action: 'start_operation', verb: 'stabilise', target_uuid: 'depot-1' },
      send,
    );
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'captain',
      payload: { type: 'StartOperation', data: { verb: 'stabilise', target_uuid: 'depot-1' } },
    });
  });

  it('sends nothing for a start with no verb or no target', () => {
    for (const action of [
      { action: 'start_operation', target_uuid: 'depot-1' },
      { action: 'start_operation', verb: 'stabilise' },
    ]) {
      const send = mkSend();
      ACTION_MAP.start_operation(action, send);
      expect(send).not.toHaveBeenCalled();
    }
  });

  it('sends AbortOperation with no payload — a ship runs at most one', () => {
    const send = mkSend();
    ACTION_MAP.abort_operation({ action: 'abort_operation' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'captain',
      payload: { type: 'AbortOperation' },
    });
  });
});

// ── set_objective_priority (issue #675) ───────────────────────────────────────

describe('set_objective_priority', () => {
  it('sends ControlSystem SetObjectivePriority with id', () => {
    const send = mkSend();
    ACTION_MAP.set_objective_priority({ action: 'set_objective_priority', id: 'obj-1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'captain',
      payload: { type: 'SetObjectivePriority', data: { id: 'obj-1' } },
    });
  });

  it('does nothing when id is absent', () => {
    const send = mkSend();
    ACTION_MAP.set_objective_priority({ action: 'set_objective_priority' }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

// ── dispatchConsoleAction ─────────────────────────────────────────────────────

describe('dispatchConsoleAction', () => {
  it('routes a known action to its handler', () => {
    const send = mkSend();
    dispatchConsoleAction({ action: 'set_red_alert', active: true }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'red-alert',
      payload: { type: 'SetRedAlert', data: { active: true } },
    });
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
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'sensors',
      payload: { type: 'SetScienceTarget', data: { uuid: 'x' } },
    });
  });
});

// ── set_torpedo_volley_target (issue #632) ────────────────────────────────────

describe('set_torpedo_volley_target', () => {
  it('sends ControlSystem SetTorpedoVolleyTarget to torpedo-tube-<id> system', () => {
    const send = mkSend();
    ACTION_MAP.set_torpedo_volley_target({ tube: 'fore_port', count: 3 }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'torpedo-tube-fore-port',
      payload: { type: 'SetTorpedoVolleyTarget', data: { count: 3 } },
    });
  });

  it('converts underscores to hyphens in tube id', () => {
    const send = mkSend();
    ACTION_MAP.set_torpedo_volley_target({ tube: 'fore_starboard', count: 1 }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'torpedo-tube-fore-starboard',
      payload: { type: 'SetTorpedoVolleyTarget', data: { count: 1 } },
    });
  });

  it('does nothing when tube is null', () => {
    const send = mkSend();
    ACTION_MAP.set_torpedo_volley_target({ tube: null, count: 1 }, send);
    expect(send).not.toHaveBeenCalled();
  });

  it('does nothing when count is null', () => {
    const send = mkSend();
    ACTION_MAP.set_torpedo_volley_target({ tube: 'fore_port', count: null }, send);
    expect(send).not.toHaveBeenCalled();
  });
});
