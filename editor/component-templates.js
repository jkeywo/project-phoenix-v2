import { COMPONENT_SCHEMA } from './component-schema.js';

/**
 * Build a default data value for a section from its schema fields.
 *
 * Returns a bare scalar / array for top-level scalar sections (where the
 * single field's key matches the section name — e.g. `name = "Sun"`,
 * `tags = ["..."]`, `faction = "uuid"`) so the data shape matches what TOML
 * parsing produces. Returns an empty array `[]` for `arrayOfTables` sections
 * (e.g. `[[light]]` → bare array of entry objects). Otherwise returns an
 * object keyed by field name with each field's `default` value (optional
 * fields without a default are omitted).
 *
 * @param {string} sectionKey
 * @returns {any|null} defaults value, or null if the section is unknown
 */
export function getRawSectionDefaults(sectionKey) {
  const schema = COMPONENT_SCHEMA[sectionKey];
  if (!schema) return null;

  // Array-of-tables top-level section (e.g. light → [[light]]). Card data
  // is a bare array of entry objects.
  if (schema.arrayOfTables) return [];

  // Top-level scalar/array section: section name === single field's key.
  // The runtime data is the bare value, not a wrapping object.
  if (
    schema.fields.length === 1 &&
    schema.fields[0].key === sectionKey
  ) {
    const f = schema.fields[0];
    if ('default' in f) {
      // Defensive clone for arrays/objects so callers can mutate freely.
      const d = f.default;
      if (Array.isArray(d)) return [...d];
      if (d && typeof d === 'object') return { ...d };
      return d;
    }
    return undefined;
  }

  const obj = {};
  for (const field of schema.fields) {
    if ('default' in field) {
      obj[field.key] = field.default;
    }
    // optional fields without a default are not included
  }
  return obj;
}
