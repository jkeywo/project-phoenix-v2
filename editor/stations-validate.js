/**
 * stations-validate.js
 *
 * Pure validation module for [[stations]] blocks in entity TOML.
 * Follows the same rules as the Rust stations_config.rs parser.
 *
 * No DOM manipulation; fully testable in Node.
 */

const VALID_CONSOLE_NAMES = [
  'CaptainChair',
  'Helm',
  'Tactical',
  'Repair',
  'Sensors',
  'Shields',
  'Navigation',
  'Power',
  'Comms',
];

/**
 * Validate a parsed [stations] block.
 *
 * @param {object|null|undefined} config  Parsed stations object (min_players, max_players, count keys).
 * @returns {{ valid: boolean, errors: Array<{count: number|null, station: string|null, message: string, type: string}> }}
 */
export function validateStations(config) {
  const errors = [];

  if (!config || typeof config !== 'object') {
    return {
      valid: false,
      errors: [{ count: null, station: null, message: 'Stations config is missing or invalid', type: 'parse-error' }],
    };
  }

  const minPlayers = Number(config.min_players);
  const maxPlayers = Number(config.max_players);

  if (isNaN(minPlayers) || isNaN(maxPlayers)) {
    return {
      valid: false,
      errors: [{ count: null, station: null, message: 'min_players and max_players must be numbers', type: 'parse-error' }],
    };
  }

  // Collect count keys (exclude metadata fields)
  const countKeys = [];
  for (const key of Object.keys(config)) {
    if (key === 'min_players' || key === 'max_players' || key === 'complexity_presets') continue;
    countKeys.push(key);
  }

  // Parse count keys into numeric lookup; collect parse/count-range errors
  const parsedCounts = {};

  for (const key of countKeys) {
    const count = Number(key);
    if (!Number.isInteger(count) || isNaN(count)) {
      errors.push({ count: null, station: null, message: `Invalid player count key: "${key}"`, type: 'parse-error' });
      continue;
    }

    if (count < minPlayers || count > maxPlayers) {
      errors.push({ count, station: null, message: `Player count ${count} is out of range [${minPlayers}, ${maxPlayers}]`, type: 'count-out-of-range' });
    }

    const defs = config[key];
    if (!Array.isArray(defs)) {
      errors.push({ count, station: null, message: `Stations at count ${count} is not an array`, type: 'parse-error' });
      continue;
    }

    // Station-level validation for this count
    const seenNames = new Set();
    const validDefs = [];

    for (const def of defs) {
      if (!def || typeof def !== 'object') continue;

      // ── Duplicate names ──
      if (seenNames.has(def.name)) {
        errors.push({ count, station: def.name, message: `Duplicate station name "${def.name}" at player count ${count}`, type: 'duplicate-name' });
      } else {
        seenNames.add(def.name);
      }

      // ── Empty consoles ──
      if (!Array.isArray(def.consoles) || def.consoles.length === 0) {
        errors.push({ count, station: def.name, message: `Station "${def.name}" at player count ${count} has no consoles`, type: 'empty-consoles' });
      }

      // ── Unknown console names ──
      if (Array.isArray(def.consoles)) {
        for (const consoleName of def.consoles) {
          if (!VALID_CONSOLE_NAMES.includes(consoleName)) {
            errors.push({ count, station: def.name, message: `Unknown console "${consoleName}" in station "${def.name}" at player count ${count}`, type: 'unknown-console' });
          }
        }
      }

      validDefs.push(def);
    }

    parsedCounts[count] = validDefs;
  }

  // Cross-count validation (next / previous / missing)
  for (const countStr of Object.keys(parsedCounts)) {
    const count = Number(countStr);
    const defs = parsedCounts[count];

    for (const def of defs) {
      const hasExplicitNext = def.next !== undefined && def.next !== null && def.next !== '';
      const hasExplicitPrevious = def.previous !== undefined && def.previous !== null && def.previous !== '';

      // ── Dangling next ──
      if (hasExplicitNext && count < maxPlayers) {
        const nextCount = count + 1;
        const nextDefs = parsedCounts[nextCount];
        if (!nextDefs || !nextDefs.some((d) => d.name === def.next)) {
          errors.push({
            count,
            station: def.name,
            message: `Station "${def.name}" at count ${count} has dangling next "${def.next}" — not found at count ${nextCount}`,
            type: 'dangling-next',
          });
        }
      }

      // ── Missing next (no explicit next, no same-named station at count+1) ──
      if (!hasExplicitNext && count < maxPlayers) {
        const nextCount = count + 1;
        const nextDefs = parsedCounts[nextCount];
        if (nextDefs && nextDefs.length > 0) {
          if (!nextDefs.some((d) => d.name === def.name)) {
            errors.push({
              count,
              station: def.name,
              message: `Station "${def.name}" at count ${count} has no explicit next and no matching station at count ${nextCount}`,
              type: 'missing-next',
            });
          }
        }
      }

      // ── Dangling previous ──
      if (hasExplicitPrevious && count > minPlayers) {
        const prevCount = count - 1;
        const prevDefs = parsedCounts[prevCount];
        if (!prevDefs || !prevDefs.some((d) => d.name === def.previous)) {
          errors.push({
            count,
            station: def.name,
            message: `Station "${def.name}" at count ${count} has dangling previous "${def.previous}" — not found at count ${prevCount}`,
            type: 'dangling-previous',
          });
        }
      }
    }
  }

  return { valid: errors.length === 0, errors };
}
