import { validateFile } from './validation.js';

export class SaveFlow {
  constructor(modeShell, stringifyFunctions) {
    this._modeShell = modeShell;
    this._stringifyFunctions = stringifyFunctions;
    this._contentCache = {};
  }

  setContent(mode, filePath, parsedContent) {
    if (!this._contentCache[mode]) {
      this._contentCache[mode] = {};
    }
    this._contentCache[mode][filePath] = parsedContent;
  }

  _getStringifyFn(mode) {
    if (mode === 'Scenario') {
      return this._stringifyFunctions.world;
    }
    return this._stringifyFunctions.entity;
  }

  getContentForFile(mode, filePath, parsedContent) {
    const fn = this._getStringifyFn(mode);
    return fn(parsedContent);
  }

  _getParsedContent(mode, filePath) {
    return this._contentCache[mode]?.[filePath];
  }

  saveActive(crossRefIndex) {
    const mode = this._modeShell.getCurrentMode();
    const path = this._modeShell.getActiveFile(mode);

    if (!path) {
      return { ok: false, errors: ['No active file to save'], warnings: [] };
    }

    const parsedContent = this._getParsedContent(mode, path);
    if (!parsedContent) {
      return { ok: false, errors: ['No content available for the active file'], warnings: [] };
    }

    let content;
    try {
      content = this.getContentForFile(mode, path, parsedContent);
    } catch (e) {
      return { ok: false, errors: [e.message], warnings: [] };
    }

    const validationResults = validateFile(path, parsedContent);
    const warnings = validationResults.map((r) => r.message);

    this._modeShell.markDirty(mode, path, false);

    return { ok: true, errors: [], warnings };
  }

  saveAll(crossRefIndex) {
    const dirtyFiles = this.getDirtyFiles();
    const results = [];

    for (const { mode, path } of dirtyFiles) {
      const parsedContent = this._getParsedContent(mode, path);
      if (!parsedContent) {
        results.push({ path, ok: false, errors: ['No content available'], warnings: [] });
        continue;
      }

      try {
        this.getContentForFile(mode, path, parsedContent);
      } catch (e) {
        results.push({ path, ok: false, errors: [e.message], warnings: [] });
        continue;
      }

      const validationResults = validateFile(path, parsedContent);
      const warnings = validationResults.map((r) => r.message);

      this._modeShell.markDirty(mode, path, false);
      results.push({ path, ok: true, errors: [], warnings });
    }

    return results;
  }

  getDirtyFiles() {
    const result = [];
    const modes = this._modeShell.getModes();
    for (const mode of modes) {
      const files = this._modeShell.getOpenFiles(mode);
      if (!files) continue;
      for (const file of files) {
        if (this._modeShell.isDirty(mode, file)) {
          result.push({ mode, path: file });
        }
      }
    }
    return result;
  }
}
