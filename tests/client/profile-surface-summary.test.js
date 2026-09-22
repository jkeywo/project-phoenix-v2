import { describe, it, expect } from 'vitest';
import { liveSurfaceRates } from '../../scripts/profile-surface-summary.mjs';

describe('native UI display cadence', () => {
  const event = (id, kind='console', visible=true, type='uploaded') =>
    ({event:type, surface:{id,kind,visible}});

  it('counts displayed frames, not copies, iterations or JS updates', () => {
    const events = [event(1), event(1), event(1,'console',true,'copied'),
      {event:'iteration'}, {event:'js-update'}, {event:'js-update'}];
    expect(liveSurfaceRates(events, 2)).toEqual({1:1});
  });

  it('keeps separate GM/player rates and detects a stalled visible surface', () => {
    expect(liveSurfaceRates([
      event(1),event(1),event(2,'gm'),event(3,'console',true,'copied'),
      event(4,'console',false),event(5,'lobby'),event(6,'hud'),
    ], 2)).toEqual({1:1,2:0.5,3:0});
  });

  it('rejects missing or invalid measurement windows', () => {
    for (const seconds of [0,-1,NaN,Infinity]) {
      expect(()=>liveSurfaceRates([],seconds)).toThrow('Invalid sample duration');
    }
  });
});
