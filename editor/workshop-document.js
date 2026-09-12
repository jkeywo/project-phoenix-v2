/** One offline pack's source documents and chronological history.
 * No project root, live session, runtime state, IO, or normalized serializer.
 * The imported archive remains immutable; validation only derives a temporary
 * semantic workspace from the current source and never writes it back.
 */
import { ModPackWorkspace } from './mod-pack-workspace.js';
import { exportModPack, MANIFEST_PATH, readStoreZipArchive } from './mod-pack-export.js';
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
      throw new Error('scenarios.toml is missing');
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
}
