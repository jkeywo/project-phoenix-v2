import { describe, it, expect, beforeEach } from 'vitest';
import { ModeShell } from '../mode-shell.js';
import { createUndoController, restoreWorldLayer } from '../undo-controller.js';

/**
 * End-to-end undo/redo for the World mode. Exercises the contract
 * documented in `undo-controller.js`:
 *
 *   1. snapshot-BEFORE-mutation
 *   2. Cmd+Z: pop pre-mutation snapshot, push CURRENT (post-mutation) onto redo
 *   3. Cmd+Shift+Z: pop post-mutation snapshot, push CURRENT (pre-mutation) onto undo
 *
 * Uses real ModeShell + UndoStack and a minimal mock layerManager so the
 * test stays a true integration of the undo/restore wiring without dragging
 * DOM, Konva or window globals in.
 */
describe('undo integration (world mode)', () => {
  let modeShell;
  let undoController;
  let snapshotForUndo;
  let layer;
  let layerManager;

  beforeEach(() => {
    modeShell = new ModeShell();
    undoController = createUndoController({ modeShell });
    snapshotForUndo = undoController.snapshotForUndo;
    layer = {
      filename: 'worlds/test.toml',
      toml: { name: 'original', entity: [{ name: 'sun' }] },
      isDirty: false,
    };
    layerManager = {
      getLayers: () => [layer],
    };
  });

  // Mirror of scenario-mode.js's registerWorldUndoRestore callback. This
  // lets the test exercise the exact swap-then-restore flow that the
  // keydown handler triggers in production.
  function performUndo(mode, path) {
    const layerObj = layerManager.getLayers().find((l) => l.filename === path);
    if (!layerObj) return null;
    const current = structuredClone(layerObj.toml);
    const snapshot = modeShell.swapUndoActive(mode, path, current);
    if (!snapshot) return null;
    restoreWorldLayer(layerManager, path, snapshot);
    return snapshot;
  }

  function performRedo(mode, path) {
    const layerObj = layerManager.getLayers().find((l) => l.filename === path);
    if (!layerObj) return null;
    const current = structuredClone(layerObj.toml);
    const snapshot = modeShell.swapRedoActive(mode, path, current);
    if (!snapshot) return null;
    restoreWorldLayer(layerManager, path, snapshot);
    return snapshot;
  }

  it('undo restores the pre-mutation state', () => {
    // Pre-edit value.
    expect(layer.toml.name).toBe('original');

    // Snapshot BEFORE mutating.
    snapshotForUndo(layer);
    layer.toml.name = 'edited';

    expect(layer.toml.name).toBe('edited');

    performUndo('World', layer.filename);

    expect(layer.toml.name).toBe('original');
  });

  it('redo restores the post-mutation state after undo', () => {
    snapshotForUndo(layer);
    layer.toml.name = 'edited';

    performUndo('World', layer.filename);
    expect(layer.toml.name).toBe('original');

    performRedo('World', layer.filename);
    expect(layer.toml.name).toBe('edited');
  });

  it('multiple undos walk back through the history in reverse order', () => {
    snapshotForUndo(layer);
    layer.toml.name = 'step1';

    snapshotForUndo(layer);
    layer.toml.name = 'step2';

    snapshotForUndo(layer);
    layer.toml.name = 'step3';

    performUndo('World', layer.filename);
    expect(layer.toml.name).toBe('step2');

    performUndo('World', layer.filename);
    expect(layer.toml.name).toBe('step1');

    performUndo('World', layer.filename);
    expect(layer.toml.name).toBe('original');
  });

  it('redo after multiple undos walks forward again', () => {
    snapshotForUndo(layer);
    layer.toml.name = 'step1';
    snapshotForUndo(layer);
    layer.toml.name = 'step2';

    performUndo('World', layer.filename);
    performUndo('World', layer.filename);
    expect(layer.toml.name).toBe('original');

    performRedo('World', layer.filename);
    expect(layer.toml.name).toBe('step1');

    performRedo('World', layer.filename);
    expect(layer.toml.name).toBe('step2');
  });

  it('undo on empty stack is a no-op and does not corrupt layer state', () => {
    const before = layer.toml;
    const result = performUndo('World', layer.filename);
    expect(result).toBeNull();
    expect(layer.toml).toBe(before);
  });

  it('snapshots are independent of subsequent in-place edits', () => {
    snapshotForUndo(layer);
    // Mutate a nested object in-place.
    layer.toml.entity[0].name = 'moon';

    performUndo('World', layer.filename);
    expect(layer.toml.entity[0].name).toBe('sun');
  });

  it('restoreWorldLayer returns null when no layer matches the path', () => {
    const result = restoreWorldLayer(layerManager, 'unknown.toml', {});
    expect(result).toBeNull();
  });

  it('a new edit after undo clears the redo branch', () => {
    snapshotForUndo(layer);
    layer.toml.name = 'edited';
    performUndo('World', layer.filename);
    expect(modeShell.canRedoActive('World', layer.filename)).toBe(true);

    // New edit while in the "undone" state.
    snapshotForUndo(layer);
    layer.toml.name = 'new-branch';

    expect(modeShell.canRedoActive('World', layer.filename)).toBe(false);
  });
});
