import fs from 'node:fs';
import {expect, it} from 'vitest';
import {parse} from 'smol-toml';
import {COMPONENT_SCHEMA} from '../component-schema.js';
import {getRawSectionDefaults} from '../component-templates.js';
import {resolveTemplate} from '../entity-includes.js';

function classified(value, fields, prefix) {
  for (const [key, child] of Object.entries(value)) {
    const field = fields.find(field => field.key === key);
    expect(field, `${prefix}.${key} must have an authoring descriptor`).toBeDefined();
    if (field.type === 'subobject') classified(child, field.subfields, `${prefix}.${key}`);
    if (field.type === 'array-of-tables') for (const row of child) classified(row, field.entryFields, `${prefix}.${key}[]`);
  }
}

it('classifies actual included Radio media/capture/listening fields without enabling optional equipment by default', () => {
  const read = path => fs.existsSync(path) ? fs.readFileSync(path, 'utf8') : null;
  const radio = COMPONENT_SCHEMA.comms.fields.find(field => field.key === 'radio');
  for (const path of ['assets/entities/alliance_cruiser.toml', 'assets/entities/capture_cruiser.toml', 'tests/fixtures/listening-cruiser.toml']) {
    const resolved = resolveTemplate(path, {read}, parse);
    expect(resolved.ok, path).toBe(true);
    classified(resolved.resolved.value.comms.radio, radio.subfields, 'comms.radio');
  }
  expect(getRawSectionDefaults('comms')).not.toHaveProperty('radio');
  expect(radio.subfields.find(field => field.key === 'traffic').optional).toBe(true);
});
