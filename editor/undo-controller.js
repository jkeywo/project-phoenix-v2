/**
 * Undo controller.
 *
 * Contract: snapshot-BEFORE-mutation.
 *
 *   const undoController = createUndoController({ modeShell });
 *   undoController.snapshotForUndo(layer);   // records layer.toml as the PRE-mutation state
 *   layer.toml.something = newValue;          // mutate
 *
 * The undo stack therefore holds *pre-mutation* states. To restore on Cmd+Z
 * we pop the pre-mutation snapshot AND push the current (post-mutation)
 * value onto the redo stack via a swap. Redo is the mirror image.
 *
 * The swap is implemented on top of `UndoStack` via the dedicated
 * `swapUndo(current)` / `swapRedo(current)` helpers on the stack — the
 * built-in `undo()`/`redo()` methods do not give us this behaviour because
 * they push the popped entry onto the opposite stack rather than the
 * caller-supplied current state.
 *
 * `restoreWorldLayer` is exported as a pure function so the integration
 * test can drive it without DOM, Konva, or window globals.
 *
 * The controller is constructed once in `scenario-mode.js` next to the
 * ModeShell + SaveFlow and threaded as a normal dependency into the
 * leaf views (canvas, sidebar, override-view).
 */

/**
 * Create an undo controller bound to a specific ModeShell.
 *
 * @param {{ modeShell: import('./mode-shell.js').ModeShell }} deps
 * @returns {{ snapshotForUndo: (layer: { filename: string, toml: object }) => void }}
 */
export function createUndoController({ modeShell } = {}) {
  if (!modeShell) {
    throw new Error('createUndoController: modeShell is required');
  }

  return {
    /**
     * Push the current (pre-mutation) layer state onto the World undo stack.
     * Callers MUST invoke this BEFORE mutating `layer.toml`.
     *
     * @param {{ filename: string, toml: object }} layer
     */
    snapshotForUndo(layer) {
      if (!layer || !layer.filename) return;
      try {
        const snapshot = structuredClone(layer.toml);
        modeShell.pushUndoEntry('World', layer.filename, snapshot);
        modeShell.markDirty('World', layer.filename, true);
      } catch (err) {
        console.warn('[undo-controller] snapshot failed:', err?.message || err);
      }
    },
  };
}

/**
 * Pure restoration helper. Finds the layer by `path` in `layerManager` and
 * sets its `toml` to `snapshot`. Returns the matched layer (for the caller
 * to drive UI re-renders) or null if no match.
 *
 * @param {{ getLayers: () => Array<{ filename: string, toml: object, isDirty?: boolean }> }} layerManager
 * @param {string} path
 * @param {object} snapshot
 * @returns {object | null}
 */
export function restoreWorldLayer(layerManager, path, snapshot) {
  if (!layerManager || !path) return null;
  const layers = layerManager.getLayers();
  const layer = layers.find((l) => l.filename === path);
  if (!layer) return null;
  layer.toml = snapshot;
  layer.isDirty = true;
  return layer;
}
