import { describe, expect, it } from 'vitest';
import { createDockLayoutHistory } from '../../gui/dock-layout-history.js';
import { PANEL_KIND } from '../../gui/dock-layout-model.js';

const group = tabs => ({ type: 'tabs', tabs, active: tabs[0] });
const state = (version, tabs = ['base'], extra = {}) => ({
  version, root: group(tabs), floats: [], closed: [], selected: tabs[0], ...extra,
});
const records = [
  { id: 'base', since: 1, kind: PANEL_KIND.DOCUMENT },
  { id: 'last', since: 2, migration: { target: 'first', order: 2 } },
  { id: 'first', since: 2, migration: { target: 'base', order: 1 } },
  { id: 'draft', since: 2 },
  { id: 'retired', since: 1, retired: 3 },
];
const history = options => createDockLayoutHistory({
  version: 3, panels: records, defaultLayout: version => state(version), ...options,
});

describe('dock layout history', () => {
  it('keeps registry order separate from placement order and leaves unplaced panels closed', () => {
    const { registry, current, migrate } = history();
    expect(registry.map(panel => panel.id)).toEqual(['base', 'last', 'first', 'draft']);
    expect(Object.isFrozen(registry)).toBe(true);
    expect(registry.every(Object.isFrozen)).toBe(true);
    expect(current.kind('base')).toBe(PANEL_KIND.DOCUMENT);
    const result = migrate(state(1));
    expect(result.root.tabs).toEqual(['base', 'first', 'last']);
    expect(result.root.active).toBe('base');
    expect(result.selected).toBe('base');
    expect(result.closed).toEqual(['draft']);
  });
  it('does not accept future placements or reopen explicitly closed existing panels', () => {
    const { migrate } = history();
    expect(migrate(state(1, ['base', 'last'])).root.tabs).toEqual(['base', 'first', 'last']);
    const result = migrate(state(2, ['base', 'first'], { closed: ['last'] }));
    expect(result.root.tabs).toEqual(['base', 'first']);
    expect(result.closed).toContain('last');
  });
  it('retires panels and repairs their active and selected identities', () => {
    const { migrate } = history();
    const result = migrate(state(2, ['retired', 'base'], { selected: 'retired' }));
    expect(result.root.tabs).toEqual(['base']);
    expect(result.root.active).toBe('base');
    expect(result.selected).toBe('base');
    expect(result.closed).not.toContain('retired');
  });
  it('uses context policies for historical floating drafts and current pinned repair', () => {
    const panels = [{ id: 'base', since: 1 }, { id: 'draft', since: 1 }, { id: 'pin', since: 1 }];
    const { migrate } = history({ panels, policy: version => ({
      temporary: version === 1 ? ['draft'] : [], pinned: version === 3 ? ['pin'] : [],
    }) });
    const floating = [{ panel: 'draft', x: 20, y: 30, width: 300, height: 200 }];
    expect(migrate(state(1, ['base'], { floats: floating })).floats).toEqual([]);
    expect(migrate(state(2, ['base'], { floats: floating })).floats).toEqual(floating);
    expect(migrate(state(1)).root.tabs).toEqual(['base', 'pin']);
  });
  it('shares aliased vocabularies, defaults and the model passed to context rehoming', () => {
    const panels = [{ id: 'base', since: 2 }, { id: 'new', since: 3 }];
    const seen = [];
    const { migrate, current } = history({ panels, aliases: { 1: 2 },
      rehome: (value, from, input, generation) => { seen.push(generation.model); return value; },
    });
    expect(migrate({ version: 1 })).toEqual(migrate({ version: 2 }));
    expect(seen[0]).toBe(seen[1]);
    expect(seen[0].normalize({ version: 1 })).toEqual(state(2));
    expect(migrate({ version: 99 })).toEqual(current.defaultLayout());
    const fresh = current.defaultLayout();
    fresh.root.tabs.push('new');
    expect(current.defaultLayout()).toEqual(state(3));
  });
});
