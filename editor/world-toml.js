import { parse, stringify } from 'smol-toml';

export function parseWorldToml(text) {
  return parse(text);
}

export function stringifyWorldToml(obj) {
  return stringify(obj);
}

export function validateWorldToml(obj) {
  if (!obj || typeof obj !== 'object') {
    return { valid: false, errors: ['Root value must be an object'] };
  }
  const errors = [];
  if (!obj.global || typeof obj.global !== 'object') {
    errors.push('Missing [global] section');
  }
  if (!obj.anchors || typeof obj.anchors !== 'object') {
    errors.push('Missing [anchors] section');
  }
  return { valid: errors.length === 0, errors };
}
