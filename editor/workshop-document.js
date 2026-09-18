/** One offline pack's source documents and chronological history.
 * No project root, live session, runtime state, IO, or normalized serializer.
 * The imported archive remains immutable; validation only derives a temporary
 * semantic workspace from the current source and never writes it back.
 */
import { ModPackWorkspace } from './mod-pack-workspace.js';
import { createStoreZip, exportModPack, isPackAssetPath, MANIFEST_PATH, readStoreZipArchive } from './mod-pack-export.js';
import { UndoStack } from './undo-stack.js';

const encode = text => new TextEncoder().encode(text);
export const isWorkshopBinary = path => /\.(glb|png|jpg|jpeg|ktx2|ptex|wav|ogg|mp3)$/.test(path)
  || (path.startsWith('assets/models/') && path.endsWith('.bin'));
export const isNativeAssetReference = value => value && typeof value === 'object'
  && Object.keys(value).length === 2 && typeof value.asset === 'string'
  && /^[0-9a-f]{16}-[0-9]+$/.test(value.asset) && Number.isSafeInteger(value.length)
  && value.length >= 0 && value.length <= 512 * 1024 * 1024
  && value.asset.split('-')[1] === String(value.length);
const copy = value => typeof value === 'string' || value == null ? value
  : isNativeAssetReference(value) ? Object.freeze({ ...value }) : Uint8Array.from(value);
const equal = (a, b) => typeof a === 'string' || typeof b === 'string' || a == null || b == null
  ? a === b : isNativeAssetReference(a) || isNativeAssetReference(b)
    ? isNativeAssetReference(a) && isNativeAssetReference(b) && a.asset === b.asset && a.length === b.length
    : a.length === b.length && a.every((byte, index) => byte === b[index]);
