import { parse, stringify } from 'smol-toml';

export function parseEntityToml(text) {
  return parse(text);
}

export function stringifyEntityToml(obj) {
  return stringify(obj);
}

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
