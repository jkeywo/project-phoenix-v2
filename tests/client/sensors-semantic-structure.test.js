import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

function source(path) {
  return readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');
}

const SENSOR_VARIANTS = [
  ['gui/battleship/sensors.html', 'sensors', ['<ph-sensor-radar', 'SENSORS_CANCEL_IMPULSE_ACTION_ID']],
  ['gui/cruiser/science.html', 'science', ['<ph-sensor-radar', '<ph-shield-facings']],
  ['gui/destroyer/captain.html', 'captain', ['<ph-sensor-radar', '<ph-scan-readout']],
  ['gui/courier/tactical.html', 'tactical', ['<ph-sensor-radar']],
];

const SHIELD_VARIANTS = [
  ['gui/battleship/shields.html', 'shields'],
  ['gui/cruiser/science.html', 'science'],
  ['gui/destroyer/engineering.html', 'engineering'],
  ['gui/courier/captain.html', 'captain'],
];

const SEMANTIC_COMPONENTS = [
  ['gui/components/ph-sensor-radar.js', [
    'SENSORS_TARGET_ACTION_ID', 'SENSORS_VIEWSCREEN_ACTION_ID',
  ], ["sendAction('set_sensors_target'", "sendAction('set_view'"]],
  ['gui/components/ph-scan-readout.js', [
    'SENSORS_SCAN_ACTION_ID',
  ], ["sendAction('scan_target'"]],
  ['gui/components/ph-shield-facings.js', [
    'SCIENCE_SHIELD_FOCUS_ACTION_ID',
  ], ["sendAction('set_shield_focus'"]],
  ['gui/components/ph-courier-radar.js', [
    'SENSORS_TARGET_ACTION_ID', "sendAction?.('set_target'",
  ], ["sendAction?.('set_sensors_target'"]],
];

describe('Sensors and Science semantic-action structural coverage', () => {
  for (const [path, context, controls] of SENSOR_VARIANTS) {
    it(`${path} mounts its complete shipped Sensors/Science variant`, () => {
      const html = source(path);
      expect(html).toMatch(new RegExp(`initConsole\\s*\\(\\s*\\{[\\s\\S]*?name:\\s*'${context}'`));
      for (const control of controls) expect(html).toContain(control);
    });
  }

  for (const [path, context] of SHIELD_VARIANTS) {
    it(`${path} mounts shared shield focus in its real console context`, () => {
      const html = source(path);
      expect(html).toContain('<ph-shield-facings');
      expect(html).toMatch(new RegExp(`initConsole\\(\\{[\\s\\S]*?name:\\s*'${context}'`));
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

  it('the one light-DOM Sensors command cannot bypass its semantic identity', () => {
    const html = source('gui/battleship/sensors.html');
    expect(html).toContain('SENSORS_CANCEL_IMPULSE_ACTION_ID');
    expect(html).toContain('activateSemanticAction');
    expect(html).not.toContain("sendAction('cancel_impulse'");
  });

  it('registers the family once at the console seam and parent private catalogue', () => {
    const consoleCore = source('gui/console-core.js');
    expect(consoleCore).toContain(
      'SENSOR_SCIENCE_ACTION_CONTEXTS.some((context) => _actionContexts.has(context))',
    );
    expect(consoleCore).toContain('registerSensorScienceActions(_semanticActions');
    expect(source('gui/client-semantic-actions.js')).toContain('...SENSOR_SCIENCE_ACTIONS');
  });

  it('no shipped Sensors selection route can mutate the client target mirror', () => {
    const selectionRoutes = [
      'client.html',
      'gui/action-map.js',
      'gui/stations/sensors-actions.js',
      'gui/components/ph-sensor-radar.js',
      'gui/components/ph-courier-radar.js',
    ];
    for (const path of selectionRoutes) {
      const text = source(path);
      expect(text, path).not.toContain('sensorsTarget');
    }

    const client = source('client.html');
    expect(client).not.toContain("action.action === 'set_sensors_target'");
    expect(client).toContain('Semantic actions deliberately have no direct-wire fallback');
  });
});
