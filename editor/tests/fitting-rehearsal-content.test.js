import fs from 'node:fs';
import { expect, it } from 'vitest';
import { parse } from 'smol-toml';
import { resolveTemplate } from '../entity-includes.js';

it('retains the complete real probe recipe while changing only reference identity and finite room HP', () => {
  const read = path => fs.existsSync(path) ? fs.readFileSync(path, 'utf8') : null;
  const resolve = path => {
    const result = resolveTemplate(path, { read }, parse);
    expect(result.ok).toBe(true);
    return result.resolved.value;
  };
  const base = resolve('assets/entities/battery_probe.toml');
  const subjectPath = 'assets/entities/fitting_rehearsal_subject.toml';
  const expected = structuredClone(base);
  expected.name = 'entity.fitting_rehearsal_subject.name';
  expected.faction = 'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb';
  expected.behaviour = {};
  expect(expected.interior.compartment).toHaveLength(1);
  expected.interior.compartment[0].max_hp = 10000;

  // This section is atomic in the actual loader. Check the authored section,
  // not just the composed value, so missing decks/placements cannot be hidden.
  expect(parse(read(subjectPath)).interior).toEqual(expected.interior);
  expect(resolve(subjectPath)).toEqual(expected);
});
