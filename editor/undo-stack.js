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
