/** One offline pack's source documents and chronological history.
 * No project root, live session, runtime state, IO, or normalized serializer.
 * The imported archive remains immutable; validation only derives a temporary
 * semantic workspace from the current source and never writes it back.
 */
import { ModPackWorkspace } from './mod-pack-workspace.js';
import { createStoreZip, exportModPack, MANIFEST_PATH, readStoreZipArchive } from './mod-pack-export.js';
import { UndoStack } from './undo-stack.js';

const encode = text => new TextEncoder().encode(text);

function editedLineEndings(previous, edited) {
  // A textarea exposes LF even where source had CRLF. Locate the actual edit
  // in that normalized view, then retain the unchanged ORIGINAL spans. This
  // also handles a file with deliberately mixed line endings without rewriting
  // unrelated lines merely because a browser displayed it.
  const normalized = previous.replace(/\r\n/g, '\n');
  const next = edited.replace(/\r\n/g, '\n');
  if (normalized === next) return previous;
  let prefix = 0;
  while (prefix < normalized.length && prefix < next.length && normalized[prefix] === next[prefix]) prefix += 1;
  let suffix = 0;
  while (suffix < normalized.length - prefix && suffix < next.length - prefix
    && normalized[normalized.length - 1 - suffix] === next[next.length - 1 - suffix]) suffix += 1;
  const offsets = [0];
  for (let index = 0; index < previous.length; index += 1) {
    if (previous[index] === '\r' && previous[index + 1] === '\n') index += 1;
    offsets.push(index + 1);
  }
  const crlfCount = (previous.match(/\r\n/g) || []).length;
  const lfCount = (normalized.match(/\n/g) || []).length - crlfCount;
  const inserted = next.slice(prefix, next.length - suffix);
  return previous.slice(0, offsets[prefix])
    + (crlfCount > lfCount ? inserted.replace(/\n/g, '\r\n') : inserted)
    + previous.slice(offsets[normalized.length - suffix]);
}

export class WorkshopDocument {
  constructor(bytes) {
    const archive = readStoreZipArchive(Uint8Array.from(bytes));
    if (!Object.hasOwn(archive.files, MANIFEST_PATH)) {
      const error = new Error(MANIFEST_PATH);
      error.code = 'workshop-missing-manifest';
      throw error;
    }
    this._source = archive.source;
    this._files = new Map(Object.entries(archive.files));
    this._exported = new Map(this._files);
    this._history = new UndoStack();
  }

  paths() { return [...this._files.keys()]; }
  read(path) { return this._files.get(path); }
  sourceBytes() { return Uint8Array.from(this._source.bytes); }
  canUndo() { return this._history.canUndo(); }
  canRedo() { return this._history.canRedo(); }
  isDirty() {
    return [...this._files].some(([path, text]) => this._exported.get(path) !== text);
  }

  edit(path, text) {
    if (!this._files.has(path) || typeof text !== 'string') return false;
    const before = this._files.get(path);
    const after = editedLineEndings(before, text);
    if (before === after) return false;
    this._history.push({ path, before, after });
    this._files.set(path, after);
    return true;
  }

  undo() {
    const entry = this._history.undo();
    if (!entry) return null;
    this._files.set(entry.path, entry.before);
    return entry.path;
  }

  redo() {
    const entry = this._history.redo();
    if (!entry) return null;
    this._files.set(entry.path, entry.after);
    return entry.path;
  }

  /** Exact candidate for runtime validation/export, even while its text is
   * invalid. No normalizing serializer and no second semantic validation. */
  archive() {
    if ([...this._files].every(([path, text]) => this._source.entries.findLast(entry => entry.path === path)?.text === text)) {
      return this.sourceBytes();
    }
    return createStoreZip([...this._files].map(([path, text]) => {
      const original = this._source.entries.findLast(entry => entry.path === path);
      return { path, text, bytes: original?.text === text ? original.bytes : encode(text) };
    }));
  }

  /** Structural checks only; the ordinary host still compiles/gates Rhai. */
  check() {
    try {
      const entries = [...this._files].map(([path, text]) => {
        const original = this._source.entries.findLast(entry => entry.path === path);
        // The archive reader uses ignoreBOM:true: its text RETAINS U+FEFF, so
        // ordinary encoding preserves the original BOM without prefixing twice.
        return { path, text, bytes: original?.text === text ? original.bytes : encode(text) };
      });
      const workspace = ModPackWorkspace.fromArchiveFiles(
        Object.fromEntries(this._files), {}, { bytes: this._source.bytes, entries },
      );
      return { ...exportModPack(workspace.toExportInput()), packId: workspace.getPack().id };
    } catch (error) {
      return { ok: false, errors: [String(error.message)], warnings: [] };
    }
  }

  /** Called only after the consumer successfully offers the checked download. */
  markExported() { this._exported = new Map(this._files); }

  /** Data only: no filesystem handles, credentials, runtime state or profile. */
  snapshot() {
    const history = this._history.snapshot();
    return { version: 1, source: this.sourceBytes(), files: [...this._files],
      exported: [...this._exported], history: {
        undo: history.undo.map(entry => ({ ...entry })), redo: history.redo.map(entry => ({ ...entry })),
      } };
  }

  static restore(snapshot) {
    if (snapshot?.version !== 1 || !(snapshot.source instanceof Uint8Array)) throw new Error('Unsupported Workshop recovery record.');
    const draft = new WorkshopDocument(snapshot.source);
    const readFiles = entries => {
      if (!Array.isArray(entries) || entries.length !== draft._files.size) throw new Error('Invalid recovery documents.');
      const files = new Map();
      for (const entry of entries) {
        if (!Array.isArray(entry) || entry.length !== 2 || !draft._files.has(entry[0])
          || files.has(entry[0]) || typeof entry[1] !== 'string') throw new Error('Invalid recovery document.');
        files.set(entry[0], entry[1]);
      }
      return files;
    };
    const files = readFiles(snapshot.files);
    const exported = readFiles(snapshot.exported);
    const { undo, redo } = snapshot.history || {};
    for (const entries of [undo, redo]) {
      if (!Array.isArray(entries) || entries.length > 100) throw new Error('Invalid recovery history.');
      const current = new Map(files);
      for (const entry of entries.toReversed()) {
        const direction = entries === undo;
        if (!files.has(entry?.path) || typeof entry.before !== 'string' || typeof entry.after !== 'string'
          || current.get(entry.path) !== (direction ? entry.after : entry.before)) throw new Error('Inconsistent recovery history.');
        current.set(entry.path, direction ? entry.before : entry.after);
      }
    }
    draft._files = files;
    draft._exported = exported;
    draft._history.restore({ undo: undo.map(entry => ({ ...entry })), redo: redo.map(entry => ({ ...entry })) });
    return draft;
  }
}
