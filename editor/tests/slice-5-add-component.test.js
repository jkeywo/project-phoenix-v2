import { describe, it, expect, beforeEach } from 'vitest';
import { setupEntityMode, fireClick } from './slice-5-helpers.js';

/**
 * Slice 5 integration test: + Add Component → Region combo.
 *
 * - Open pirate_raider.toml.
 * - Click + Add Component → submenu opens.
 * - Click Region combo.
 * - Assert that the `shape` and `effects` cards now exist (new sections).
 *   `tags` already existed on pirate_raider, so the Region combo's tags
 *   section is skipped — verify the skip-warning surfaces via addCombo's
 *   return shape and that the existing tags section is left untouched.
 */

describe('Slice 5: + Add Component → Region combo', () => {
  let ctx;
  beforeEach(async () => {
    ctx = await setupEntityMode();
    await ctx.view._internal.loadEntity('assets/entities/pirate_raider.toml');
  });

  it('opens the picker when + Add Component is clicked', () => {
    const addBtn = ctx.host.querySelectorAll('button')
      .find((b) => b.classList.contains('entity-add-component-btn'));
    expect(addBtn).toBeDefined();
    fireClick(addBtn);

    const combos = ctx.host.querySelectorAll('button')
      .filter((b) => b.classList.contains('entity-add-menu-combo'));
    expect(combos.length).toBeGreaterThan(0);
    const regionBtn = combos.find((b) => b.dataset.combo === 'Region');
    expect(regionBtn).toBeDefined();
  });

  it('selecting Region adds shape + effects cards (and skips existing tags with a warning)', () => {
    // Capture warn() to assert the skip warning surfaces.
    const warnings = [];
    const origWarn = console.warn;
    console.warn = (msg) => { warnings.push(String(msg)); };

    try {
      // Drive the add directly via the internal handler — equivalent to
      // clicking + Add Component → Region.
      ctx.view._internal.handleAddChoice({ kind: 'combo', name: 'Region' });
    } finally {
      console.warn = origWarn;
    }

    const sections = ctx.view.shell.getComponentCards().map((c) => c.section);

    // shape + effects are newly added.
    expect(sections).toContain('shape');
    expect(sections).toContain('effects');

    // tags was already present and not duplicated.
    const tagsCount = sections.filter((s) => s === 'tags').length;
    expect(tagsCount).toBe(1);

    // Original tags values are preserved (not overwritten with ['region']).
    const tagsCard = ctx.view.shell.getCard('tags');
    expect(tagsCard.data).toEqual(['ship', 'npc', 'enemy']);

    // Skip warning surfaced.
    expect(warnings.some((w) => /tags.*already present/i.test(w))).toBe(true);

    // Dirty + undo snapshot recorded.
    const path = 'assets/entities/pirate_raider.toml';
    expect(ctx.modeShell.isDirty('Entity', path)).toBe(true);
    const undo = ctx.modeShell.getUndoHistory('Entity', path);
    expect(undo.length).toBe(1);
    // Snapshot is pre-mutation, so shape/effects shouldn't be in it.
    expect(undo[0].shape).toBeUndefined();
    expect(undo[0].effects).toBeUndefined();
  });
});
