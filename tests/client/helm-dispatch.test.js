import { describe, it, expect, vi } from 'vitest';
import {
  HELM_THRUST_SYSTEM_ID,
  HELM_STEERING_SYSTEM_ID,
  HELM_IMPULSE_SYSTEM_ID,
  HELM_BOOST_SYSTEM_ID,
  LATERAL_THRUST_SYSTEM_ID,
  sendThrust,
  sendSteering,
  sendHelmInput,
  sendLateralThrust,
  startImpulseCharge,
  cancelImpulse,
  toggleBoost,
  setBoost,
} from '../../gui/helm-dispatch.js';

const mkSend = () => vi.fn();

describe('helm-dispatch sends per-axis commands through the gateway', () => {
  it('sends SetThrust to the helm-thrust system', () => {
    const send = mkSend();
    const env = sendThrust(0.5, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: HELM_THRUST_SYSTEM_ID,
      payload: { type: 'SetThrust', data: { value: 0.5 } },
    });
    expect(env).toEqual({
      type: 'ControlSystem',
      data: { target: 'helm-thrust', payload: { type: 'SetThrust', data: { value: 0.5 } } },
    });
  });

  it('sends SetSteering to the helm-steering system', () => {
    const send = mkSend();
    sendSteering(-0.3, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: HELM_STEERING_SYSTEM_ID,
      payload: { type: 'SetSteering', data: { value: -0.3 } },
    });
  });

  it('fans a joystick action out to both per-axis messages', () => {
    const send = mkSend();
    sendHelmInput(0.8, 0.2, send);
    expect(send).toHaveBeenCalledTimes(2);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-thrust',
      payload: { type: 'SetThrust', data: { value: 0.8 } },
    });
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-steering',
      payload: { type: 'SetSteering', data: { value: 0.2 } },
    });
  });

  it('defaults missing axis values to 0', () => {
    const send = mkSend();
    sendHelmInput(undefined, undefined, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-thrust',
      payload: { type: 'SetThrust', data: { value: 0 } },
    });
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: 'helm-steering',
      payload: { type: 'SetSteering', data: { value: 0 } },
    });
  });

  it('sends LateralThrustInput to the lateral-thrust system', () => {
    const send = mkSend();
    sendLateralThrust(0.6, send);
    expect(send).toHaveBeenCalledWith('ControlSystem', {
      target: LATERAL_THRUST_SYSTEM_ID,
      payload: { type: 'LateralThrustInput', data: { lateral: 0.6 } },
    });
  });

  it('sends StartImpulseCharge and CancelImpulse to the helm-impulse system', () => {
    const send = mkSend();
    startImpulseCharge(send);
    cancelImpulse(send);
    expect(send).toHaveBeenNthCalledWith(1, 'ControlSystem', {
      target: HELM_IMPULSE_SYSTEM_ID,
      payload: { type: 'StartImpulseCharge' },
    });
    expect(send).toHaveBeenNthCalledWith(2, 'ControlSystem', {
      target: HELM_IMPULSE_SYSTEM_ID,
      payload: { type: 'CancelImpulse' },
    });
  });

  it('sends ToggleBoost and SetBoost to the helm-boost system', () => {
    const send = mkSend();
    toggleBoost(send);
    setBoost(true, send);
    setBoost(false, send);
    expect(send).toHaveBeenNthCalledWith(1, 'ControlSystem', {
      target: HELM_BOOST_SYSTEM_ID,
      payload: { type: 'ToggleBoost' },
    });
    expect(send).toHaveBeenNthCalledWith(2, 'ControlSystem', {
      target: HELM_BOOST_SYSTEM_ID,
      payload: { type: 'SetBoost', data: { active: true } },
    });
    expect(send).toHaveBeenNthCalledWith(3, 'ControlSystem', {
      target: HELM_BOOST_SYSTEM_ID,
      payload: { type: 'SetBoost', data: { active: false } },
    });
  });

  it('returns null when there is no transport available', () => {
    expect(sendThrust(0.5, undefined)).toBeNull();
  });
});
