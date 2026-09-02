import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

function source(path) {
  return readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');
}

const HELM_VARIANTS = [
  ['battleship', false, false],
  ['cruiser', true, false],
  ['destroyer', true, true],
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
  for (const [hull, lateral, dock] of HELM_VARIANTS) {
    it(`${hull} mounts its complete shipped Helm variant`, () => {
      const html = source(`gui/${hull}/helm.html`);
      expect(html).toContain(`initConsole({ name: 'helm'`);
      for (const control of [
        '<ph-helm-radar', '<ph-helm-joystick', '<ph-impulse-btn', '<ph-boost-btn',
      ]) expect(html).toContain(control);
      expect(html.includes('<ph-lateral-thrust-joystick')).toBe(lateral);
      expect(html.includes('id="dock-btn"')).toBe(dock);
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

  it('Destroyer dock uses the semantic identity and no direct action-map call', () => {
    const html = source('gui/destroyer/helm.html');
    expect(html).toContain('HELM_DOCK_ACTION_ID');
    expect(html).toContain('activateSemanticAction');
    expect(html).not.toContain('consoleHandle.sendAction');
  });

  it('registers the complete Helm family once in each console and the private catalogue', () => {
    const consoleCore = source('gui/console-core.js');
    expect(consoleCore).toContain('_actionContexts.has(HELM_ACTION_CONTEXT)');
    expect(consoleCore).toContain('registerHelmActions(_semanticActions');
    expect(source('gui/client-semantic-actions.js')).toContain('...HELM_ACTIONS');
  });
});
