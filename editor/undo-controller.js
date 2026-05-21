/**
 * Slice 1 undo helper. Snapshots a layer's parsed TOML onto the Scenario
 * mode's undo stack before a mutation is applied. The actual restoration is
 * gated on per-mode wiring (Slices 2/3/5/6); for now the entry is parked on
 * the stack so Cmd/Ctrl+Z surfaces it.
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
