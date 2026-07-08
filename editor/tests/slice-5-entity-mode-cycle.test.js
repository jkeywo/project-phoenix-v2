import { describe, it, expect, beforeEach } from 'vitest';
import { setupEntityMode } from './slice-5-helpers.js';

/**
 * Slice 5 integration test: full Entity Mode cycle.
 *
 * Mount → file list populated → load pirate_raider.toml → cards rendered
 * for tags/hull/collider/helm_console/weapons_console/radar_appearance/mesh/
 * behaviour/faction → edit hull.hull_integrity → saveFlow.setContent called +
 * isDirty → undo → value restored → save → writeFile called + dirty cleared.
 */

describe('Slice 5: Entity Mode full cycle', () => {
  let ctx;
  beforeEach(async () => {
    ctx = await setupEntityMode();
  });

  it('populates the file list with both entity TOMLs', () => {
    const rows = ctx.host.querySelectorAll('div')
      .filter((d) => d.classList.contains('entity-file-list-row'));
    expect(rows.length).toBe(2);
    const labels = rows.map((r) => r.dataset.path).sort();
    expect(labels).toEqual([
      'assets/entities/alliance_battleship.toml',
      'assets/entities/pirate_raider.toml',
    ]);
  });

  it('opens pirate_raider.toml and renders cards for all top-level sections', async () => {
    await ctx.view._internal.loadEntity('assets/entities/pirate_raider.toml');

    const cards = ctx.view.shell.getComponentCards().map((c) => c.section);
    expect(cards).toEqual(expect.arrayContaining([
      'tags', 'faction', 'hull', 'collider',
      'helm_console', 'weapons_console',
      'radar_appearance', 'mesh', 'behaviour',
    ]));
  });

  it('editing hull.hull_integrity updates parsed + saveFlow + marks dirty + snapshots undo', async () => {
    await ctx.view._internal.loadEntity('assets/entities/pirate_raider.toml');

    const hull = ctx.view.shell.getCard('hull');
    expect(hull.data.hull_integrity).toBe(30);

    ctx.view._internal.handleCardEdit('hull', { hull_integrity: 99.5 });

    expect(ctx.view.shell.getParsedEntity().hull.hull_integrity).toBe(99.5);
    const path = 'assets/entities/pirate_raider.toml';
    expect(ctx.modeShell.isDirty('Entity', path)).toBe(true);

    const stash = ctx.saveFlow._contentCache.Entity[path];
    expect(stash.hull.hull_integrity).toBe(99.5);

    const undoEntries = ctx.modeShell.getUndoHistory('Entity', path);
    expect(undoEntries.length).toBe(1);
    expect(undoEntries[0].hull.hull_integrity).toBe(30);
  });

  it('undo restores the previous hull.hull_integrity value', async () => {
    await ctx.view._internal.loadEntity('assets/entities/pirate_raider.toml');
    ctx.view._internal.handleCardEdit('hull', { hull_integrity: 99.5 });

    const restoreCb = ctx.getRestoreCb();
    expect(restoreCb).toBeDefined();
    restoreCb(ctx.modeShell, 'assets/entities/pirate_raider.toml', 'undo');

    expect(ctx.view.shell.getParsedEntity().hull.hull_integrity).toBe(30);
  });

  it('save writes the file and clears dirty', async () => {
    await ctx.view._internal.loadEntity('assets/entities/pirate_raider.toml');
    ctx.view._internal.handleCardEdit('hull', { hull_integrity: 99.5 });

    const path = 'assets/entities/pirate_raider.toml';
    ctx.modeShell.setActiveFile('Entity', path);
    const result = await ctx.saveFlow.saveActive();

    expect(result.ok).toBe(true);
    expect(ctx.writeFileCalls.length).toBe(1);
    expect(ctx.writeFileCalls[0].path).toBe(path);
    expect(ctx.writeFileCalls[0].content).toContain('hull_integrity');
    expect(ctx.writeFileCalls[0].content).toContain('99.5');
    expect(ctx.modeShell.isDirty('Entity', path)).toBe(false);
  });
});
