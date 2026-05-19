const DEFAULT_MODES = ['Scenario', 'Entity', 'Definitions'];

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
    if (!this._undoHistory[mode]) return [];
    return this._undoHistory[mode][filePath] || [];
  }

  pushUndoEntry(mode, filePath, entry) {
    if (!this._undoHistory[mode]) return;
    if (!this._undoHistory[mode][filePath]) {
      this._undoHistory[mode][filePath] = [];
    }
    this._undoHistory[mode][filePath].push(entry);
  }
}
