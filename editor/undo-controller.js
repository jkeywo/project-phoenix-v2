/**
 * Undo controller.
 *
 * Contract: snapshot-BEFORE-mutation.
 *
 *   snapshotForUndo(layer);   // records layer.toml as the PRE-mutation state
 *   layer.toml.something = newValue;  // mutate
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
 * `restoreScenarioLayer` is exported as a pure function so the integration
 * test can drive it without DOM, Konva, or window globals.
 */

/**
 * Push the current (pre-mutation) layer state onto the Scenario undo stack.
 * Callers MUST invoke this BEFORE mutating `layer.toml`.
 *
 * @param {{ filename: string, toml: object }} layer
 */
export function snapshotForUndo(layer) {
  if (!layer || !layer.filename) return;
  const editorV2 = (typeof window !== 'undefined') ? window.__editorV2 : null;
  if (!editorV2 || !editorV2.modeShell) return;
  try {
    const snapshot = structuredClone(layer.toml);
    editorV2.modeShell.pushUndoEntry('Scenario', layer.filename, snapshot);
    editorV2.modeShell.markDirty('Scenario', layer.filename, true);
  } catch (err) {
    console.warn('[undo-controller] snapshot failed:', err?.message || err);
  }
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
export function restoreScenarioLayer(layerManager, path, snapshot) {
  if (!layerManager || !path) return null;
  const layers = layerManager.getLayers();
  const layer = layers.find((l) => l.filename === path);
  if (!layer) return null;
  layer.toml = snapshot;
  layer.isDirty = true;
  return layer;
}
