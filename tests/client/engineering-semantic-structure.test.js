import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

const root = process.cwd();
const source = (relative) => fs.readFileSync(path.join(root, relative), 'utf8');

describe('Engineering-family visible-control semantic structure', () => {
  for (const [file, actions] of [
    ['gui/components/ph-power-controls.js', ['POWER_DECREASE_ACTION_ID', 'POWER_INCREASE_ACTION_ID']],
    ['gui/components/ph-repair-teams.js', [
      'REPAIR_DISPATCH_ACTION_ID', 'REPAIR_PRIORITY_ACTION_ID', 'REPAIR_RECALL_ACTION_ID',
    ]],
    ['gui/battleship/repair.html', ['EXTERNAL_REPAIR_TOGGLE_ACTION_ID']],
    ['gui/destroyer/engineering.html', [
      'TRACTOR_TOGGLE_ACTION_ID', 'UMBILICAL_TOGGLE_ACTION_ID', 'EXTERNAL_REPAIR_TOGGLE_ACTION_ID',
    ]],
    // The cruiser mounts the same two Operations controls since #1390, and
    // reaches them the same way. It authors no `[repair.external_dispatch]`,
    // so it carries no Field Repair control to route.
    ['gui/cruiser/engineering.html', [
      'TRACTOR_TOGGLE_ACTION_ID', 'UMBILICAL_TOGGLE_ACTION_ID',
    ]],
  ]) {
    it(`${file} routes its visible controls through semantic identities`, () => {
      const text = source(file);
      expect(text).toContain('activateEngineeringAction');
      for (const action of actions) expect(text).toContain(action);
    });
  }

  it('layers Engineering-family adapters onto Captain rather than replacing them', () => {
    const text = source('gui/console-core.js');
    expect(text).toContain('registerCaptainActions');
    expect(text).toContain('registerEngineeringActions');
    expect(text).toContain(
      'ENGINEERING_ACTION_REGISTRATION_CONTEXTS.some((context) => _actionContexts.has(context))',
    );
  });

  it('mounts the shared Power/Repair controls in every shipped variant', () => {
    for (const file of [
      'gui/battleship/power.html',
      'gui/battleship/repair.html',
      'gui/cruiser/engineering.html',
      'gui/destroyer/engineering.html',
      'gui/courier/captain.html',
    ]) {
      const text = source(file);
      expect(text).toMatch(/ph-(power-controls|repair-teams)/);
      expect(text).toContain('initConsole');
    }
  });

  it('uses correlated envelopes for every migrated authoritative command', () => {
    const text = source('gui/action-map.js');
    for (const action of [
      'engage_tractor:', 'release_tractor:', 'dispatch_external_repair:',
      'recall_external_repair:', 'start_transfer:', 'stop_transfer:',
      'dispatch_repair_team:', 'recall_repair_team:', 'set_repair_target_priority:',
      'set_power:',
    ]) {
      const start = text.indexOf(action);
      expect(start).toBeGreaterThan(-1);
      expect(text.slice(start, start + 900)).toContain('ControlSystemCorrelated');
    }
  });
});
