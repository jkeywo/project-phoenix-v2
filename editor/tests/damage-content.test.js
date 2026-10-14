import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { parse } from 'smol-toml';
import { resolveTemplate } from '../entity-includes.js';
import { authoredAndGeneratedSystems } from '../workshop-generated-systems.js';
import { validateInterior } from '../workshop-interiors.js';
import { validateDamage } from '../workshop-damage-model.js';
const root = path.resolve(import.meta.dirname, '../..');

describe('shipped component damage migration', () => {
  it('authors physical coverage, finite parts and conserved baseline structure on every equipped template', () => {
    const baselines = JSON.parse(fs.readFileSync(path.join(root, 'editor/tests/damage-content-baseline.json'), 'utf8'));
    let systems = 0, components = 0;
    for (const baseline of baselines) {
      const resolved = resolveTemplate(baseline.file, { read: file => fs.existsSync(path.join(root, file)) ? fs.readFileSync(path.join(root, file), 'utf8') : null }, parse);
      expect(resolved.ok, baseline.file).toBe(true); const value = resolved.resolved.value, fitted = authoredAndGeneratedSystems(value);
      expect(() => validateInterior(value.interior, fitted, value.repair?.repair_team_count || 0), baseline.file).not.toThrow();
      expect(() => validateDamage(value.damage, fitted, value), baseline.file).not.toThrow();
      expect(value.hull, baseline.file).toBeUndefined(); expect(value.interior.repair_placements).toBeUndefined();
      expect(value.interior.compartment.reduce((sum, room) => sum + room.max_hp, 0)).toBeCloseTo(baseline.baseline_hp, 5);
      for (const weapon of value.weapon || []) expect(weapon.impact).toMatchObject({ damage_type: weapon.delivery.kind === 'beam' ? 'laser' : 'physical' });
      for (const resource of ['structural_parts', 'machinery_parts']) {
        const stock = value.infrastructure.capacity.find(stock => stock.id === value.resource_services[resource].id);
        expect(stock.amount).toBeGreaterThan(0); expect(stock.amount).toBeLessThanOrEqual(stock.ceiling);
        if (value.umbilical) expect(value.umbilical.allowed_resources).toContain(resource);
      }
      for (const arc of value.shield_arc || []) {
        expect(arc.hull_max_hp).toBeUndefined(); expect(arc.damage_component.length).toBe(3);
        expect(value.system.some(system => system.id === `shield-arc-${arc.id}`)).toBe(false);
      }
      systems += fitted.length; components += fitted.reduce((sum, system) => sum + system.damage_component.length, 0);
    }
    expect(baselines).toHaveLength(21); expect(systems).toBe(305); expect(components).toBe(753);
  });
});
