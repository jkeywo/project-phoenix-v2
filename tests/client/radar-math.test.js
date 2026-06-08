/**
 * tests/client/radar-math.test.js
 *
 * Unit tests for gui/radar-math.js — pure coordinate projection math.
 * Issue #447 (Slice 5a).
 *
 * Coordinate convention recap:
 *   World +X = starboard (right), World −Z = forward (north) at yaw = 0
 *   Radar +rx = right, Radar +rz = forward (up on display)
 */
import { describe, it, expect } from 'vitest';
import {
  worldToRadar,
  radarToScreen,
  autoScaleRange,
  clipToCircle,
  RADAR_MIN_RANGE,
} from '../../gui/radar-math.js';

// ── worldToRadar ─────────────────────────────────────────────────────────────

describe('worldToRadar – ship_relative', () => {
  it('entity at ship position maps to origin', () => {
    const r = worldToRadar(5, 3, 5, 3, 0, 'ship_relative');
    expect(r.rx).toBeCloseTo(0);
    expect(r.rz).toBeCloseTo(0);
  });

  it('entity directly ahead (−Z) at yaw=0 maps to positive rz', () => {
    // At yaw=0, forward = −Z direction.  Entity at (0, −1) relative to ship.
    const r = worldToRadar(0, -1, 0, 0, 0, 'ship_relative');
    expect(r.rx).toBeCloseTo(0);
    expect(r.rz).toBeCloseTo(1);  // positive = forward = up on radar
  });

  it('entity directly to starboard (+X) at yaw=0 maps to positive rx', () => {
    const r = worldToRadar(1, 0, 0, 0, 0, 'ship_relative');
    expect(r.rx).toBeCloseTo(1);
    expect(r.rz).toBeCloseTo(0);
  });

  it('entity directly aft (+Z) at yaw=0 maps to negative rz', () => {
    const r = worldToRadar(0, 1, 0, 0, 0, 'ship_relative');
    expect(r.rx).toBeCloseTo(0);
    expect(r.rz).toBeCloseTo(-1);  // aft = below on radar
  });

  it('entity to port (−X) at yaw=0 maps to negative rx', () => {
    const r = worldToRadar(-1, 0, 0, 0, 0, 'ship_relative');
    expect(r.rx).toBeCloseTo(-1);
    expect(r.rz).toBeCloseTo(0);
  });

  it('non-zero ship position is subtracted correctly', () => {
    // Ship at (10, 20), entity at (11, 18) → dx=1, dz=−2.
    // At yaw=0: rx = 1*1 + (−2)*0 = 1, rz = 1*0 − (−2)*1 = 2.
    const r = worldToRadar(11, 18, 10, 20, 0, 'ship_relative');
    expect(r.rx).toBeCloseTo(1);
    expect(r.rz).toBeCloseTo(2);
  });

  it('yaw=π/2 rotates the projection 90° clockwise', () => {
    // Ship faces +X (yaw = π/2).  Entity ahead of ship is in the +X direction.
    // dx=1, dz=0; cos(π/2)≈0, sin(π/2)≈1
    //   rx = 1*0 + 0*1 = 0
    //   rz = 1*1 − 0*0 = 1 (entity is ahead, positive rz)
    const r = worldToRadar(1, 0, 0, 0, Math.PI / 2, 'ship_relative');
    expect(r.rx).toBeCloseTo(0, 4);
    expect(r.rz).toBeCloseTo(1, 4);
  });

  it('yaw=π/4 (45°) projects diagonal entity correctly', () => {
    // Ship at origin, yaw=π/4.  Entity at (1, -1) (northeast in world).
    // dx=1, dz=-1; cos(π/4)=sin(π/4)=√2/2
    //   rx = 1*(√2/2) + (-1)*(√2/2) = 0
    //   rz = 1*(√2/2) - (-1)*(√2/2) = √2
    const r = worldToRadar(1, -1, 0, 0, Math.PI / 4, 'ship_relative');
    expect(r.rx).toBeCloseTo(0, 4);
    expect(r.rz).toBeCloseTo(Math.SQRT2, 4);
  });

  it('defaults to ship_relative when orientation is unrecognised', () => {
    const r1 = worldToRadar(0, -5, 0, 0, 0, 'ship_relative');
    const r2 = worldToRadar(0, -5, 0, 0, 0, 'unknown_mode');
    expect(r2.rx).toBeCloseTo(r1.rx);
    expect(r2.rz).toBeCloseTo(r1.rz);
  });
});

