import { describe, expect, it } from 'vitest';
import { evaluateFleetDigests } from '../../scripts/fleet-digest-evidence.mjs';

const frame = (from, tick, digest = '0123456789abcdef') => ({ t: 'digest', d: { from, tick, digest }, at: '2026-09-27T12:00:01Z' });
const peers = () => [0, 1].map(slot => ({ label: `peer-${slot}`, slot, frames: [frame(slot, 2), frame(slot, 0), frame(slot, 1)] }));

describe('fleet digest evidence', () => {
  it('sorts complete checkpoints, includes tick zero and tolerates exact duplicates and unrelated frames', () => {
    const rows = peers(); rows[0].frames.push(frame(0, 0), { t: 'tick', d: {} });
    const result = evaluateFleetDigests(rows);
    expect(result.passed).toBe(true);
    expect(result.common.map(row => row.tick)).toEqual([0, 1, 2]);
  });
  it.each([null, {}, { from: 5, tick: 0, digest: '0123456789abcdef' },
    { from: 0, tick: -1, digest: '0123456789abcdef' }, { from: 0, tick: 0.5, digest: '0123456789abcdef' },
    { from: 0, tick: 0, digest: 'ABCDEF0123456789' }, { from: 0, tick: 0, digest: 'abc' }])('rejects malformed observations before the cutoff: %j', d => {
    const rows = peers(); rows[0].frames.push({ t: 'digest', d });
    const result = evaluateFleetDigests(rows, { afterTick: 0 });
    expect(result.common).toHaveLength(2); expect(result.passed).toBe(false); expect(result.malformed).not.toHaveLength(0);
  });
  it('rejects conflicts before the cutoff and disagreements after it', () => {
    const rows = peers(); rows[0].frames.push(frame(0, 0, 'ffffffffffffffff'));
    expect(evaluateFleetDigests(rows, { afterTick: 0 }).conflicts).toHaveLength(1);
    rows[0].frames.pop(); rows[0].frames[0].d.digest = 'ffffffffffffffff';
    expect(evaluateFleetDigests(rows).passed).toBe(false);
  });
  it('requires two complete checkpoints and distinct expected peers', () => {
    const rows = peers(); rows[0].frames = [frame(0, 2)];
    expect(evaluateFleetDigests(rows).passed).toBe(false);
    expect(evaluateFleetDigests([]).passed).toBe(false);
    expect(evaluateFleetDigests([rows[0], rows[0]]).passed).toBe(false);
    expect(evaluateFleetDigests(peers().map(row => ({ ...row, slot: 0 }))).passed).toBe(false);
  });
  it('uses exclusive time cutoffs without hiding invalid older evidence', () => {
    const rows = peers();
    expect(evaluateFleetDigests(rows, { afterTime: '2026-09-27T12:00:01Z' }).passed).toBe(false);
    rows[0].frames.push({ ...frame(9, 0), at: '2026-09-27T11:00:00Z' });
    expect(evaluateFleetDigests(rows, { afterTime: '2026-09-27T12:00:00Z' }).passed).toBe(false);
  });
});
