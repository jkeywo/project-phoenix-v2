import { describe, it, expect } from 'vitest';
import { parse } from 'smol-toml';
import { readFileSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';

import { validateStations } from '../stations-validate.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../..');

function readEntityToml(relPath) {
  const raw = readFileSync(resolve(projectRoot, relPath), 'utf-8');
  return parse(raw);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function validConfig() {
  // Extract the [stations] block from player_ship.toml
  const parsed = readEntityToml('assets/entities/player_ship.toml');
  return parsed.stations;
}

function makeConfig(overrides) {
  return { min_players: 1, max_players: 2, 1: [{ name: 'Alpha', consoles: ['Helm'], rank: 'Ltn.' }], 2: [{ name: 'Alpha', consoles: ['Helm'], rank: 'Ltn.' }], ...overrides };
}

// ── 1. Valid config ───────────────────────────────────────────────────────────

describe('valid config', () => {
  it('player_ship.toml stations block (1P–6P) is valid', () => {
    const result = validateStations(validConfig());
    expect(result.valid).toBe(true);
    expect(result.errors).toHaveLength(0);
  });

  it('minimal single-station config is valid', () => {
    const config = {
      min_players: 1,
      max_players: 1,
      1: [{ name: 'Bridge', consoles: ['CaptainChair', 'Helm'] }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(true);
  });
});

// ── 2. Duplicate names ───────────────────────────────────────────────────────

describe('duplicate names', () => {
  it('returns error when two stations at the same count share a name', () => {
    const config = makeConfig({
      1: [
        { name: 'Alpha', consoles: ['Helm'] },
        { name: 'Alpha', consoles: ['Tactical'] },
      ],
    });
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'duplicate-name')).toBe(true);
    expect(result.errors.find((e) => e.type === 'duplicate-name').station).toBe('Alpha');
  });

  it('same name across different counts is not an error', () => {
    const config = {
      min_players: 1,
      max_players: 2,
      1: [{ name: 'Helm', consoles: ['Helm'] }],
      2: [{ name: 'Helm', consoles: ['Helm'] }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(true);
  });
});

// ── 3. Empty consoles ─────────────────────────────────────────────────────────

describe('empty consoles', () => {
  it('returns error when a station has an empty consoles array', () => {
    const config = makeConfig({
      1: [{ name: 'Alpha', consoles: [] }],
    });
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'empty-consoles')).toBe(true);
  });

  it('returns error when consoles field is missing', () => {
    const config = makeConfig({
      1: [{ name: 'Alpha', consoles: undefined }],
    });
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'empty-consoles')).toBe(true);
  });
});

// ── 4. Unknown console ────────────────────────────────────────────────────────

describe('unknown console', () => {
  it('returns error for a console name not in the valid list', () => {
    const config = makeConfig({
      1: [{ name: 'Alpha', consoles: ['Helm', 'WarpDrive'] }],
    });
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'unknown-console')).toBe(true);
    expect(result.errors.find((e) => e.type === 'unknown-console').message).toContain('WarpDrive');
  });

  it('accepts all 9 valid console names', () => {
    const config = {
      min_players: 1,
      max_players: 1,
      1: [{
        name: 'All',
        consoles: ['CaptainChair', 'Helm', 'Tactical', 'Repair', 'Sensors', 'Shields', 'Navigation', 'Power', 'Comms'],
      }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(true);
  });
});

// ── 5. Count out of range ─────────────────────────────────────────────────────

describe('count out of range', () => {
  it('returns error when count is below min_players', () => {
    const config = makeConfig({
      min_players: 2,
      1: [{ name: 'Alpha', consoles: ['Helm'] }],
    });
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'count-out-of-range')).toBe(true);
  });

  it('returns error when count is above max_players', () => {
    const config = makeConfig({
      min_players: 1,
      max_players: 2,
      3: [{ name: 'Alpha', consoles: ['Helm'] }],
    });
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'count-out-of-range')).toBe(true);
  });
});

// ── 6. Dangling next ──────────────────────────────────────────────────────────

