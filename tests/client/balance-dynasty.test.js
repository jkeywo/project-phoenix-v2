import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { parse } from 'smol-toml';
import { tasks, generatedWorld, classifyReport, summarize, prepareOutput } from '../../scripts/balance-dynasty.mjs';

const source = readFileSync(new URL('../../assets/worlds/cruiser_elimination.toml', import.meta.url), 'utf8');
describe('ratified Dynasty balance evaluation', () => {
  it('refuses a reused output directory and preserves its prior evidence', async () => {
    const parent = await mkdtemp(path.join(os.tmpdir(), 'phoenix-balance-output-'));
    try {
      const output = path.join(parent, 'batch');
      await prepareOutput(output);
      const report = path.join(output, '001.json');
      await writeFile(report, 'prior evidence');
      await expect(prepareOutput(output)).rejects.toMatchObject({ code: 'EEXIST' });
      expect(await readFile(report, 'utf8')).toBe('prior evidence');
    } finally { await rm(parent, { recursive: true, force: true }); }
  });
  it('pairs all 200 duels by seed/condition and mirrors actual positions/headings', () => {
    const plan = tasks();
    const duels = plan.filter(task => task.kind === 'duel');
    expect(duels).toHaveLength(200);
    expect(plan.filter(task => task.kind === 'team')).toHaveLength(20);
    for (let index = 0; index < duels.length; index += 2) {
      const a = duels[index], b = duels[index + 1];
      expect([a.seed, a.condition]).toEqual([b.seed, b.condition]);
      const original = parse(generatedWorld(source, a));
      const mirrored = parse(generatedWorld(source, b));
      expect(original.ship_slot.map(slot => slot.default_ship)).toEqual([
        'assets/entities/alliance_cruiser.toml', 'assets/entities/dynasty_player_cruiser.toml',
      ]);
      expect(original.entity).toHaveLength(2);
      expect(original.script.setup).not.toContain('_two');
      expect(original.script.setup).toContain('alliance_losses >= 1');
      expect(original.script.setup).toContain('dynasty_losses >= 1');
      for (let ship = 0; ship < 2; ship++) {
        expect(mirrored.entity[ship].transform.position[2]).toBe(-original.entity[ship].transform.position[2]);
        expect(mirrored.entity[ship].transform.rotation[1]).toBeCloseTo(original.entity[ship].transform.rotation[1] + Math.PI);
      }
    }
  });
  it('keeps the four-ship reference team rules intact', () => {
    const world = parse(generatedWorld(source, tasks().at(-1)));
    expect(world.entity).toHaveLength(4);
    expect(world.ship_slot).toHaveLength(4);
    expect(world.script.setup).toBe(parse(source).script.setup);
  });
  it('classifies report-bearing team outcomes without treating every report as victory', () => {
    const report = name => ({ final_phase: 'GameOver', outcome: 'reported', scenario: { flags: [{ name, value: 1 }] } });
    expect(classifyReport(report('alliance_victory'))).toBe('alliance');
    expect(classifyReport(report('dynasty_victory'))).toBe('dynasty');
    expect(classifyReport(report('match_draw'))).toBe('draw');
    expect(classifyReport({ final_phase: 'InProgress', outcome: 'timeout', scenario: { flags: [] } })).toBe('timeout');
    expect(() => classifyReport({ ...report('alliance_victory'), scenario: { flags: [] } })).toThrow();
    expect(() => classifyReport({ outcome: 'timeout' })).toThrow();
  });
  it('counts draws/timeouts as half a win and refuses incomplete/failed acceptance', () => {
    const runs = tasks().filter(task => task.kind === 'duel').map((task, index) => ({ ...task,
      outcome: index < 80 ? 'alliance' : index < 160 ? 'dynasty' : index < 180 ? 'draw' : 'timeout' }));
    expect(summarize(runs, 'duel')).toMatchObject({ allianceRate: 0.5, dynastyRate: 0.5, parity: true, complete: true });
    expect(summarize(runs.slice(1), 'duel').parity).toBe(false);
    expect(summarize([{ ...runs[0], outcome: 'failed' }, ...runs.slice(1)], 'duel').parity).toBe(false);
  });
});
