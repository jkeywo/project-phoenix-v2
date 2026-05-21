import { validateFile } from './validation.js';

export class SaveFlow {
  /**
   * @param {ModeShell} modeShell
   * @param {{ world: (obj) => string, entity: (obj) => string }} stringifyFunctions
   * @param {(path: string, content: string) => Promise<void>} [writeFile]
   *   FSA-backed writer. Defaults to a no-op that resolves immediately so
   *   pre-Slice-1 callers / older tests that don't supply one keep working.
   * @param {{ fireEntitySaved?: (path: string) => void,
   *           fireWorldSaved?: (path: string) => void }} [invalidationBus]
   *   Notified on successful save. Optional for the same back-compat reason.
   */
  constructor(modeShell, stringifyFunctions, writeFile, invalidationBus) {
    this._modeShell = modeShell;
    this._stringifyFunctions = stringifyFunctions;
    this._writeFile = writeFile || (async () => {});
    this._invalidationBus = invalidationBus || null;
    this._contentCache = {};
    /**
     * Optional predicate keyed by `${mode}:${path}` returning true if the
     * file should be EXCLUDED from getDirtyFiles (and therefore from
     * Save All). Wired by Slice 4b for session-only triggerable-world
     * layers — see `LayerManager.addInMemoryLayer({ sessionOnly: true })`.
     * Set via `setSessionOnlyChecker`.
     */
    this._isSessionOnly = null;
  }

  /**
   * Register a predicate `(mode, path) => boolean` that returns true for
   * files which should be excluded from getDirtyFiles. Used so the
   * Triggerable-Worlds panel can layer session-only worlds without
   * Save All trying to write them back (they have no FSA handle).
   */
  setSessionOnlyChecker(fn) {
    this._isSessionOnly = typeof fn === 'function' ? fn : null;
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
    if (mode === 'Definitions') {
      // Backward-compat: tests / older callers that don't supply a
      // `definitions` stringifier fall through to `entity`. This keeps
      // pre-Slice-6 fixtures working unchanged.
      return this._stringifyFunctions.definitions || this._stringifyFunctions.entity;
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

  async saveActive(crossRefIndex) {
    const mode = this._modeShell.getCurrentMode();
    const path = this._modeShell.getActiveFile(mode);

    if (!path) {
      return { ok: false, errors: ['No active file to save'], warnings: [] };
    }

    return this._saveOne(mode, path);
  }

  async saveAll(crossRefIndex) {
    const dirtyFiles = this.getDirtyFiles();
    const results = [];

    for (const { mode, path } of dirtyFiles) {
      const result = await this._saveOne(mode, path);
      results.push({ path, ...result });
    }

    return results;
  }

  async _saveOne(mode, path) {
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

    try {
      await this._writeFile(path, content);
    } catch (e) {
      return { ok: false, errors: [`Write failed: ${e.message}`], warnings };
    }

    this._modeShell.markDirty(mode, path, false);
    this._modeShell.clearUndoHistory(mode, path);

    if (this._invalidationBus) {
      if (mode === 'Entity' && typeof this._invalidationBus.fireEntitySaved === 'function') {
        this._invalidationBus.fireEntitySaved(path);
      } else if (mode === 'Scenario' && typeof this._invalidationBus.fireWorldSaved === 'function') {
        this._invalidationBus.fireWorldSaved(path);
      } else if (
        mode === 'Definitions' &&
        typeof path === 'string' &&
        path.startsWith('assets/factions/') &&
        typeof this._invalidationBus.fireFactionSaved === 'function'
      ) {
        this._invalidationBus.fireFactionSaved(path);
      }
    }

    return { ok: true, errors: [], warnings };
  }

  getDirtyFiles() {
    const result = [];
    const modes = this._modeShell.getModes();
    for (const mode of modes) {
      const files = this._modeShell.getOpenFiles(mode);
      if (!files) continue;
      for (const file of files) {
        if (this._isSessionOnly && this._isSessionOnly(mode, file)) continue;
        if (this._modeShell.isDirty(mode, file)) {
          result.push({ mode, path: file });
        }
      }
    }
    return result;
  }
}
