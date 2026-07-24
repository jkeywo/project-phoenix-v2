import { describe, it, expect } from 'vitest';
import { validateTorpedoTubes } from '../torpedo-validate.js';
import { validateFile, hasBlockingErrors } from '../validation.js';

describe('validateTorpedoTubes (issue #766)', () => {
  it('accepts a legacy single-barrel tube with no pattern', () => {
    expect(validateTorpedoTubes([{ id: 'fore' }])).toEqual([]);
  });

  it('accepts a valid alternating + simultaneous pattern', () => {
    const findings = validateTorpedoTubes([
      {
        id: 'centre',
        barrels: ['b0', 'b1'],
        pattern: [
          { barrels: [0], offset_secs: 0.0 },
          { barrels: [1], offset_secs: 0.5 },
          { barrels: [0, 1], offset_secs: 1.0 },
        ],
      },
    ]);
    expect(findings).toEqual([]);
  });

  it('rejects a barrel index beyond the declared barrels', () => {
    const findings = validateTorpedoTubes([
      { id: 'centre', barrels: ['b0', 'b1'], pattern: [{ barrels: [2], offset_secs: 0 }] },
    ]);
    expect(findings.length).toBe(1);
    expect(findings[0].severity).toBe('error');
    expect(findings[0].message).toContain('barrel index 2');
    expect(findings[0].path).toBe('torpedoes.tubes[0].pattern[0].barrels');
  });

  it('rejects a step that fires no barrels', () => {
    const findings = validateTorpedoTubes([
      { id: 'centre', barrels: ['b0'], pattern: [{ barrels: [], offset_secs: 0 }] },
    ]);
    expect(findings.length).toBe(1);
    expect(findings[0].message).toContain('fires no barrels');
  });

  it('rejects a negative offset', () => {
    const findings = validateTorpedoTubes([
      { id: 'centre', barrels: ['b0'], pattern: [{ barrels: [0], offset_secs: -0.1 }] },
    ]);
    expect(findings.length).toBe(1);
    expect(findings[0].message).toContain('offset_secs');
  });

  it('rejects multiple barrels with no pattern', () => {
    const findings = validateTorpedoTubes([{ id: 'centre', barrels: ['b0', 'b1'] }]);
    expect(findings.length).toBe(1);
    expect(findings[0].message).toContain('pattern');
  });

  it('rejects duplicate tube ids', () => {
    const findings = validateTorpedoTubes([{ id: 'fore' }, { id: 'fore' }]);
    expect(findings.some((f) => f.message.includes('Duplicate'))).toBe(true);
  });

  it('is a no-op for a non-array', () => {
    expect(validateTorpedoTubes(undefined)).toEqual([]);
    expect(validateTorpedoTubes(null)).toEqual([]);
  });
});

describe('validateFile wires torpedo pattern validation (blocks save)', () => {
  it('surfaces an error-severity finding for an invalid barrel index', () => {
    const results = validateFile('assets/entities/x.toml', {
      tags: ['ship'],
      torpedoes: {
        tubes: [
          { id: 'centre', barrels: ['b0', 'b1'], pattern: [{ barrels: [5], offset_secs: 0 }] },
        ],
      },
    });
    const finding = results.find((r) => r.message.includes('barrel index 5'));
    expect(finding).toBeDefined();
    expect(finding.severity).toBe('error');
    // Error-severity => SaveFlow admission (issue #757) blocks the save.
    expect(hasBlockingErrors(results)).toBe(true);
  });

  it('does not flag a valid patterned tube', () => {
    const results = validateFile('assets/entities/x.toml', {
      tags: ['ship'],
      torpedoes: {
        tubes: [
          {
            id: 'centre',
            barrels: ['b0', 'b1'],
            pattern: [
              { barrels: [0], offset_secs: 0 },
              { barrels: [0, 1], offset_secs: 0.3 },
            ],
          },
        ],
      },
    });
    const finding = results.find((r) => r.message && r.message.includes('barrel'));
    expect(finding).toBeUndefined();
  });
});
