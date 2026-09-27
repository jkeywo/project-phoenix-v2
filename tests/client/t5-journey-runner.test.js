import { it, expect } from 'vitest';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { selectJourneys, verifyCount, freshOutput } from '../../scripts/t5-journeys.mjs';

it('refuses unknown lanes and zero/missing/partial test matches', () => {
  expect(() => selectJourneys('all')).toThrow();
  const row = selectJourneys('rust')[0];
  expect(() => verifyCount(row, 'test result: ok. 0 passed; 0 failed;')).toThrow();
  expect(() => verifyCount(row, 'compilation finished')).toThrow();
  expect(verifyCount(row, 'test result: ok. 1 passed; 0 failed;')).toBe(1);
  expect(() => verifyCount(selectJourneys('js')[0], '', { numPassedTests: 4, numFailedTests: 1, success: false })).toThrow();
});
it('refuses existing output without modifying prior evidence', async () => {
  const parent = await mkdtemp(path.join(os.tmpdir(), 'phoenix-journeys-'));
  try {
    const output = path.join(parent, 'run'); await freshOutput(output);
    await writeFile(path.join(output, 'evidence.json'), 'old');
    await expect(freshOutput(output)).rejects.toMatchObject({ code: 'EEXIST' });
    expect(await readFile(path.join(output, 'evidence.json'), 'utf8')).toBe('old');
  } finally { await rm(parent, { recursive: true, force: true }); }
});
