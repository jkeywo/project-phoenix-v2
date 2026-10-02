import { describe, it, expect } from 'vitest';
import cases from './barrel-pattern-cases.json';
import { validateBlasterBanks } from '../blaster-validate.js';
import { validateTorpedoTubes } from '../torpedo-validate.js';
import { validateBarrelPatterns } from '../barrel-pattern-validate.js';

describe('shared authored barrel patterns', () => {
  it.each(cases)('preserves both original diagnostic projections %#', ({ value, blaster, torpedo }) => {
    expect(validateBlasterBanks(value)).toEqual(blaster);
    expect(validateTorpedoTubes(value)).toEqual(torpedo);
  });
  it('preserves non-finite offset text, strict identities and finding order', () => {
    const id = {};
    const rows = [{ id, pattern: [{ barrels: [NaN, Infinity], offset_secs: NaN }] }, { id }];
    expect(validateBarrelPatterns(rows, { path: 'rows', label: 'Row', duplicateLabel: 'row' })).toEqual([
      { path: 'rows[0].pattern[0].barrels', severity: 'error', message: 'Row "[object Object]" pattern step 0 references barrel index NaN but only 1 barrel(s) are declared' },
      { path: 'rows[0].pattern[0].barrels', severity: 'error', message: 'Row "[object Object]" pattern step 0 references barrel index Infinity but only 1 barrel(s) are declared' },
      { path: 'rows[0].pattern[0].offset_secs', severity: 'error', message: 'Row "[object Object]" pattern step 0 has offset_secs=NaN (must be a number >= 0)' },
      { path: 'rows[1].id', severity: 'error', message: 'Duplicate row id "[object Object]"' },
    ]);
  });
});
