import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const HULLS = ['battleship', 'cruiser', 'destroyer', 'courier'];
const COMPONENTS = [
  ['gui/components/ph-red-alert.js', [
    'CAPTAIN_RED_ALERT_ACTION_ID',
    'CAPTAIN_WEAPONS_HOLD_ACTION_ID',
  ], ["sendAction('set_red_alert'", "sendAction('set_weapons_hold'"]],
  ['gui/components/ph-camera-select.js', [
    'CAPTAIN_VIEW_ACTION_ID',
  ], ["sendAction('set_view'"]],
  ['gui/components/ph-objective-list.js', [
    'CAPTAIN_OBJECTIVE_PRIORITY_ACTION_ID',
  ], ["sendAction('set_objective_priority'"]],
];

function source(path) {
  return readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');
}

describe('Captain semantic-action structural coverage', () => {
  for (const hull of HULLS) {
    it(`${hull} mounts every shared Captain command component`, () => {
      const html = source(`gui/${hull}/captain.html`);
      expect(html).toContain('<ph-red-alert');
      expect(html).toContain('<ph-camera-select');
      expect(html).toContain('<ph-objective-list');
      expect(html).toContain(`initConsole({ name: 'captain'`);
    });
  }

  for (const [path, semanticIds, forbiddenDirectCalls] of COMPONENTS) {
    it(`${path} activates semantic identities and cannot bypass them`, () => {
      const text = source(path);
      expect(text).toContain('activateSemanticAction');
      for (const id of semanticIds) expect(text).toContain(id);
      for (const directCall of forbiddenDirectCalls) expect(text).not.toContain(directCall);
    });
  }
});