describe('dangling next', () => {
  it('returns error when explicit next targets a non-existent station at count+1', () => {
    const config = {
      min_players: 1,
      max_players: 2,
      1: [{ name: 'Alpha', consoles: ['Helm'], next: 'Omega' }],
      2: [{ name: 'Bravo', consoles: ['Helm'] }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'dangling-next')).toBe(true);
  });

  it('returns error when explicit next targets a count level that does not exist', () => {
    const config = {
      min_players: 1,
      max_players: 3,
      1: [{ name: 'Alpha', consoles: ['Helm'], next: 'Bravo' }],
      3: [{ name: 'Bravo', consoles: ['Helm'] }],
    };
    // count+1=2 does not exist at all — next target "Bravo" can't be resolved
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'dangling-next')).toBe(true);
  });

  it('does not error when count is at max (no next level to validate)', () => {
    const config = {
      min_players: 1,
      max_players: 1,
      1: [{ name: 'Alpha', consoles: ['Helm'], next: 'Ghost' }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(true);
  });

  it('does not error when explicit next matches a station at count+1', () => {
    const config = {
      min_players: 1,
      max_players: 2,
      1: [{ name: 'Alpha', consoles: ['Helm'], next: 'Bravo' }],
      2: [{ name: 'Bravo', consoles: ['Helm'] }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(true);
  });
});

// ── 7. Dangling previous ──────────────────────────────────────────────────────

describe('dangling previous', () => {
  it('returns error when explicit previous targets a non-existent station at count-1', () => {
    const config = {
      min_players: 1,
      max_players: 2,
      2: [{ name: 'Alpha', consoles: ['Helm'], previous: 'Omega' }],
      1: [{ name: 'Bravo', consoles: ['Helm'] }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'dangling-previous')).toBe(true);
  });

  it('returns error when explicit previous targets a count level that does not exist', () => {
    const config = {
      min_players: 1,
      max_players: 3,
      3: [{ name: 'Alpha', consoles: ['Helm'], previous: 'Bravo' }],
      1: [{ name: 'Bravo', consoles: ['Helm'] }],
    };
    // count-1=2 does not exist at all — prev target "Bravo" can't be resolved
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'dangling-previous')).toBe(true);
  });

  it('does not error when count is at min (no prev level to validate)', () => {
    const config = {
      min_players: 1,
      max_players: 1,
      1: [{ name: 'Alpha', consoles: ['Helm'], previous: 'Ghost' }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(true);
  });

  it('does not error when explicit previous matches a station at count-1', () => {
    // Alpha at count=1 needs an explicit next to avoid missing-next at 1→2
    const config = {
      min_players: 1,
      max_players: 2,
      2: [{ name: 'Bravo', consoles: ['Helm'], previous: 'Alpha' }],
      1: [{ name: 'Alpha', consoles: ['Helm'], next: 'Bravo' }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(true);
  });
});

// ── 8. Missing next ───────────────────────────────────────────────────────────

describe('missing next', () => {
  it('returns error when count < max, count+1 has stations, but no matching name and no explicit next', () => {
    const config = {
      min_players: 1,
      max_players: 2,
      1: [{ name: 'Alpha', consoles: ['Helm'] }],
      2: [{ name: 'Bravo', consoles: ['Helm'] }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'missing-next')).toBe(true);
  });

  it('does not error when count+1 does not exist (no successor level)', () => {
    const config = {
      min_players: 1,
      max_players: 3,
      1: [{ name: 'Alpha', consoles: ['Helm'] }],
      // count 2 is missing but 3 exists — still no successor at 1→2
      3: [{ name: 'Alpha', consoles: ['Helm'] }],
    };
    const result = validateStations(config);
    // count=1 has no explicit next, count+1=2 doesn't exist, so no error for count=1
    // count=3 is at max, so no missing-next check
    // But count=3 has no explicit previous and count-1=2 doesn't exist — implicit previous is informational, no error.
    expect(result.valid).toBe(true);
  });

  it('does not error when same-named station exists at count+1 (implicit next)', () => {
    const config = {
      min_players: 1,
      max_players: 2,
      1: [{ name: 'Helm', consoles: ['Helm'] }],
      2: [{ name: 'Helm', consoles: ['Helm'] }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(true);
  });
});

// ── 9. Parse errors ───────────────────────────────────────────────────────────

describe('parse errors', () => {
  it('returns error when a count key is not a number', () => {
    const config = makeConfig({
      abc: [{ name: 'Alpha', consoles: ['Helm'] }],
    });
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'parse-error')).toBe(true);
    expect(result.errors.find((e) => e.type === 'parse-error').message).toContain('abc');
  });

  it('returns error when config is null', () => {
    const result = validateStations(null);
    expect(result.valid).toBe(false);
    expect(result.errors).toHaveLength(1);
    expect(result.errors[0].type).toBe('parse-error');
  });

  it('returns error when config is undefined', () => {
    const result = validateStations(undefined);
    expect(result.valid).toBe(false);
    expect(result.errors).toHaveLength(1);
  });

  it('returns error when config is not an object', () => {
    const result = validateStations('not-an-object');
    expect(result.valid).toBe(false);
    expect(result.errors).toHaveLength(1);
  });

  it('returns error when min_players / max_players are missing', () => {
    const config = { 1: [{ name: 'Alpha', consoles: ['Helm'] }] };
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'parse-error')).toBe(true);
  });

  it('returns error when stations at a count is not an array', () => {
    const config = makeConfig({
      1: 'not-an-array',
    });
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'parse-error' && e.message.includes('not an array'))).toBe(true);
  });
});

// ── 10. Multiple errors ───────────────────────────────────────────────────────

describe('multiple errors', () => {
  it('collects all errors in one pass (does not short-circuit)', () => {
    const config = {
      min_players: 1,
      max_players: 2,
      1: [
        { name: 'Alpha', consoles: ['Helm', 'Hyperdrive'], next: 'Phantom' },
        { name: 'Alpha', consoles: [] },
      ],
      2: [{ name: 'Bravo', consoles: ['Helm'] }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    const types = result.errors.map((e) => e.type);
    expect(types).toContain('duplicate-name');
    expect(types).toContain('unknown-console');
    expect(types).toContain('empty-consoles');
    expect(types).toContain('dangling-next');
    expect(result.errors.length).toBeGreaterThanOrEqual(4);
  });
});

// ── 11. Edge cases ────────────────────────────────────────────────────────────

describe('edge cases', () => {
  it('empty min_players defaults to NaN and triggers parse error', () => {
    const config = { 1: [{ name: 'Alpha', consoles: ['Helm'] }] };
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors[0].type).toBe('parse-error');
  });

  it('count keys as strings are handled correctly', () => {
    const config = {
      min_players: 1,
      max_players: 1,
      '1': [{ name: 'Alpha', consoles: ['Helm'] }],
    };
    const result = validateStations(config);
    expect(result.valid).toBe(true);
  });

  it('float count keys are rejected as parse errors', () => {
    const config = makeConfig({
      '1.5': [{ name: 'Alpha', consoles: ['Helm'] }],
    });
    const result = validateStations(config);
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.type === 'parse-error')).toBe(true);
  });
});
