import { describe, it, expect, vi } from 'vitest';
import { ACTION_MAP, dispatchConsoleAction } from '../../gui/action-map.js';

// ── ACTION_MAP structure ──────────────────────────────────────────────────────

describe('ACTION_MAP', () => {
  it('is frozen', () => {
    expect(Object.isFrozen(ACTION_MAP)).toBe(true);
  });

  it('contains exactly the 15 expected action keys', () => {
    expect(Object.keys(ACTION_MAP).sort()).toEqual([
      'cancel_impulse',
      'decrease_power',
      'dispatch_repair_team',
      'fire_phaser',
      'fire_torpedo',
      'helm_input',
      'increase_power',
      'set_phaser_mode',
      'set_radar_view',
      'set_sensors_target',
      'set_shield_focus',
      'set_target',
      'set_view',
      'start_impulse_charge',
      'toggle_red_alert',
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
