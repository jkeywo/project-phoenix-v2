import { validateFile, partitionFindings } from './validation.js';
import { validateRigSidecarText } from './models-rig.js';
import { INCLUDES_KEY } from './entity-includes.js';

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
   * @param {(content: string) => boolean} [commentConfirm]
   *   Optional gate: called with the ON-DISK file text (read via
   *   `readFile`) before writing, if and only if that text contains a
   *   line whose first non-whitespace character is `#` — i.e. an actual
   *   TOML comment that would be lost by the editor's normalised
   *   write. If it returns `false` the save is aborted with
   *   `{ ok: false, ... }`. When omitted (or null) no gate is applied.
   * @param {(path: string) => Promise<string>} [readFile]
   *   Optional reader for the on-disk file content. Required for the
   *   comment gate to fire — without it the gate is skipped (back-
   *   compat for older tests / callers).
   */
  constructor(modeShell, stringifyFunctions, writeFile, invalidationBus, commentConfirm, readFile) {
    this._modeShell = modeShell;
    this._stringifyFunctions = stringifyFunctions;
    this._writeFile = writeFile || (async () => {});
    this._invalidationBus = invalidationBus || null;
    this._commentConfirm = typeof commentConfirm === 'function' ? commentConfirm : null;
    this._readFile = typeof readFile === 'function' ? readFile : null;
    this._contentCache = {};
    /**
     * Optional predicate keyed by `${mode}:${path}` returning true if the
     * file should be EXCLUDED from getDirtyFiles (and therefore from
     * Save All) — see `LayerManager.addInMemoryLayer({ sessionOnly: true })`.
     * Set via `setSessionOnlyChecker`. No editor surface creates a
     * session-only layer since issue #985 deleted the triggerable-worlds
     * preview panel; the hook is kept for a future preview loader.
     */
    this._isSessionOnly = null;
    /**
     * Optional cross-file rig index (issue #758). When set, entity saves
     * resolve every authored marker reference against the rig sidecar the
     * entity's `[mesh]` selects; unresolved references are errors and block
     * the write, so no invalid attachment is ever persisted. Set via
     * `setRigIndex`.
     */
    this._rigIndex = null;
    /**
     * Optional include resolver (issue #910). When set, an Entity save whose
     * document declares `includes` is validated against its RESOLVED document
     * (fragment fields merged in) rather than the authored one — so a hull that
     * authors only `tags` + `includes`, and inherits its `[behaviour]`/`[mesh]`
     * from a fragment, still has that behaviour/markers validated at the save
     * gate. The AUTHORED document is still what gets written; only validation
     * sees the resolved shape. Set via `setEntityResolver`.
     */
    this._entityResolver = null;
  }

  /**
   * Register the `RigIndex` (see `marker-validate.js`) used for cross-file
   * model-marker validation. Without one, marker checks are skipped.
   */
  setRigIndex(rigIndex) {
    this._rigIndex = rigIndex && typeof rigIndex.forEntity === 'function' ? rigIndex : null;
  }

  /**
   * Register the include resolver (issue #910) used to validate a composed
   * entity against its RESOLVED document on save. `fn(path, authoredText)`
   * resolves the LIVE authored text plus its on-disk fragment closure and
   * returns `{ ok: true, value }` (the resolved document) or `{ ok: false,
   * error }` (a located `IncludeError`). Without one, entity saves validate the
   * authored document alone, exactly as before.
   */
  setEntityResolver(fn) {
    this._entityResolver = typeof fn === 'function' ? fn : null;
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
    if (mode === 'World') {
      return this._stringifyFunctions.world;
    }
    if (mode === 'Definitions') {
      // Backward-compat: tests / older callers that don't supply a
      // `definitions` stringifier fall through to `entity`. This keeps
      // pre-Slice-6 fixtures working unchanged.
      return this._stringifyFunctions.definitions || this._stringifyFunctions.entity;
    }
    if (mode === 'Models') {
      // Models Mode caches a ready-made TOML *string* via setContent, so its
      // stringifier is a passthrough. Without one, a Save-All over a dirty
      // Models file would mis-route to the entity stringifier. Falls back to
      // a passthrough when not supplied so older fixtures can't crash.
      return this._stringifyFunctions.models || ((s) => s);
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

  async saveActive() {
    const mode = this._modeShell.getCurrentMode();
    const path = this._modeShell.getActiveFile(mode);

    if (!path) {
      return { ok: false, errors: ['No active file to save'], warnings: [] };
    }

    return this._saveOne(mode, path);
  }

  async saveAll() {
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

    // A composed entity (issue #910) is validated against its RESOLVED
    // document, so a hull that authors only `tags` + `includes` and inherits
    // its `[behaviour]`/`[mesh]` from a fragment does not slip past
    // behaviour/marker validation. We resolve FRESH from `content` (the LIVE
    // authored text about to be written), never a possibly-stale cache, and
    // still write the AUTHORED document — `includes` stay intact on disk. A
    // resolution failure (missing fragment, cycle, malformed include) blocks
    // the save with the located error instead of writing a broken hull.
    let toValidate = parsedContent;
    if (
      mode === 'Entity' &&
      this._entityResolver &&
      parsedContent &&
      typeof parsedContent === 'object' &&
      !Array.isArray(parsedContent) &&
      Object.prototype.hasOwnProperty.call(parsedContent, INCLUDES_KEY)
    ) {
      let resolution;
      try {
        resolution = await this._entityResolver(path, content);
      } catch (e) {
        return { ok: false, errors: [`Include resolution failed: ${e.message}`], warnings: [] };
      }
      if (!resolution || !resolution.ok) {
        const err = resolution?.error;
        const located = err
          ? `${err.category}: ${err.message}${err.file ? ` (in ${err.file})` : ''}`
          : 'include resolution failed';
        return { ok: false, errors: [located], warnings: [] };
      }
      toValidate = resolution.value;
    }

    // Models mode caches a ready-made TOML *string* (not a parsed object), so
    // validateFile would emit a junk "Root value must be an object" warning.
    // It gets the rig-sidecar validator instead (issue #758) — a rig that
    // declares the same marker twice would silently last-wins on load, so the
    // duplicate blocks the save rather than shipping a broken mount point.
    // `validateRigSidecarText` is the SAME rule set Models Mode's own Save
    // button applies, so neither write path is a way around the other.
    const validationResults =
      mode === 'Models'
        ? validateRigSidecarText(content)
        : validateFile(path, toValidate, { rigIndex: this._rigIndex });
    // Split findings by severity (issue #757). Warnings stay visible and flow
    // through untouched on every path; definite errors BLOCK the save before
    // anything is written and before any cache/undo/invalidation fires, so a
    // blocked save leaves the file dirty and the editor caches intact.
    const { errors: errorFindings, warnings: warningFindings } =
      partitionFindings(validationResults);
    const warnings = warningFindings.map((r) => r.message);

    if (errorFindings.length > 0) {
      return { ok: false, errors: errorFindings.map((r) => r.message), warnings };
    }

    if (this._commentConfirm && this._readFile) {
      let onDisk = null;
      try {
        onDisk = await this._readFile(path);
      } catch {
        // Missing/new file → nothing to lose. Skip gate.
      }
      if (typeof onDisk === 'string' && /^\s*#/m.test(onDisk)) {
        const ok = this._commentConfirm(onDisk);
        if (!ok) {
          return { ok: false, errors: ['Save aborted: file contains comments'], warnings };
        }
      }
    }

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
      } else if (mode === 'World' && typeof this._invalidationBus.fireWorldSaved === 'function') {
        this._invalidationBus.fireWorldSaved(path);
      } else if (
        mode === 'Definitions' &&
        typeof path === 'string' &&
        path.startsWith('assets/factions/') &&
        typeof this._invalidationBus.fireFactionSaved === 'function'
      ) {
        this._invalidationBus.fireFactionSaved(path);
      } else if (mode === 'Models' && typeof this._invalidationBus.fireModelSaved === 'function') {
        // Re-seed the cross-file rig index (issue #758). Without this, a
        // marker added in Models Mode stays invisible to entity saves until
        // the editor reloads, and a legitimate entity save is refused.
        this._invalidationBus.fireModelSaved(path, content);
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
