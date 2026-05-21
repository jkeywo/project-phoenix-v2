export class UndoStack {
  constructor(maxOps = 100) {
    this._maxOps = maxOps;
    this._undoStack = [];
    this._redoStack = [];
  }

  push(entry) {
    this._redoStack = [];
    this._undoStack.push(entry);
    if (this._undoStack.length > this._maxOps) {
      this._undoStack.shift();
    }
  }

  undo() {
    if (this._undoStack.length === 0) return null;
    const entry = this._undoStack.pop();
    this._redoStack.push(entry);
    return entry;
  }

  redo() {
    if (this._redoStack.length === 0) return null;
    const entry = this._redoStack.pop();
    this._undoStack.push(entry);
    return entry;
  }

  /**
   * Snapshot-before-mutation swap: pop the most recent pre-mutation entry
   * off the undo stack, push the caller-supplied *current* (post-mutation)
   * value onto the redo stack, and return the popped pre-mutation entry.
   *
   * Returns null when the undo stack is empty (no swap is performed).
   */
  swapUndo(currentValue) {
    if (this._undoStack.length === 0) return null;
    const entry = this._undoStack.pop();
    this._redoStack.push(currentValue);
    return entry;
  }

  /**
   * Inverse of `swapUndo`: pop from redo, push caller's current value back
   * onto the undo stack, return the popped entry.
   */
  swapRedo(currentValue) {
    if (this._redoStack.length === 0) return null;
    const entry = this._redoStack.pop();
    this._undoStack.push(currentValue);
    return entry;
  }

  clear() {
    this._undoStack = [];
    this._redoStack = [];
  }

  getUndoCount() {
    return this._undoStack.length;
  }

  getRedoCount() {
    return this._redoStack.length;
  }

  canUndo() {
    return this._undoStack.length > 0;
  }

  canRedo() {
    return this._redoStack.length > 0;
  }
}