describe('worldToRadar – world_fixed', () => {
  it('entity directly ahead (−Z) maps to positive rz regardless of yaw', () => {
    // Ship at origin, yaw=π (facing −X).  Entity at (0, −1) is still north.
    const r = worldToRadar(0, -1, 0, 0, Math.PI, 'world_fixed');
    expect(r.rx).toBeCloseTo(0);
    expect(r.rz).toBeCloseTo(1);
  });

  it('entity to +X maps to positive rx', () => {
    const r = worldToRadar(3, 0, 0, 0, 0, 'world_fixed');
    expect(r.rx).toBeCloseTo(3);
    expect(r.rz).toBeCloseTo(0);
  });

  it('ship position is subtracted from entity position', () => {
    // Ship at (5, 5), entity at (5, 3) → entity is 2 units north.
    const r = worldToRadar(5, 3, 5, 5, 0, 'world_fixed');
    expect(r.rx).toBeCloseTo(0);
    expect(r.rz).toBeCloseTo(2);  // +Z delta=−2 → rz = +2
  });

  it('yaw does not affect the result', () => {
    const r0 = worldToRadar(2, 3, 0, 0, 0,         'world_fixed');
    const rP = worldToRadar(2, 3, 0, 0, Math.PI/3, 'world_fixed');
    expect(rP.rx).toBeCloseTo(r0.rx);
    expect(rP.rz).toBeCloseTo(r0.rz);
  });
});

describe('worldToRadar – world_centred', () => {
  it('entity at world origin maps to radar origin', () => {
    const r = worldToRadar(0, 0, 100, 200, 1.5, 'world_centred');
    expect(r.rx).toBeCloseTo(0);
    expect(r.rz).toBeCloseTo(0);
  });

  it('entity at (1, -1) in world space maps to (1, 1) in radar space', () => {
    // entityX=1, entityZ=-1 → rx=1, rz=-(-1)=1
    const r = worldToRadar(1, -1, 99, 99, 0, 'world_centred');
    expect(r.rx).toBeCloseTo(1);
    expect(r.rz).toBeCloseTo(1);
  });

  it('ship position is ignored (no translation)', () => {
    const r1 = worldToRadar(5, -3, 0,   0,  0, 'world_centred');
    const r2 = worldToRadar(5, -3, 100, 50, 0, 'world_centred');
    expect(r1.rx).toBeCloseTo(r2.rx);
    expect(r1.rz).toBeCloseTo(r2.rz);
  });
});

// ── radarToScreen ─────────────────────────────────────────────────────────────

describe('radarToScreen', () => {
  it('radar origin maps to screen offset (0, 0)', () => {
    const s = radarToScreen(0, 0, 100, 350);
    expect(s.sx).toBeCloseTo(0);
    expect(s.sy).toBeCloseTo(0);
  });

  it('entity at full range forward (+rz) maps to −sy (up on canvas)', () => {
    const s = radarToScreen(0, 100, 100, 350);
    expect(s.sx).toBeCloseTo(0);
    expect(s.sy).toBeCloseTo(-350);  // at range boundary, top of radar
  });

  it('entity at full range to starboard maps to +sx', () => {
    const s = radarToScreen(100, 0, 100, 350);
    expect(s.sx).toBeCloseTo(350);
    expect(s.sy).toBeCloseTo(0);
  });

  it('entity at half range maps to half canvas radius', () => {
    const s = radarToScreen(50, 0, 100, 360);
    expect(s.sx).toBeCloseTo(180);
    expect(s.sy).toBeCloseTo(0);
  });

  it('entity aft (−rz) maps to +sy (down on canvas)', () => {
    const s = radarToScreen(0, -100, 100, 350);
    expect(s.sx).toBeCloseTo(0);
    expect(s.sy).toBeCloseTo(350);  // aft = bottom of radar
  });

  it('entity to port (−rx) maps to −sx (left on canvas)', () => {
    const s = radarToScreen(-75, 0, 100, 200);
    expect(s.sx).toBeCloseTo(-150);
    expect(s.sy).toBeCloseTo(0);
  });

  it('range=range maps to exactly canvasRadius at boundary', () => {
    const R = 280;
    const range = 150;
    const s = radarToScreen(range, 0, range, R);
    expect(s.sx).toBeCloseTo(R);
  });
});

