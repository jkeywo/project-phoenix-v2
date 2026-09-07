import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

function source(path) {
  return readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');
}

// [hull, lateral-thrust pad, contextual Dock control, under-tow-load banner].
// The Dock column tracks which hulls author a `kind = "dock"` System: the
// destroyer since #1164 S11a, the cruiser since #1388. The tow-load column
// tracks which hulls author a `kind = "tractor"` System — the beam is
// Engineering's control on both, but the tow's mass penalty lands on the Helm,
// so this is the seat that has to say so: the destroyer since #1157, the
// cruiser since #1390. The battleship authors neither, so its Helm must still
// carry no dock button and no banner at all.
const HELM_VARIANTS = [
  ['battleship', false, false, false],
  ['cruiser', true, true, true],
  ['destroyer', true, true, true],
];

const SEMANTIC_COMPONENTS = [
  ['gui/components/ph-helm-joystick.js', [
    'HELM_THRUST_ACTION_ID', 'HELM_STEERING_ACTION_ID',
  ], ["sendAction('set_helm'", "sendAction('helm_input'"]],
  ['gui/components/ph-lateral-thrust-joystick.js', [
    'HELM_LATERAL_ACTION_ID',
  ], ["sendAction('set_lateral_thrust'", 'navigator.getGamepads']],
  ['gui/components/ph-impulse-btn.js', [
    'HELM_IMPULSE_ACTION_ID',
  ], ["sendAction('start_impulse_charge'", "sendAction('cancel_impulse'", 'observeGamepadButton']],
  ['gui/components/ph-boost-btn.js', [
    'HELM_BOOST_ACTION_ID',
  ], ["sendAction('set_boost'", 'observeGamepadButton']],
  ['gui/components/ph-helm-radar.js', [
    'HELM_VIEWSCREEN_ACTION_ID',
  ], ["sendAction('set_radar_view'"]],
];

describe('Helm semantic-action structural coverage', () => {
  for (const [hull, lateral, dock, towLoad] of HELM_VARIANTS) {
    it(`${hull} mounts its complete shipped Helm variant`, () => {
      const html = source(`gui/${hull}/helm.html`);
      expect(html).toContain(`initConsole({ name: 'helm'`);
      for (const control of [
        '<ph-helm-radar', '<ph-helm-joystick', '<ph-impulse-btn', '<ph-boost-btn',
      ]) expect(html).toContain(control);
      expect(html.includes('<ph-lateral-thrust-joystick')).toBe(lateral);
      expect(html.includes('id="dock-btn"')).toBe(dock);
      expect(html.includes('id="tow-load-panel"')).toBe(towLoad);
    });
  }

  for (const [path, semanticIds, forbiddenDirectCalls] of SEMANTIC_COMPONENTS) {
    it(`${path} activates semantic identities and cannot bypass them`, () => {
      const text = source(path);
      expect(text).toContain('activateSemanticAction');
      for (const id of semanticIds) expect(text).toContain(id);
      for (const directCall of forbiddenDirectCalls) expect(text).not.toContain(directCall);
    });
  }

  // Every hull that mounts the Dock control reaches it the same way: through
  // the semantic identity, never a direct action-map call.
  for (const [hull] of HELM_VARIANTS.filter(([, , dock]) => dock)) {
    it(`${hull} dock uses the semantic identity and no direct action-map call`, () => {
      const html = source(`gui/${hull}/helm.html`);
      expect(html).toContain('HELM_DOCK_ACTION_ID');
      expect(html).toContain('activateSemanticAction');
      expect(html).not.toContain('consoleHandle.sendAction');
    });
  }

  it('registers the complete Helm family once in each console and the private catalogue', () => {
    const consoleCore = source('gui/console-core.js');
    expect(consoleCore).toContain('_actionContexts.has(HELM_ACTION_CONTEXT)');
    expect(consoleCore).toContain('registerHelmActions(_semanticActions');
    expect(source('gui/client-semantic-actions.js')).toContain('...HELM_ACTIONS');
  });
});
