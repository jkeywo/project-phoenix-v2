import { describe, it, expect } from 'vitest';
import { validateBlasterBanks } from '../blaster-validate.js';
import { validateFile, hasBlockingErrors } from '../validation.js';

describe('validateBlasterBanks (issue #765)', () => {
  it('accepts a legacy single-barrel bank with no pattern', () => {
    expect(validateBlasterBanks([{ id: 'fore' }])).toEqual([]);
  });

  it('accepts a valid alternating + simultaneous pattern', () => {
    const findings = validateBlasterBanks([
      {
        id: 'heavy',
        barrels: ['b0', 'b1'],
        pattern: [
          { barrels: [0], offset_secs: 0.0 },
          { barrels: [1], offset_secs: 0.3 },
          { barrels: [0, 1], offset_secs: 0.6 },
        ],
      },
    ]);
    expect(findings).toEqual([]);
  });

  it('rejects a barrel index beyond the declared barrels', () => {
    const findings = validateBlasterBanks([
      { id: 'heavy', barrels: ['b0', 'b1'], pattern: [{ barrels: [2], offset_secs: 0 }] },
    ]);
    expect(findings.length).toBe(1);
    expect(findings[0].severity).toBe('error');
    expect(findings[0].message).toContain('barrel index 2');
    expect(findings[0].path).toBe('weapons_console.blaster_banks[0].pattern[0].barrels');
  });

  it('rejects a step that fires no barrels', () => {
    const findings = validateBlasterBanks([
      { id: 'heavy', barrels: ['b0'], pattern: [{ barrels: [], offset_secs: 0 }] },
    ]);
    expect(findings.length).toBe(1);
    expect(findings[0].message).toContain('fires no barrels');
  });

  it('rejects a negative offset', () => {
    const findings = validateBlasterBanks([
      { id: 'heavy', barrels: ['b0'], pattern: [{ barrels: [0], offset_secs: -0.1 }] },
    ]);
    expect(findings.length).toBe(1);
    expect(findings[0].message).toContain('offset_secs');
  });

  it('rejects multiple barrels with no pattern', () => {
    const findings = validateBlasterBanks([{ id: 'heavy', barrels: ['b0', 'b1'] }]);
    expect(findings.length).toBe(1);
    expect(findings[0].message).toContain('pattern');
  });

  it('rejects duplicate bank ids', () => {
    const findings = validateBlasterBanks([{ id: 'fore' }, { id: 'fore' }]);
    expect(findings.some((f) => f.message.includes('Duplicate'))).toBe(true);
  });

  it('is a no-op for a non-array', () => {
    expect(validateBlasterBanks(undefined)).toEqual([]);
    expect(validateBlasterBanks(null)).toEqual([]);
  });
});

describe('validateFile wires blaster pattern validation (blocks save)', () => {
  it('surfaces an error-severity finding for an invalid barrel index', () => {
    const results = validateFile('assets/entities/x.toml', {
      tags: ['ship'],
      weapons_console: {
        blaster_banks: [
          { id: 'heavy', barrels: ['b0', 'b1'], pattern: [{ barrels: [5], offset_secs: 0 }] },
        ],
      },
    });
    const blasterFinding = results.find((r) => r.message.includes('barrel index 5'));
    expect(blasterFinding).toBeDefined();
    expect(blasterFinding.severity).toBe('error');
    // Error-severity => SaveFlow admission (issue #757) blocks the save.
    expect(hasBlockingErrors(results)).toBe(true);
  });

  it('does not flag a valid patterned bank', () => {
    const results = validateFile('assets/entities/x.toml', {
      tags: ['ship'],
      weapons_console: {
        blaster_banks: [
          {
            id: 'heavy',
            barrels: ['b0', 'b1'],
            pattern: [
              { barrels: [0], offset_secs: 0 },
              { barrels: [0, 1], offset_secs: 0.3 },
            ],
          },
        ],
      },
    });
    expect(results.some((r) => r.message.includes('barrel'))).toBe(false);
  });
});
