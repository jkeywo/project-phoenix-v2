const DEFAULT_MODES = ['World', 'Entity', 'Definitions'];

import { UndoStack } from './undo-stack.js';

export class ModeShell {
  constructor(modes = DEFAULT_MODES) {
    this._modes = [...modes];
    this._currentMode = modes[0];
    this._openFiles = {};
    this._dirtyFiles = {};
    this._activeFiles = {};
    this._activeLayer = {};
    this._undoHistory = {};

    for (const mode of modes) {
      this._openFiles[mode] = [];
      this._dirtyFiles[mode] = {};
      this._activeFiles[mode] = null;
      this._activeLayer[mode] = null;
      this._undoHistory[mode] = {};
    }
  }

  getCurrentMode() {
    return this._currentMode;
  }

  getModes() {
    return [...this._modes];
  }

  switchMode(mode) {
    if (!this._modes.includes(mode)) {
      return false;
    }
    this._currentMode = mode;
    return true;
  }

  getOpenFiles(mode) {
    return this._openFiles[mode];
  }

  setOpenFiles(mode, files) {
    if (!this._openFiles[mode]) {
      return;
    }
    this._openFiles[mode] = [...files];
  }

  isDirty(mode, filePath) {
    if (!this._dirtyFiles[mode]) return false;
    return !!this._dirtyFiles[mode][filePath];
  }

  markDirty(mode, filePath, dirty) {
    if (!this._dirtyFiles[mode]) return;
    if (dirty) {
      this._dirtyFiles[mode][filePath] = true;
    } else {
      delete this._dirtyFiles[mode][filePath];
    }
  }

  hasAnyDirty() {
    for (const mode of this._modes) {
      if (Object.keys(this._dirtyFiles[mode]).length > 0) {
        return true;
      }
    }
    return false;
  }

  getActiveFile(mode) {
    if (!this._modes.includes(mode)) return null;
    return this._activeFiles[mode];
  }

  setActiveFile(mode, filePath) {
    if (!this._modes.includes(mode)) return;
    this._activeFiles[mode] = filePath;
  }

  getActiveLayer(mode) {
    if (!this._modes.includes(mode)) return null;
    return this._activeLayer[mode];
  }

  setActiveLayer(mode, filePath) {
    if (!this._modes.includes(mode)) return;
    this._activeLayer[mode] = filePath;
  }

  getUndoHistory(mode, filePath) {
    const stack = this._getStack(mode, filePath, false);
    if (!stack) return [];
    // Preserve legacy contract: callers (and tests) expect a raw array.
    return stack._undoStack;
  }

  pushUndoEntry(mode, filePath, entry) {
    const stack = this._getStack(mode, filePath, true);
    if (!stack) return;
    stack.push(entry);
  }

  undoActive(mode, filePath) {
    const stack = this._getStack(mode, filePath, false);
    if (!stack) return null;
    return stack.undo();
  }

  redoActive(mode, filePath) {
    const stack = this._getStack(mode, filePath, false);
    if (!stack) return null;
    return stack.redo();
  }

  /**
   * Snapshot-before-mutation swap variants. The caller passes the current
   * external state so it gets parked on the opposite stack — see
   * `undo-controller.js` for the contract.
   */
  swapUndoActive(mode, filePath, currentValue) {
    const stack = this._getStack(mode, filePath, false);
    if (!stack) return null;
    return stack.swapUndo(currentValue);
  }

  swapRedoActive(mode, filePath, currentValue) {
    const stack = this._getStack(mode, filePath, false);
    if (!stack) return null;
    return stack.swapRedo(currentValue);
  }

  clearUndoHistory(mode, filePath) {
    const stack = this._getStack(mode, filePath, false);
    if (stack) stack.clear();
  }

  canUndoActive(mode, filePath) {
    const stack = this._getStack(mode, filePath, false);
    return !!stack && stack.canUndo();
  }

  canRedoActive(mode, filePath) {
    const stack = this._getStack(mode, filePath, false);
    return !!stack && stack.canRedo();
  }

  _getStack(mode, filePath, createIfMissing) {
    if (!this._undoHistory[mode]) return null;
    let stack = this._undoHistory[mode][filePath];
    if (!stack && createIfMissing) {
      stack = new UndoStack();
      this._undoHistory[mode][filePath] = stack;
    }
    return stack || null;
  }
}
