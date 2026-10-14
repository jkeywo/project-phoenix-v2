import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { parse } from 'smol-toml';
import { resolveTemplate } from '../entity-includes.js';
import { authoredAndGeneratedSystems } from '../workshop-generated-systems.js';
import { legacyRepairTargets, validateInterior } from '../workshop-interiors.js';
const root = path.resolve(import.meta.dirname, '../..');
const walk = directory => fs.readdirSync(path.join(root, directory), { withFileTypes: true })
  .flatMap(entry => entry.isDirectory() ? walk(directory + '/' + entry.name) : [directory + '/' + entry.name]);
describe('authored ship interiors', () => {
  it('places every actual/generated System and connects repair homes across all equipped templates', () => {
    let checked = 0, systems = 0, gates = 0;
    for (const file of walk('assets/entities').filter(file => file.endsWith('.toml') && (!file.includes('/fragments/') || file.endsWith('/composed_escort.toml')))) {
      const resolved = resolveTemplate(file, { read: file => fs.existsSync(path.join(root, file)) ? fs.readFileSync(path.join(root, file), 'utf8') : null }, parse);
      expect(resolved.ok, file).toBe(true); const value = resolved.resolved.value, fitted = authoredAndGeneratedSystems(value);
      if (!fitted.length) continue;
      expect(() => validateInterior(value.interior, fitted, value.repair?.repair_team_count || 0, legacyRepairTargets(value)), file).not.toThrow();
      for (const gate of fitted.filter(system => system.kind === 'access-gate')) {
        expect(gate.console).toBe(fitted.find(system => system.kind === 'repair').console);
        expect(value.interior.connection.some(connection => connection.boundary === gate.id), file).toBe(true);
        expect(gate.config.ai.tree[0].state[0].task[0].action.value).toEqual({ source: 'fact', name: 'seal_requested' }); gates++;
      }
      checked++; systems += fitted.length;
    }
    expect(checked).toBeGreaterThanOrEqual(21); expect(systems).toBeGreaterThanOrEqual(301); expect(gates).toBeGreaterThanOrEqual(5);
  });
});
