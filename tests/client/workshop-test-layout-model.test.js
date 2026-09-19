import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import {
  defaultWorkshopTestLayout,
  normalizeWorkshopTestLayout,
  workshopTestLayoutModel,
} from '../../gui/workshop-test-layout-model.js';

describe('Workshop Test layout model', () => {
  const parity = JSON.parse(readFileSync('tests/fixtures/workshop-test-layout-migrations.json', 'utf8'));

  it('matches the native sanitizer fixture case for case', () => {
    expect(defaultWorkshopTestLayout()).toEqual(parity.default);
    for (const testCase of parity.cases) {
      expect(normalizeWorkshopTestLayout(testCase.stored), testCase.name).toEqual(testCase.expected);
    }
  });

  it('keeps controls as a tool and the viewscreen as the document', () => {
    expect(workshopTestLayoutModel.kind('test-controls')).toBe('tool');
    expect(workshopTestLayoutModel.kind('test-viewscreen')).toBe('document');
    expect(defaultWorkshopTestLayout().selected).toBe('test-viewscreen');
  });

  it('supports independent keyboard-equivalent moves, close, reopen and reset transitions', () => {
    const initial = defaultWorkshopTestLayout();
    const tabbed = workshopTestLayoutModel.dock(initial, 'test-controls', 'test-viewscreen', 'tab');
    expect(tabbed.root).toMatchObject({ type: 'tabs', tabs: ['test-viewscreen', 'test-controls'] });
    const floated = workshopTestLayoutModel.float(tabbed, 'test-controls');
    expect(floated.floats.map(entry => entry.panel)).toEqual(['test-controls']);
    const closed = workshopTestLayoutModel.close(floated, 'test-controls');
    expect(closed.closed).toContain('test-controls');
    expect(workshopTestLayoutModel.reopen(closed, 'test-controls').closed).not.toContain('test-controls');
    expect(workshopTestLayoutModel.defaultLayout()).toEqual(initial);
  });

  it('repairs invalid data and retains only placement vocabulary', () => {
    expect(normalizeWorkshopTestLayout({ version: 99, run: { tick: 8 } }))
      .toEqual(defaultWorkshopTestLayout());
    const repaired = normalizeWorkshopTestLayout({
      version: 1,
      root: { type: 'tabs', tabs: ['test-viewscreen', 'unknown'], active: 'unknown', payload: 'stale' },
      floats: [], closed: ['test-controls'], selected: 'unknown',
      selection: { world: 'stale.toml' }, run: { tick: 8 },
    });
    expect(repaired).toEqual({
      version: 1,
      root: { type: 'tabs', tabs: ['test-viewscreen'], active: 'test-viewscreen' },
      floats: [], closed: ['test-controls'], selected: 'test-viewscreen',
    });
    expect(JSON.stringify(repaired)).not.toMatch(/stale|world|tick|run|selection/);
  });
});