// ── autoScaleRange ───────────────────────────────────────────────────────────

describe('autoScaleRange', () => {
  it('returns RADAR_MIN_RANGE when array is empty', () => {
    expect(autoScaleRange([])).toBe(RADAR_MIN_RANGE);
  });

  it('returns RADAR_MIN_RANGE when array is null/undefined', () => {
    expect(autoScaleRange(null)).toBe(RADAR_MIN_RANGE);
    expect(autoScaleRange(undefined)).toBe(RADAR_MIN_RANGE);
  });

  it('returns RADAR_MIN_RANGE when single entity is at origin', () => {
    expect(autoScaleRange([{ rx: 0, rz: 0 }])).toBe(RADAR_MIN_RANGE);
  });

  it('returns maxDist * (1 + margin) for a single entity', () => {
    const range = autoScaleRange([{ rx: 100, rz: 0 }], 0.1);
    expect(range).toBeCloseTo(110);
  });

  it('uses default margin of 10% when margin is omitted', () => {
    const range = autoScaleRange([{ rx: 0, rz: 200 }]);
    expect(range).toBeCloseTo(220);
  });

  it('picks the furthest entity distance', () => {
    const range = autoScaleRange([
      { rx: 50, rz: 0 },
      { rx: 0,  rz: 80 },
      { rx: 30, rz: 40 },  // dist = 50 (less than 80)
    ], 0.0);
    expect(range).toBeCloseTo(80);
  });

  it('margin=0 returns exact max distance', () => {
    const range = autoScaleRange([{ rx: 60, rz: 0 }], 0);
    expect(range).toBeCloseTo(60);
  });

  it('result is never less than RADAR_MIN_RANGE even with margin=0', () => {
    const range = autoScaleRange([{ rx: 5, rz: 0 }], 0.0);
    expect(range).toBeGreaterThanOrEqual(RADAR_MIN_RANGE);
  });

  it('entity at exact diagonal (30-40-50 triangle) is measured correctly', () => {
    // dist = sqrt(900 + 1600) = 50; margin=0 → max(10, 50) = 50
    const range = autoScaleRange([{ rx: 30, rz: 40 }], 0.0);
    expect(range).toBeCloseTo(50);
  });

  it('handles rz-only entity positions', () => {
    const range = autoScaleRange([{ rx: 0, rz: 300 }], 0.1);
    expect(range).toBeCloseTo(330);
  });
});

// ── clipToCircle ─────────────────────────────────────────────────────────────

describe('clipToCircle', () => {
  it('calls ctx.arc and ctx.clip without throwing', () => {
    const calls = [];
    const ctx = {
      beginPath: () => calls.push('beginPath'),
      arc: (x, y, r, s, e) => calls.push(['arc', x, y, r]),
      clip: () => calls.push('clip'),
    };
    clipToCircle(ctx, 100, 100, 50);
    expect(calls[0]).toBe('beginPath');
    expect(calls[1]).toEqual(['arc', 100, 100, 50]);
    expect(calls[2]).toBe('clip');
  });

  it('passes the correct centre and radius to ctx.arc', () => {
    let arcArgs;
    const ctx = {
      beginPath: () => {},
      arc: (...args) => { arcArgs = args; },
      clip: () => {},
    };
    clipToCircle(ctx, 200, 150, 80);
    expect(arcArgs[0]).toBe(200);  // cx
    expect(arcArgs[1]).toBe(150);  // cy
    expect(arcArgs[2]).toBe(80);   // radius
    expect(arcArgs[3]).toBe(0);    // startAngle
    expect(arcArgs[4]).toBeCloseTo(Math.PI * 2);  // endAngle
  });
});

// ── RADAR_MIN_RANGE constant ──────────────────────────────────────────────────

describe('RADAR_MIN_RANGE', () => {
  it('equals 10.0', () => {
    expect(RADAR_MIN_RANGE).toBe(10.0);
  });
});
