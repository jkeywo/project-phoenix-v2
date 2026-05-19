import { parse, stringify } from 'smol-toml';

/**
 * Parse an entity TOML string into a plain JS object.
 * Throws if the TOML is syntactically invalid.
 * @param {string} text
 * @returns {object}
 */
export function parseEntityToml(text) {
  return parse(text);
}

/**
 * Serialize a plain JS object back to a TOML string.
 * @param {object} obj
 * @returns {string}
 */
export function stringifyEntityToml(obj) {
  return stringify(obj);
}

/**
 * Validate the top-level shape of an entity config object.
 * Returns { valid: boolean, errors: string[] }.
 * @param {any} obj
 * @returns {{ valid: boolean, errors: string[] }}
 */
export function validateEntityToml(obj) {
  if (!obj || typeof obj !== 'object' || Array.isArray(obj)) {
    return { valid: false, errors: ['Root value must be an object'] };
  }
  const errors = [];
  if (!obj.tags || !Array.isArray(obj.tags) || obj.tags.length === 0) {
    errors.push('Entity must have at least one tag');
  }
  return { valid: errors.length === 0, errors };
}

/**
 * Parse every faction TOML string and return a map of uuid → name.
 *
 * @param {Array<{name: string, content: string}>} factionFiles
 *   Each element is { name: 'federation.toml', content: '<toml text>' }.
 * @returns {Map<string, string>} uuid → faction name
 */
export function buildFactionMap(factionFiles) {
  const map = new Map();
  for (const { content } of factionFiles) {
    try {
      const parsed = parse(content);
      if (parsed.uuid && parsed.name) {
        map.set(String(parsed.uuid), String(parsed.name));
      }
    } catch {
      // skip malformed faction files
    }
  }
  return map;
}

/**
 * Parse every complexity TOML filename and return a sorted array of path strings.
 * The paths are in the form 'assets/complexity/<filename>'.
 *
 * @param {string[]} complexityFilenames  e.g. ['tactical.toml', 'power.toml']
 * @returns {string[]}
 */
export function buildComplexityPaths(complexityFilenames) {
  return complexityFilenames
    .map((f) => `assets/complexity/${f}`)
    .sort();
}

/**
 * Validate that an entity config object with effects also has a shape.
 * Returns { valid: boolean, errors: string[] }.
 * @param {object} obj
 * @returns {{ valid: boolean, errors: string[] }}
 */
export function validateEntitySections(obj) {
  const errors = [];
  if (obj.effects && Object.keys(obj.effects).length > 0 && !obj.shape) {
    errors.push('region entity has effects but no [shape] section');
  }
  return { valid: errors.length === 0, errors };
}
