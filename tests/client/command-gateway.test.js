import { describe, it, expect, afterEach } from 'vitest';
import {
  CONTROL_SYSTEM,
  controlSystemEnvelope,
  resolveTransport,
  sendControlSystem,
} from '../../gui/command-gateway.js';

describe('command-gateway envelope', () => {
  it('wraps target and payload in a ControlSystem envelope', () => {
    const env = controlSystemEnvelope('repair', { type: 'DispatchRepairTeam', data: { team_idx: 1 } });
    expect(env).toEqual({
      type: 'ControlSystem',
      data: {
        target: 'repair',
        payload: { type: 'DispatchRepairTeam', data: { team_idx: 1 } },
      },
    });
    expect(env.type).toBe(CONTROL_SYSTEM);
  });

  it('rejects a missing target', () => {
    expect(() => controlSystemEnvelope('', { type: 'X' })).toThrow(TypeError);
  });

  it('rejects a payload with no type', () => {
    expect(() => controlSystemEnvelope('repair', { data: {} })).toThrow(TypeError);
  });
});

describe('command-gateway transport resolution', () => {
  afterEach(() => {
    if (typeof globalThis.window !== 'undefined') delete globalThis.window;
  });

  it('prefers an explicit send function', () => {
    const calls = [];
    const send = (type, data) => calls.push([type, data]);
    expect(resolveTransport(send)).toBe(send);
    sendControlSystem('repair', { type: 'Ping' }, send);
    expect(calls).toEqual([['ControlSystem', { target: 'repair', payload: { type: 'Ping' } }]]);
  });

  it('falls back to the live connectionManager singleton', () => {
    const calls = [];
    globalThis.window = { connectionManager: { send: (type, data) => calls.push([type, data]) } };
    const env = sendControlSystem('repair', { type: 'Ping' });
    expect(env.data.target).toBe('repair');
    expect(calls).toEqual([['ControlSystem', { target: 'repair', payload: { type: 'Ping' } }]]);
  });

  it('returns null and sends nothing when there is no transport', () => {
    expect(resolveTransport(undefined)).toBeNull();
    expect(sendControlSystem('repair', { type: 'Ping' })).toBeNull();
  });
});
