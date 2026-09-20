import { describe, expect, it } from 'vitest';
import { readFile } from 'node:fs/promises';
import { legacyWorkshopUrl, parseWorkshopLaunch } from '../workshop-launch.js';
import { workshopLaunchQuery } from '../../scripts/dev-workshop.mjs';

describe('retired shell Workshop links', () => {
  it('maps the complete viewer subject vocabulary into one Workshop descriptor', () => {
    const model = legacyWorkshopUrl(new URL('http://localhost/viewer.html?model=assets/models/ship.glb&variant=damaged&lighting=directional&gizmos=1'), 'viewer');
    expect(parseWorkshopLaunch(new URL(model).hash.slice(1))).toEqual({ version: 1, panel: 'model-preview', file: null,
      preview: { model: 'assets/models/ship.glb', variant: 'damaged' }, controls: { lighting: 'directional', gizmos: true } });
    const entity = legacyWorkshopUrl(new URL('http://localhost/viewer.html?entity=assets/entities/sol.toml'), 'viewer');
    expect(parseWorkshopLaunch(new URL(entity).hash.slice(1)).preview).toEqual({ entity: 'assets/entities/sol.toml' });
  });

  it('maps editor files and drops traversal or unknown capabilities', () => {
    expect(parseWorkshopLaunch(new URL(legacyWorkshopUrl(new URL('https://host/tools/editor.html?file=assets/worlds/demo.toml'), 'editor')).hash.slice(1)).file)
      .toBe('assets/worlds/demo.toml');
    expect(parseWorkshopLaunch('panel=admin&file=../secret&model=C:/secret.glb')).toBeNull();
  });

  it('keeps the direct model command on the same strict launch vocabulary', () => {
    expect(parseWorkshopLaunch(workshopLaunchQuery(['--models', '--model=assets/models/ship.glb',
      '--variant=damaged', '--lighting=ambient', '--gizmos=0']))).toEqual({
      version: 1, panel: 'model-preview', file: null,
      preview: { model: 'assets/models/ship.glb', variant: 'damaged' },
      controls: { lighting: 'ambient', gizmos: false },
    });
  });

  it('accepts only exact npm selectors and the non-expanded Windows environment query', () => {
    expect(parseWorkshopLaunch(workshopLaunchQuery(['--models'],
      'model=assets%2Fmodels%2Fship.glb&lighting=directional')).preview)
      .toEqual({ model: 'assets/models/ship.glb', variant: null });
    expect(parseWorkshopLaunch(workshopLaunchQuery([], 'file=assets%2Fworlds%2Fdemo.toml')).file)
      .toBe('assets/worlds/demo.toml');
    for (const args of [['--wat=1'], ['--model=../ship.glb'], ['--gizmos=true'],
      ['--model=assets/models/a.glb', '--entity=assets/entities/a.toml'],
      ['--model=assets/models/a.glb', '--model=assets/models/b.glb']]) {
      expect(() => workshopLaunchQuery(args)).toThrow(/Invalid|Choose|Duplicate/);
    }
    expect(() => workshopLaunchQuery([], 'file=../secret')).toThrow(/Invalid/);
  });

  it('keeps both batch launchers free of cmd-expanded arguments', async () => {
    for (const file of ['start-editor.bat', 'start-viewer.bat']) {
      const source = await readFile(new URL(`../../${file}`, import.meta.url), 'utf8');
      expect(source).not.toContain('%*');
      expect(source).toContain('PHOENIX_WORKSHOP_OPEN');
      expect(source).toContain('scripts\\dev-workshop.mjs');
    }
  });
});
