import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

const root = process.cwd();
const source = (relative) => fs.readFileSync(path.join(root, relative), 'utf8');

describe('Tactical visible-control semantic structure', () => {
  for (const [file, action] of [
    ['gui/components/ph-tactical-radar.js', 'TACTICAL_TARGET_ACTION_ID'],
    ['gui/components/ph-phasers-controls.js', 'TACTICAL_PHASER_FIRE_ACTION_ID'],
    ['gui/components/ph-blasters-controls.js', 'TACTICAL_BLASTER_CHARGE_ACTION_ID'],
    ['gui/components/ph-torpedo-controls.js', 'TACTICAL_TORPEDO_FIRE_ACTION_ID'],
  ]) {
    it(`${file} uses the shared Tactical dispatcher`, () => {
      const text = source(file);
      expect(text).toContain('activateTacticalAction');
      expect(text).toContain(action);
    });
  }

  it('keeps target selection out of the old optimistic client mutation', () => {
    const text = source('gui/action-map.js');
    const setTarget = text.slice(text.indexOf('set_target:'), text.indexOf('set_phaser_mode:'));
    expect(setTarget).not.toContain('weaponsTarget');
    expect(setTarget).toContain("'ControlSystemCorrelated'");
  });
});
