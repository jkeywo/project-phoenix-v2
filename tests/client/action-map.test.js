import { describe, it, expect, vi } from 'vitest';
import { ACTION_MAP, dispatchConsoleAction } from '../../gui/action-map.js';

// ── ACTION_MAP structure ──────────────────────────────────────────────────────

describe('ACTION_MAP', () => {
  it('is frozen', () => {
    expect(Object.isFrozen(ACTION_MAP)).toBe(true);
  });

  it('contains exactly the 54 expected action keys', () => {
    expect(Object.keys(ACTION_MAP).sort()).toEqual([
      'cancel_impulse',
      'charge_blaster_cancel',
      'charge_blaster_start',
      'clear_comms',
      'clear_navigation_waypoint',
      'dispatch_external_repair',
      'dispatch_repair_team',
      'dispatch_security_team',
      'dock',
      'engage_tractor',
      'fire_blaster',
      'fire_phaser',
      'fire_torpedo',
      'hail',
      'helm_input',
      'load_tube',
      'order_civilian',
      'recall_external_repair',
      'recall_repair_team',
      'recall_security_team',
      'release_tractor',
      'respond_to_message',
      'return_to_lobby',
      'scan_target',
      'select_player_ship',
      'select_scenario',
      'set_boost',
      'set_helm',
      'set_helm_lateral',
      'set_helm_steering',
      'set_helm_thrust',
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
      'set_station_stance',
      'set_target',
      'set_torpedo_volley_target',
      'set_view',
      'show_on_screen',
      'start_impulse_charge',
      'start_transfer',
      'stop_transfer',
      'toggle_boost',
      'undock',
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

describe('dock / undock (issue #1159)', () => {
  it('dock sends ControlSystem Dock targeting the dock system', () => {
    const send = mkSend();
    ACTION_MAP.dock({ action: 'dock', target: 'berthing-clamps' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'berthing-clamps',
      payload: { type: 'Dock' },
    });
  });

  it('undock sends ControlSystem Undock targeting the dock system', () => {
    const send = mkSend();
    ACTION_MAP.undock({ action: 'undock', target: 'dock' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'dock',
      payload: { type: 'Undock' },
    });
  });

  it('sends neither command without an authored Dock SystemId', () => {
    const send = mkSend();
    ACTION_MAP.dock({ action: 'dock' }, send);
    ACTION_MAP.undock({ action: 'undock' }, send);
    expect(send).not.toHaveBeenCalled();
  });

  it('preserves the authored Dock target in correlated semantic envelopes', () => {
    const send = mkSend();
    ACTION_MAP.dock({ target: 'dock', correlation: 'helm-dock-1' }, send);
    ACTION_MAP.undock({ target: 'dock', correlation: 'helm-undock-1' }, send);
    expect(send.mock.calls).toEqual([
      ['ControlSystemCorrelated', {
        correlation: 'helm-dock-1',
        target: 'dock',
        payload: { type: 'Dock' },
      }],
      ['ControlSystemCorrelated', {
        correlation: 'helm-undock-1',
        target: 'dock',
        payload: { type: 'Undock' },
      }],
    ]);
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
  it('sends SetTarget with uuid without optimistically mutating gameplay state', () => {
    const send = mkSend();
    const mutate = mkMutate();
    ACTION_MAP.set_target({ action: 'set_target', uuid: 'abc' }, send, mutate);
    expect(mutate).not.toHaveBeenCalled();
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
  it('calls send correlated viewscreen SetView with Camera kind and direction', () => {
    const send = mkSend();
    ACTION_MAP.set_view({ action: 'set_view', direction: 'Aft', correlation: 'view-1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'view-1',
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
    ACTION_MAP.set_view({ action: 'set_view', direction: 'SensorsRadar', correlation: 'view-2' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'view-2',
      target: 'viewscreen',
      payload: { type: 'SetView', data: { mode: { kind: 'SensorsRadar' } } },
    });
  });

  it('preserves the legacy uncorrelated SetView route for non-Captain adapters', () => {
    const send = mkSend();
    ACTION_MAP.set_view({ action: 'set_view', direction: 'Aft' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'viewscreen',
      payload: { type: 'SetView', data: { mode: { kind: 'Camera', data: 'Aft' } } },
    });
  });
});

describe('set_red_alert', () => {
  it('sends correlated ControlSystem with the explicit desired active=true state', () => {
    const send = mkSend();
    ACTION_MAP.set_red_alert({ action: 'set_red_alert', active: true, correlation: 'alert-1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'alert-1',
      target: 'red-alert',
      payload: { type: 'SetRedAlert', data: { active: true } },
    });
    expect(send).toHaveBeenCalledTimes(1);
  });

  it('sends the explicit desired active=false state', () => {
    const send = mkSend();
    ACTION_MAP.set_red_alert({ action: 'set_red_alert', active: false, correlation: 'alert-2' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'alert-2',
      target: 'red-alert',
      payload: { type: 'SetRedAlert', data: { active: false } },
    });
  });

  it('coerces a missing active flag to false (never inverts)', () => {
    const send = mkSend();
    ACTION_MAP.set_red_alert({ action: 'set_red_alert', correlation: 'alert-3' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'alert-3',
      target: 'red-alert',
      payload: { type: 'SetRedAlert', data: { active: false } },
    });
  });

  it('does not create an untracked Red Alert command without a correlation', () => {
    const send = mkSend();
    ACTION_MAP.set_red_alert({ action: 'set_red_alert', active: true }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('set_station_stance (issue #1107)', () => {
  it('sends ControlSystem targeting the command system with the station + stance', () => {
    const send = mkSend();
    ACTION_MAP.set_station_stance(
      { action: 'set_station_stance', station: 'tactical', stance: 'tactical-hold' },
      send,
    );
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'command',
      payload: { type: 'SetStationStance', data: { station: 'tactical', stance: 'tactical-hold' } },
    });
    expect(send).toHaveBeenCalledTimes(1);
  });

  it('is a no-op when the station or stance is missing (never invents an order)', () => {
    const send = mkSend();
    ACTION_MAP.set_station_stance({ action: 'set_station_stance', station: 'tactical' }, send);
    ACTION_MAP.set_station_stance({ action: 'set_station_stance', stance: 'tactical-hold' }, send);
    expect(send).not.toHaveBeenCalled();
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

describe('set_helm_steering', () => {
  it('sends only SetSteering through the existing fine-system route', () => {
    const send = mkSend();
    ACTION_MAP.set_helm_steering({ action: 'set_helm_steering', value: -0.4 }, send);
    expect(send).toHaveBeenCalledOnce();
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-steering',
      payload: { type: 'SetSteering', data: { value: -0.4 } },
    });
  });

  it('refuses a missing or non-finite steering scalar', () => {
    const send = mkSend();
    ACTION_MAP.set_helm_steering({ value: Number.NaN }, send);
    expect(send).not.toHaveBeenCalled();
  });
});

describe('semantic Helm thrust variants', () => {
  it('keeps thrust and lateral thrust on their existing narrow routes', () => {
    const send = mkSend();
    ACTION_MAP.set_helm_thrust({ value: 0.75 }, send);
    ACTION_MAP.set_helm_lateral({ value: -0.25 }, send);
    expect(send.mock.calls).toEqual([
      ['ControlSystem', {
        target: 'helm-thrust',
        payload: { type: 'SetThrust', data: { value: 0.75 } },
      }],
      ['ControlSystem', {
        target: 'helm-lateral-thrust',
        payload: { type: 'LateralThrustInput', data: { lateral: -0.25 } },
      }],
    ]);
  });

  it('refuses non-finite thrust values before transport', () => {
    const send = mkSend();
    ACTION_MAP.set_helm_thrust({ value: Number.NaN }, send);
    ACTION_MAP.set_helm_lateral({ value: Number.POSITIVE_INFINITY }, send);
    expect(send).not.toHaveBeenCalled();
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

  it('carries semantic correlation on the same impulse target', () => {
    const send = mkSend();
    ACTION_MAP.start_impulse_charge({ correlation: 'helm-impulse-1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'helm-impulse-1',
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

  it('carries hold press and release correlations without changing SetBoost', () => {
    const send = mkSend();
    ACTION_MAP.set_boost({ active: true, correlation: 'helm-boost-on' }, send);
    ACTION_MAP.set_boost({ active: false, correlation: 'helm-boost-off' }, send);
    expect(send.mock.calls).toEqual([
      ['ControlSystemCorrelated', {
        correlation: 'helm-boost-on',
        target: 'helm-boost',
        payload: { type: 'SetBoost', data: { active: true } },
      }],
      ['ControlSystemCorrelated', {
        correlation: 'helm-boost-off',
        target: 'helm-boost',
        payload: { type: 'SetBoost', data: { active: false } },
      }],
    ]);
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

  it('carries semantic correlation without changing the legacy route', () => {
    const send = mkSend();
    ACTION_MAP.cancel_impulse({ correlation: 'cancel-impulse-1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'cancel-impulse-1',
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

  it('carries semantic correlation on the existing viewscreen route', () => {
    const send = mkSend();
    ACTION_MAP.set_radar_view({ correlation: 'helm-view-1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'helm-view-1',
      target: 'viewscreen',
      payload: { type: 'SetView', data: { mode: { kind: 'Radar' } } },
    });
  });
});

describe('authored Helm owner routing', () => {
  it('targets arbitrary authored SystemIds for every Helm command family', () => {
    const send = mkSend();
    ACTION_MAP.helm_input({
      thrust: 0.2,
      steering: -0.3,
      thrust_system_id: 'drive-array-port',
      steering_system_id: 'yaw-ring-a',
    }, send);
    ACTION_MAP.set_helm_thrust({ value: 0.4, control_system_id: 'drive-array-starboard' }, send);
    ACTION_MAP.set_helm_steering({ value: 0.5, control_system_id: 'yaw-ring-b' }, send);
    ACTION_MAP.set_helm_lateral({ value: -1, control_system_id: 'translation-ring' }, send);
    ACTION_MAP.start_impulse_charge({
      control_system_id: 'jump-coil', correlation: 'impulse-start',
    }, send);
    ACTION_MAP.cancel_impulse({
      control_system_id: 'jump-coil', correlation: 'impulse-cancel',
    }, send);
    ACTION_MAP.toggle_boost({ control_system_id: 'overburner' }, send);
    ACTION_MAP.set_boost({
      active: true, control_system_id: 'overburner', correlation: 'boost-on',
    }, send);
    ACTION_MAP.set_radar_view({
      control_system_id: 'forward-display', correlation: 'radar',
    }, send);
    ACTION_MAP.dock({
      target: 'legacy-dock', control_system_id: 'berthing-clamps', correlation: 'dock',
    }, send);
    ACTION_MAP.undock({
      target: 'legacy-dock', control_system_id: 'berthing-clamps', correlation: 'undock',
    }, send);

    expect(send.mock.calls.map(([, envelope]) => envelope.target)).toEqual([
      'drive-array-port',
      'yaw-ring-a',
      'drive-array-starboard',
      'yaw-ring-b',
      'translation-ring',
      'jump-coil',
      'jump-coil',
      'overburner',
      'overburner',
      'forward-display',
      'berthing-clamps',
      'berthing-clamps',
    ]);
  });
});

describe('security teams (issue #1346)', () => {
  it('refuses a verb the engine has never heard of rather than sending it', () => {
    const send = mkSend();
    expect(() =>
      ACTION_MAP.dispatch_security_team(
        {
          action: 'vent_the_deck',
          team_idx: 1,
          target: '00000000-0000-8000-8000-000000000042',
        },
        send,
      ),
    ).toThrow(TypeError);
    expect(send).not.toHaveBeenCalled();
  });

  it('sends a ControlSystem envelope targeting the security system', () => {
    const send = mkSend();
    ACTION_MAP.dispatch_security_team(
      {
        team_idx: 1,
        target: '00000000-0000-8000-8000-000000000042',
        action: 'assist_evacuation',
      },
      send,
    );
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'security',
      payload: {
        type: 'DispatchSecurityTeam',
        data: {
          team_idx: 1,
          target: '00000000-0000-8000-8000-000000000042',
          action: 'assist_evacuation',
        },
      },
    });
  });

  it('recall_security_team names only the team', () => {
    const send = mkSend();
    ACTION_MAP.recall_security_team({ action: 'recall_security_team', team_idx: 0 }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'security',
      payload: { type: 'RecallSecurityTeam', data: { team_idx: 0 } },
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

describe('recall_repair_team', () => {
  it('names only the team (issue #1385)', () => {
    const send = mkSend();
    ACTION_MAP.recall_repair_team({ action: 'recall_repair_team', team_idx: 1 }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'repair',
      payload: { type: 'RecallRepairTeam', data: { team_idx: 1 } },
    });
  });

  it('routes through the correlated envelope when the card correlated it', () => {
    const send = mkSend();
    ACTION_MAP.recall_repair_team(
      { action: 'recall_repair_team', team_idx: 0, correlation: 'abc123' },
      send,
    );
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'abc123',
      target: 'repair',
      payload: { type: 'RecallRepairTeam', data: { team_idx: 0 } },
    });
  });

  it('addresses the exact authored Repair owner when the console names one', () => {
    const send = mkSend();
    ACTION_MAP.recall_repair_team(
      { action: 'recall_repair_team', team_idx: 2, control_system_id: 'damage_control' },
      send,
    );
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'damage_control',
      payload: { type: 'RecallRepairTeam', data: { team_idx: 2 } },
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

describe('Engineering semantic owner identities', () => {
  it('carries each authoritative control SystemId into the wire target', () => {
    const cases = [
      ['engage_tractor', { control_system_id: 'tractor-primary' }, 'tractor-primary', 'EngageTractor'],
      ['release_tractor', { control_system_id: 'tractor-primary' }, 'tractor-primary', 'ReleaseTractor'],
      ['start_transfer', { control_system_id: 'umbilical-port' }, 'umbilical-port', 'StartTransfer'],
      ['stop_transfer', { control_system_id: 'umbilical-port' }, 'umbilical-port', 'StopTransfer'],
      ['dispatch_external_repair', { control_system_id: 'repair-control' }, 'repair-control', 'DispatchExternalRepair'],
      ['recall_external_repair', { control_system_id: 'repair-control' }, 'repair-control', 'RecallExternalRepair'],
      ['set_power', {
        control_system_id: 'reactor-main', target: 'helm', level: 3,
      }, 'reactor-main', 'SetPowerGroupAllocation'],
      ['dispatch_repair_team', {
        control_system_id: 'repair-control', team_idx: 0, target: 'core',
      }, 'repair-control', 'DispatchRepairTeam'],
      ['set_repair_target_priority', {
        control_system_id: 'repair-control', system_id: 'reactor-main',
      }, 'repair-control', 'SetRepairTargetPriority'],
    ];

    for (const [name, action, target, payloadType] of cases) {
      const send = mkSend();
      ACTION_MAP[name]({ ...action, correlation: `owner-${name}` }, send);
      expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', expect.objectContaining({
        correlation: `owner-${name}`,
        target,
        payload: expect.objectContaining({ type: payloadType }),
      }));
    }
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

  it('carries semantic correlation to the selected authored arc', () => {
    const send = mkSend();
    ACTION_MAP.set_shield_focus({
      action: 'set_shield_focus', arc_id: 'fore', focused: true, correlation: 'focus-1',
    }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'focus-1',
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
  it('sends correlated SetScienceTarget without painting an optimistic selection', () => {
    const send = mkSend();
    const mutate = mkMutate();
    ACTION_MAP.set_sensors_target({
      action: 'set_sensors_target', uuid: 'tgt-42', correlation: 'science-target-1',
    }, send, mutate);
    expect(mutate).not.toHaveBeenCalled();
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'science-target-1',
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

  it('retains the legacy uncorrelated route for non-semantic callers', () => {
    const send = mkSend();
    const mutate = mkMutate();
    ACTION_MAP.set_sensors_target({ action: 'set_sensors_target', uuid: 'tgt-42' }, send, mutate);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'sensors',
      payload: { type: 'SetScienceTarget', data: { uuid: 'tgt-42' } },
    });
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

  it('uses the additive correlated envelope for semantic hail feedback', () => {
    const send = mkSend();
    ACTION_MAP.hail({ target_uuid: 'npc-1', correlation: 'hail-1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'hail-1',
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

describe('retired select_comms_message route', () => {
  it('cannot emit the unconsumed host command even for a forged legacy action', () => {
    const send = mkSend();
    expect(ACTION_MAP.select_comms_message).toBeUndefined();
    dispatchConsoleAction({ action: 'select_comms_message', message_id: 'msg-42' }, send);
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

  it('preserves the exact message and index in the correlated response envelope', () => {
    const send = mkSend();
    ACTION_MAP.respond_to_message({
      message_id: 'msg-1', response_index: 2, correlation: 'response-1',
    }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'response-1',
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

  it('uses the additive correlated envelope for semantic clear feedback', () => {
    const send = mkSend();
    ACTION_MAP.clear_comms({ correlation: 'clear-1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'clear-1', target: 'comms', payload: { type: 'ClearComms' },
    });
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

  it('uses the additive correlated envelope for semantic viewscreen feedback', () => {
    const send = mkSend();
    ACTION_MAP.show_on_screen({ message_id: 'msg-7', correlation: 'show-1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'show-1',
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

// ── order_civilian (issue #1028) ─────────────────────────────────────────────

describe('order_civilian', () => {
  it('sends each verb targeting the navigation system', () => {
    const send = mkSend();
    ACTION_MAP.order_civilian({ target: 'civ-1', verb: 'hold' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'navigation',
      payload: { type: 'OrderCivilian', data: { target: 'civ-1', order: { verb: 'hold' } } },
    });

    ACTION_MAP.order_civilian({ target: 'civ-1', verb: 'divert', route: 'depot_run' }, send);
    expect(send).toHaveBeenLastCalledWith('ControlSystem', {
      target: 'navigation',
      payload: {
        type: 'OrderCivilian',
        data: { target: 'civ-1', order: { verb: 'divert', route: 'depot_run' } },
      },
    });

    ACTION_MAP.order_civilian({ target: 'civ-1', verb: 'divert', anchor: 'holding_point' }, send);
    expect(send).toHaveBeenLastCalledWith('ControlSystem', {
      target: 'navigation',
      payload: {
        type: 'OrderCivilian',
        data: { target: 'civ-1', order: { verb: 'divert', anchor: 'holding_point' } },
      },
    });

    ACTION_MAP.order_civilian({ target: 'civ-1', verb: 'dock', structure: 'depot' }, send);
    expect(send).toHaveBeenLastCalledWith('ControlSystem', {
      target: 'navigation',
      payload: {
        type: 'OrderCivilian',
        data: { target: 'civ-1', order: { verb: 'dock', structure: 'depot' } },
      },
    });
  });

  // The server refuses a divert naming both or neither, so sending one would be
  // a guaranteed rejection bounce. A mistyped lane silently read as an anchor
  // name is the exact failure the split destination exists to stop.
  it('sends nothing for an order that cannot be carried out', () => {
    const send = mkSend();
    for (const a of [
      { verb: 'hold' },
      { target: '', verb: 'hold' },
      { target: 'civ-1', verb: 'divert' },
      { target: 'civ-1', verb: 'divert', route: 'depot_run', anchor: 'holding_point' },
      { target: 'civ-1', verb: 'dock' },
      { target: 'civ-1', verb: 'evacuate' },
    ]) {
      ACTION_MAP.order_civilian(a, send);
    }
    expect(send).not.toHaveBeenCalled();
  });
});

describe('Navigation correlated semantic envelopes', () => {
  it('preserves correlation for chart, free/anchored waypoint, clear and civilian order', () => {
    const send = mkSend();
    const correlation = 'navigation-occurrence-7';

    ACTION_MAP.set_navigation_chart({ correlation }, send);
    ACTION_MAP.set_navigation_waypoint({ x: 1, z: 2, correlation }, send);
    ACTION_MAP.set_navigation_waypoint({
      x: 3, z: 4, source_uuid: 'beacon-a', correlation,
    }, send);
    ACTION_MAP.clear_navigation_waypoint({ correlation }, send);
    ACTION_MAP.order_civilian({
      target: 'civilian-a', verb: 'hold', correlation,
    }, send);

    expect(send.mock.calls).toEqual([
      ['ControlSystemCorrelated', {
        correlation,
        target: 'viewscreen',
        payload: { type: 'SetView', data: { mode: { kind: 'NavigationChart' } } },
      }],
      ['ControlSystemCorrelated', {
        correlation,
        target: 'navigation',
        payload: { type: 'SetNavigationWaypoint', data: { x: 1, z: 2 } },
      }],
      ['ControlSystemCorrelated', {
        correlation,
        target: 'navigation',
        payload: {
          type: 'SetNavigationWaypoint',
          data: { x: 3, z: 4, source_uuid: 'beacon-a' },
        },
      }],
      ['ControlSystemCorrelated', {
        correlation,
        target: 'navigation',
        payload: { type: 'ClearNavigationWaypoint' },
      }],
      ['ControlSystemCorrelated', {
        correlation,
        target: 'navigation',
        payload: {
          type: 'OrderCivilian',
          data: { target: 'civilian-a', order: { verb: 'hold' } },
        },
      }],
    ]);
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

// ── The science scan (issue #1032) ────────────────────────────────────────────

describe('scan_target', () => {
  it('sends correlated ScanTarget at the sensors system with the contact uuid', () => {
    // The sensors system, not a scan one: the suite is the thing aboard the
    // ship that can be commanded and damaged, so a scan rides the same
    // station-tenure admission the science target selection already does.
    const send = mkSend();
    ACTION_MAP.scan_target({
      action: 'scan_target', uuid: 'depot-1', correlation: 'scan-1',
    }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'scan-1',
      target: 'sensors',
      payload: { type: 'ScanTarget', data: { uuid: 'depot-1' } },
    });
  });

  it('sends nothing when no contact is named', () => {
    const send = mkSend();
    ACTION_MAP.scan_target({ action: 'scan_target' }, send);
    expect(send).not.toHaveBeenCalled();
  });

  it('retains the legacy uncorrelated transport for non-semantic callers', () => {
    const send = mkSend();
    ACTION_MAP.scan_target({ action: 'scan_target', uuid: 'depot-1' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'sensors',
      payload: { type: 'ScanTarget', data: { uuid: 'depot-1' } },
    });
  });
});

// ── set_objective_priority (issue #675) ───────────────────────────────────────

describe('set_objective_priority', () => {
  it('sends correlated SetObjectivePriority with id', () => {
    const send = mkSend();
    ACTION_MAP.set_objective_priority({
      action: 'set_objective_priority', id: 'obj-1', correlation: 'objective-1',
    }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'objective-1',
      target: 'captain',
      payload: { type: 'SetObjectivePriority', data: { id: 'obj-1' } },
    });
  });

  it('does not create an untracked Objective Priority command without a correlation', () => {
    const send = mkSend();
    ACTION_MAP.set_objective_priority({ action: 'set_objective_priority', id: 'obj-1' }, send);
    expect(send).not.toHaveBeenCalled();
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
    dispatchConsoleAction({ action: 'set_red_alert', active: true, correlation: 'alert-dispatch' }, send);
    expect(send).toHaveBeenCalledWith('ControlSystemCorrelated', {
      correlation: 'alert-dispatch',
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
    // Sensor selection has no optimistic patch and does not require mutate.
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