const mapsEqual = (a, b) => a.size === b.size && [...a].every(([path, value]) => equal(value, b.get(path)));
const byteArray = value => Array.isArray(value) && value.every(byte => Number.isInteger(byte) && byte >= 0 && byte <= 255);
const safePath = path => typeof path === 'string' && !/[\\:\0]/.test(path)
  && path.split('/').every(part => part && part !== '.' && part !== '..' && !/[. ]$/.test(part));

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
  constructor(bytes, { kind = 'mod' } = {}) {
    if (!['mod', 'project'].includes(kind)) throw new Error('Invalid Workshop kind');
    const archive = readStoreZipArchive(Uint8Array.from(bytes), { binary: isWorkshopBinary });
    if (kind === 'mod' && !Object.hasOwn(archive.files, MANIFEST_PATH)) {
      const error = new Error(MANIFEST_PATH);
      error.code = 'workshop-missing-manifest';
      throw error;
    }
    this.kind = kind;
    this._source = archive.source;
    this._files = new Map(archive.source.entries.map(entry => [entry.path, entry.text ?? entry.bytes]));
    this._exported = new Map(this._files);
    this._history = new UndoStack();
  }

  paths() { return [...this._files.keys()]; }
  read(path) { const value = this._files.get(path); return typeof value === 'string' ? value : undefined; }
  isBinary(path) { return this._files.has(path) && typeof this._files.get(path) !== 'string'; }
  bytes(path) {
    const value = this._files.get(path);
    if (isNativeAssetReference(value)) throw new Error('Native asset bytes require the private provider');
    return value == null ? undefined : Uint8Array.from(typeof value === 'string' ? encode(value) : value);
  }
  byteLength(path) { const value = this._files.get(path); return typeof value === 'string' ? encode(value).length : value?.length; }
  toFiles() { return Object.fromEntries(this.paths().map(path => {
    const value = this._files.get(path);
    return [path, isNativeAssetReference(value) ? { ...value } : Array.from(this.bytes(path))];
  })); }
  toNativeSources() {
    if (!this._native) return this.toFiles();
    return Object.fromEntries([...this._files].map(([path, value]) => [path,
      value instanceof Uint8Array ? Array.from(value) : isNativeAssetReference(value) ? { ...value } : value]));
  }
  /** Native capability only: immutable references never enter a browser pack.
   * They name byte versions in the provider's private store, never paths. */
  static fromNativeFiles(files, { kind } = {}) {
    if (!['project', 'mod'].includes(kind) || !files || Object.keys(files).length > 16384) throw new Error('Invalid native Workshop bundle');
    const entries = Object.entries(files).map(([path, value]) => {
      if (!safePath(path)) throw new Error('Invalid native Workshop source path');
      if (isWorkshopBinary(path) && isNativeAssetReference(value)) return { path, value: copy(value) };
      if (!isWorkshopBinary(path) && typeof value === 'string') return { path, value };
      if (!(value instanceof Uint8Array || byteArray(value))) throw new Error('Invalid native Workshop source');
      const bytes = Uint8Array.from(value);
      return { path, value: isWorkshopBinary(path) ? bytes : new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes) };
    });
    if (kind === 'mod' && !entries.some(entry => entry.path === MANIFEST_PATH)) throw new Error('Missing native Workshop manifest');
    const draft = Object.create(WorkshopDocument.prototype);
    draft.kind = kind; draft._native = true;
    draft._source = { entries: entries.map(({ path, value }) => ({ path, text: typeof value === 'string' ? value : undefined, bytes: copy(value) })) };
    draft._files = new Map(entries.map(({ path, value }) => [path, value]));
    draft._exported = new Map(draft._files); draft._history = new UndoStack();
    return draft;
  }
  static fromFiles(files, options) {
    if (!files || Object.entries(files).some(([path, value]) => !safePath(path) || !(value instanceof Uint8Array || byteArray(value)))) throw new Error('Invalid Workshop source bundle');
    return new WorkshopDocument(createStoreZip(Object.entries(files).map(([path, bytes]) => ({ path, bytes: Uint8Array.from(bytes) }))), options);
  }
  /** Add or replace one authored source/asset in the same chronological history. */
  put(path, value) {
    const binary = value instanceof Uint8Array || (this._native && isNativeAssetReference(value));
    if (!safePath(path) || !(typeof value === 'string' || binary)) throw new Error('Invalid Workshop document');
    if (isWorkshopBinary(path) !== Boolean(binary)) throw new Error('Workshop document type does not match path');
    const before = this._files.get(path) ?? null;
    const after = copy(value);
    if (equal(before, after)) return false;
    return this.apply([{ path, before: copy(before), after }]);
  }

  /** Record one or more member changes as a SINGLE history entry.
   *
   * A rename is two changes — the old path goes, the new path arrives — and a
   * migration may rewrite several members at once. Both have to undo as one
   * press: half a rename is a member with no name, and half an accepted
   * migration is source nobody reviewed. `after: null` removes the member.
   */
  apply(changes) {
    const applicable = changes.filter(change => !equal(change.before, change.after));
    if (!applicable.length) return false;
    this._history.push({ changes: applicable.map(change => ({
      path: change.path, before: copy(change.before), after: copy(change.after) })) });
    for (const change of applicable) {
      if (change.after === null) this._files.delete(change.path);
      else this._files.set(change.path, copy(change.after));
    }
    this._sourceRevision = this.sourceRevision + 1;
    return true;
  }

  /** Remove one authored member. */
  remove(path) {
    const before = this._files.get(path) ?? null;
    if (before === null) return false;
    return this.apply([{ path, before: copy(before), after: null }]);
  }

  /** Move one authored member to another path, bytes untouched.
   *
   * The VALUE is carried across rather than re-encoded, so a rename cannot
   * normalise line endings, a BOM or an asset reference — renaming a file is
   * not an edit of it.
   */
  rename(from, to) {
    if (!safePath(to)) throw new Error('Invalid Workshop document');
    const value = this._files.get(from);
    if (value === undefined) return false;
    if (from === to) return false;
    if (isWorkshopBinary(from) !== isWorkshopBinary(to)) {
      throw new Error('Workshop document type does not match path');
    }
    return this.apply([
      { path: from, before: copy(value), after: null },
      { path: to, before: copy(this._files.get(to) ?? null), after: copy(value) },
    ]);
  }
  sourceBytes() {
    if (this._native) throw new Error('Native source has no imported archive');
    return Uint8Array.from(this._source.bytes);
  }
  canUndo() { return this._history.canUndo(); }
  get sourceRevision() { return this._sourceRevision || 0; }
  canRedo() { return this._history.canRedo(); }
  isDirty() {
    return !mapsEqual(this._files, this._exported);
  }

  /** The members as they stand, for a reader that compares rather than edits.
   * A copy: the live map is the document's own, and a diff view holding it
   * would see edits it never asked for. */
  members() {
    return new Map([...this._files].map(([path, value]) => [path, copy(value)]));
  }

  /** What this draft has done to the source it was imported as.
   *
   * Computed from the live maps WITHOUT copying them. `members()` deep-copies
   * every binary member, and this runs on each keystroke; on a pack carrying
   * multi-megabyte models and audio that is two full copies of the asset set
   * per character typed. Comparison never mutates, so it does not need copies.
   */
  changes(diff) {
    const imported = new Map(this._source.entries.map(entry => [entry.path, entry.text ?? entry.bytes]));
    return diff(imported, this._files);
  }

  /** The members this draft was imported as, which is what "changed" is
   * measured against. Native drafts carry their opened files here too. */
  importedMembers() {
    return new Map(this._source.entries.map(entry => [entry.path, copy(entry.text ?? entry.bytes)]));
  }

  /** The members at the last successful save or export. */
  exportedMembers() {
    return new Map([...this._exported].map(([path, value]) => [path, copy(value)]));
  }

  edit(path, text) {
    if (typeof this._files.get(path) !== 'string' || typeof text !== 'string') return false;
    const before = this._files.get(path);
    const after = editedLineEndings(before, text);
    if (before === after) return false;
    return this.apply([{ path, before, after }]);
  }

  undo() {
    const entry = this._history.undo();
    if (!entry) return null;
    // Reversed: a rename's two halves must come back in the opposite order they
    // were applied, or the arriving member is deleted after it is restored.
    for (const change of [...entry.changes].reverse()) {
      if (change.before === null) this._files.delete(change.path);
      else this._files.set(change.path, copy(change.before));
    }
    this._sourceRevision = this.sourceRevision + 1;
    return entry.changes[0].path;
  }

  redo() {
    const entry = this._history.redo();
    if (!entry) return null;
    for (const change of entry.changes) {
      if (change.after === null) this._files.delete(change.path);
      else this._files.set(change.path, copy(change.after));
    }
    this._sourceRevision = this.sourceRevision + 1;
    return entry.changes.at(-1).path;
  }

  /** Exact candidate for runtime validation/export, even while its text is
   * invalid. No normalizing serializer and no second semantic validation. */
  archive() {
    const original = new Map(this._source.entries.map(entry => [entry.path, entry.text ?? entry.bytes]));
    if (!this._native && mapsEqual(this._files, original)) return this.sourceBytes();
    return createStoreZip(this.paths().map(path => ({ path, bytes: this.bytes(path) })));
  }

  /** Structural checks only; the ordinary host still compiles/gates Rhai. */
  check() {
    try {
      for (const path of this.paths().filter(path => this.isBinary(path))) {
        if (!isPackAssetPath(path)) throw new Error(`Unsupported asset path "${path}"`);
      }
      const textFiles = [...this._files].filter(([, value]) => typeof value === 'string');
      const entries = textFiles.map(([path, text]) => {
        const original = this._source.entries.findLast(entry => entry.path === path);
        // The archive reader uses ignoreBOM:true: its text RETAINS U+FEFF, so
        // ordinary encoding preserves the original BOM without prefixing twice.
        return { path, text, bytes: original?.text === text ? original.bytes : encode(text) };
      });
      const workspace = ModPackWorkspace.fromArchiveFiles(
        Object.fromEntries(textFiles), {}, { bytes: this._source.bytes, entries },
      );
      const result = exportModPack(workspace.toExportInput());
      return { ...result, ...(result.ok ? { zip: this.archive(), paths: this.paths() } : {}), packId: workspace.getPack().id };
    } catch (error) {
      return { ok: false, errors: [String(error.message)], warnings: [] };
    }
  }

  /** Called only after the consumer successfully offers the checked download. */
  markExported() { this._exported = new Map(this._files); }

  /** Data only: no filesystem handles, credentials, runtime state or profile. */
  snapshot() {
    const history = this._history.snapshot();
    const serialize = value => value instanceof Uint8Array ? Array.from(value) : isNativeAssetReference(value) ? { ...value } : value;
    const entry = value => ({ changes: value.changes.map(change => ({
      path: change.path, before: serialize(change.before), after: serialize(change.after) })) });
    // Versions 4 and 5 carry grouped history entries: a rename and an accepted
    // migration each undo as ONE press, so an entry is a list of changes rather
    // than a single path. Versions 2 and 3 are still READ, as one-change
    // groups, because a draft recovered from a pre-#1471 crash is still a draft.
    return { version: this._native ? 5 : 4, kind: this.kind,
      ...(this._native ? { sourceFiles: this._source.entries.map(entry => [entry.path, serialize(entry.text ?? entry.bytes)]) } : { source: this.sourceBytes() }),
      files: [...this._files].map(([path, value]) => [path, serialize(value)]),
      exported: [...this._exported].map(([path, value]) => [path, serialize(value)]), history: {
        undo: history.undo.map(entry), redo: history.redo.map(entry),
      } };
  }

  static restore(snapshot, { native = false } = {}) {
    const nativeSource = native && [3, 5].includes(snapshot?.version);
    if (!nativeSource && (![1, 2, 4].includes(snapshot?.version) || !(snapshot.source instanceof Uint8Array))) throw new Error('Unsupported Workshop recovery record.');
    const grouped = [4, 5].includes(snapshot?.version);
    let draft;
    if (nativeSource) {
      if (!Array.isArray(snapshot.sourceFiles) || new Set(snapshot.sourceFiles.map(entry => entry?.[0])).size !== snapshot.sourceFiles.length
        || snapshot.sourceFiles.some(entry => !Array.isArray(entry) || entry.length !== 2)) throw new Error('Invalid native recovery source');
      draft = WorkshopDocument.fromNativeFiles(Object.fromEntries(snapshot.sourceFiles.map(([path, value]) => [path,
        typeof value === 'string' ? encode(value) : value])), { kind: snapshot.kind });
    } else draft = new WorkshopDocument(snapshot.source, { kind: snapshot.kind || 'mod' });
    const decode = (path, value, nullable = false) => {
      if (nullable && value === null) return null;
      if (typeof value === 'string' && !isWorkshopBinary(path)) return value;
      if (isWorkshopBinary(path) && byteArray(value)) return Uint8Array.from(value);
      if (nativeSource && isWorkshopBinary(path) && isNativeAssetReference(value)) return copy(value);
      throw new Error('Invalid recovery document.');
    };
    const readFiles = entries => {
      if (!Array.isArray(entries) || entries.length > 16384) throw new Error('Invalid recovery documents.');
      const files = new Map();
      for (const entry of entries) {
        if (!Array.isArray(entry) || entry.length !== 2 || !safePath(entry[0]) || files.has(entry[0])) throw new Error('Invalid recovery document.');
        files.set(entry[0], decode(entry[0], entry[1]));
      }
      // Deliberately NOT requiring every imported path to be present. That was
      // valid only while a member could not be removed; since issue #1471 a
      // deleted or renamed member is legitimately absent, and refusing it here
      // would throw away the whole crash draft over the very edit the operator
      // most wants back. History consistency is still proved by the replay
      // below, which is the check that actually protects the bytes.
      return files;
    };
    const files = readFiles(snapshot.files);
    const exported = readFiles(snapshot.exported);
    const { undo, redo } = snapshot.history || {};
    const decoded = [];
    for (const entries of [undo, redo]) {
      if (!Array.isArray(entries) || entries.length > 100) throw new Error('Invalid recovery history.');
      const values = entries.map(entry => {
        // A pre-#1471 record has one change per entry; read it as a group of
        // one rather than refusing a draft someone was in the middle of.
        const changes = grouped ? entry?.changes : [entry];
        if (!Array.isArray(changes) || !changes.length || changes.length > 64) {
          throw new Error('Invalid recovery history.');
        }
        return { changes: changes.map(change => {
          if (!safePath(change?.path)) throw new Error('Invalid recovery document.');
          return { path: change.path, before: decode(change.path, change.before, true),
            after: decode(change.path, change.after, true) };
        }) };
      });
      const current = new Map(files);
      for (const entry of values.toReversed()) {
        const direction = entries === undo;
        // Undo walks a group backwards, redo forwards: the same order the live
        // document applies them, so the replay proves the recorded bytes really
        // do reconstruct what was on screen.
        const ordered = direction ? [...entry.changes].reverse() : entry.changes;
        for (const change of ordered) {
          if (!equal(current.get(change.path) ?? null, direction ? change.after : change.before)) {
            throw new Error('Inconsistent recovery history.');
          }
          const value = direction ? change.before : change.after;
          if (value === null) current.delete(change.path); else current.set(change.path, value);
        }
      }
      decoded.push(values);
    }
    draft._files = files;
    draft._exported = exported;
    draft._history.restore({ undo: decoded[0], redo: decoded[1] });
    return draft;
  }
}
