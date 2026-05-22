import { describe, it, expect, beforeEach } from 'vitest';
import { ModeShell } from '../mode-shell.js';
import { analyzeAnchorRename } from '../anchor-rename.js';
import { createUndoController } from '../undo-controller.js';

/**
 * End-to-end integration: cross-layer anchor rename.
 *
 * Layer A defines anchor "X"; Layer B has an entity that references "X".
 * The renamer must:
 *   - allow the rename (no collision)
 *   - report a cross-layer reference
 *   - rewrite the anchor key in A and the entity's anchor field in B
 *   - snapshot BOTH layers before mutation (Slice 1 contract)
 */
describe('anchor rename integration (cross-layer)', () => {
  let modeShell;
  let snapshotForUndo;
  let layerA;
  let layerB;
  let layerManager;

  beforeEach(() => {
    modeShell = new ModeShell();
    snapshotForUndo = createUndoController({ modeShell }).snapshotForUndo;
    layerA = {
      filename: 'worlds/a.toml',
      toml: {
        anchors: { X: [10, 0, 20] },
      },
      isDirty: false,
    };
    layerB = {
      filename: 'worlds/b.toml',
      toml: {
        entity: [{ name: 'patrol', template_path: 'entities/raider.toml', transform: { anchor: 'X' } }],
      },
      isDirty: false,
    };
    layerManager = {
      getLayers: () => [layerA, layerB],
    };
  });

  it('rewrites the anchor key and the cross-layer reference, snapshotting both layers', () => {
    const v2Layers = layerManager.getLayers().map(l => ({ path: l.filename, worldState: l.toml }));

    const result = analyzeAnchorRename('X', 'Y', v2Layers);
    expect(result.allowed).toBe(true);
    expect(result.rewritePairs).toEqual([{ layerPath: 'worlds/a.toml', newAnchorValue: 'Y' }]);
    expect(result.crossLayerReferences).toEqual([
      { layerPath: 'worlds/b.toml', entityName: 'patrol', field: 'anchor' },
    ]);
    expect(result.inLayerReferences).toEqual([]);

    // Apply rewrites — snapshot BEFORE mutation per Slice 1 contract.
    snapshotForUndo(layerA);
    layerA.toml.anchors.Y = layerA.toml.anchors.X;
    delete layerA.toml.anchors.X;
    layerA.isDirty = true;

    snapshotForUndo(layerB);
    for (const ent of layerB.toml.entity) {
      if (ent.transform && ent.transform.anchor === 'X') ent.transform.anchor = 'Y';
    }
    layerB.isDirty = true;

    // Owner layer: anchor key was renamed.
    expect(layerA.toml.anchors.Y).toEqual([10, 0, 20]);
    expect(layerA.toml.anchors.X).toBeUndefined();
    // Cross layer: entity's anchor was rewritten.
    expect(layerB.toml.entity[0].transform.anchor).toBe('Y');

    // Both layers were snapshotted onto the World undo stack.
    const undoEntryA = modeShell.swapUndoActive('World', 'worlds/a.toml', layerA.toml);
    expect(undoEntryA).toBeTruthy();
    expect(undoEntryA.anchors.X).toEqual([10, 0, 20]);
    expect(undoEntryA.anchors.Y).toBeUndefined();

    const undoEntryB = modeShell.swapUndoActive('World', 'worlds/b.toml', layerB.toml);
    expect(undoEntryB).toBeTruthy();
    expect(undoEntryB.entity[0].transform.anchor).toBe('X');

    // Both layers marked dirty.
    expect(layerA.isDirty).toBe(true);
    expect(layerB.isDirty).toBe(true);
  });
});
