import { describe, expect, it } from 'vitest';
import { validateTestBreakpoint } from '../workshop-test-breakpoint.js';

const worlds = ['assets/worlds/root.toml', 'assets/worlds/layer.toml'];

describe('typed Workshop Test breakpoints', () => {
  it('accepts only the bounded Flag/counter vocabulary and exact layer paths', () => {
    expect(validateTestBreakpoint({ condition: { kind: 'flag', name: 'alarm:on', value: true } }, worlds))
      .toEqual({ condition: { kind: 'flag', name: 'alarm:on', value: true } });
    expect(validateTestBreakpoint({ layer: worlds[1], condition: {
      kind: 'counter', name: 'arrivals', comparison: 'ge', value: 3,
    } }, worlds)).toMatchObject({ layer: worlds[1], condition: { comparison: 'ge', value: 3 } });
    expect(() => validateTestBreakpoint({ condition: { kind: 'counter', name: 'x', comparison: 'eval', value: 1 } }, worlds))
      .toThrow('counter');
    expect(() => validateTestBreakpoint({ layer: 'assets/worlds/missing.toml', condition: {
      kind: 'flag', name: 'x', value: true,
    } }, worlds)).toThrow('exact draft');
    expect(() => validateTestBreakpoint({ condition: { kind: 'flag', name: 'x', value: true, expression: 'debug()' } }, worlds))
      .toThrow('Flag');
  });
});
