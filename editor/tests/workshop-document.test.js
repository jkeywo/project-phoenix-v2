import { describe, expect, it } from 'vitest';
import { WorkshopDocument } from '../workshop-document.js';
import { readStoreZip, readStoreZipArchive, createStoreZip } from '../mod-pack-export.js';
import { workshopPack, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT, WORKSHOP_MANIFEST } from '../../tests/fixtures/workshop-pack.js';

describe('offline Workshop source documents', () => {
  it.each(['assets/models/test.glb', 'assets/models/nested/buffer.bin'])('imports %s bytes without text decoding and undoes additions in chronological order', path => {
    const draft = new WorkshopDocument(workshopPack());
    const asset = new Uint8Array([0, 255, 13, 10, 128, 10]);
    draft.put(path, asset);
    asset.fill(0);
    expect(draft.isBinary(path)).toBe(true);
    expect(draft.read(path)).toBeUndefined();
    expect(draft.bytes(path)).toEqual(new Uint8Array([0, 255, 13, 10, 128, 10]));
    const cloned = new WorkshopDocument(draft.archive());
    expect(cloned.bytes(path)).toEqual(draft.bytes(path));
    draft.edit(WORKSHOP_WORLD, '# changed\n[global]\n');
    draft.undo();
    expect(draft.undo()).toBe(path);
    expect(draft.archive()).toEqual(workshopPack());
    const recovered = WorkshopDocument.restore(draft.snapshot());
    recovered.redo();
    expect(recovered.bytes(path)).toEqual(cloned.bytes(path));
    // Structural round-trip; export still requires the runtime decoder.
    expect(recovered.check()).toMatchObject({ ok: true, zip: recovered.archive() });
  });

  it('loads a native project with its actual manifest path and preserves exact binary/text members', () => {
    const files = { 'assets/scenarios.toml': [...new TextEncoder().encode('[content]\nid="base"\nepoch=1\n')],
      'assets/models/mesh.glb': [255, 0, 10, 13] };
    const project = WorkshopDocument.fromFiles(files, { kind: 'project' });
    expect(project.toFiles()).toEqual(files);
    project.put('assets/worlds/new.toml', '[global]\n');
    const recovered = WorkshopDocument.restore(project.snapshot());
    expect(recovered.kind).toBe('project');
    expect(recovered.toFiles()).toEqual(project.toFiles());
    expect(recovered.undo()).toBe('assets/worlds/new.toml');
    expect(recovered.toFiles()).toEqual(files);
  });
  it.each(['assets/sounds/music.mp3', 'assets/models/mesh.bin'])('retains %s bytes through source import, history and recovery before runtime pack admission', path => {
    const bytes = new Uint8Array([73, 68, 51, 255, 128, 0]);
    const draft = new WorkshopDocument(workshopPack());
    draft.put(path, bytes);
    const imported = new WorkshopDocument(draft.archive());
    expect(imported.bytes(path)).toEqual(bytes);
    draft.undo();
    const recovered = WorkshopDocument.restore(draft.snapshot());
    recovered.redo();
    expect(recovered.bytes(path)).toEqual(bytes);
  });
  it('retains the complete original ZIP container when no member source changed', () => {
    const base = workshopPack();
    const original = new Uint8Array([...base, 7, 8]);
    new DataView(original.buffer).setUint16(base.length - 2, 2, true); // two-byte ZIP comment
    const draft = new WorkshopDocument(original);
    expect(draft.archive()).toEqual(original);
    draft.edit(WORKSHOP_WORLD, `${WORKSHOP_WORLD_TEXT}# Changed\n`);
    expect(draft.archive()).not.toEqual(original);
    draft.undo();
    expect(draft.archive()).toEqual(original);
  });

  it('exports an untouched pack byte-for-byte and retains immutable imported bytes after edits', () => {
    const bytes = workshopPack();
    const original = Uint8Array.from(bytes);
    const draft = new WorkshopDocument(bytes);
    bytes.fill(0);
    expect(draft.check()).toMatchObject({ ok: true, zip: original });
    draft.edit(WORKSHOP_WORLD, `${WORKSHOP_WORLD_TEXT}# Extra\n`);
    const source = draft.sourceBytes();
    source.fill(0);
    expect(draft.sourceBytes()).toEqual(original);
  });

  it('keeps edited manifest comments, extension keys and CRLF without normalized serialization', () => {
    const draft = new WorkshopDocument(workshopPack());
    const manifest = WORKSHOP_MANIFEST.replace('Workshop test', 'Edited pack');
    draft.edit('scenarios.toml', manifest.replace(/\r\n/g, '\n'));
    const result = draft.check();
    expect(result.ok).toBe(true);
    const files = readStoreZip(result.zip);
    expect(files['scenarios.toml']).toBe(manifest);
    expect(files[WORKSHOP_WORLD]).toBe(WORKSHOP_WORLD_TEXT);
  });

  it('undoes edits chronologically across documents and selects the affected path', () => {
    const draft = new WorkshopDocument(workshopPack());
    expect(draft.edit(WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT.replace(/\r\n/g, '\n'))).toBe(false);
    draft.edit(WORKSHOP_WORLD, `${WORKSHOP_WORLD_TEXT}# First\n`);
    draft.edit('scenarios.toml', WORKSHOP_MANIFEST.replace('Workshop test', 'Second'));
    expect(draft.undo()).toBe('scenarios.toml');
    expect(draft.read('scenarios.toml')).toBe(WORKSHOP_MANIFEST);
    expect(draft.undo()).toBe(WORKSHOP_WORLD);
    expect(draft.isDirty()).toBe(false);
    expect(draft.redo()).toBe(WORKSHOP_WORLD);
    draft.edit(WORKSHOP_WORLD, `${WORKSHOP_WORLD_TEXT}# Branch\n`);
    expect(draft.canRedo()).toBe(false);
  });

  it('tracks the last successful export separately from history or validation', () => {
    const draft = new WorkshopDocument(workshopPack());
    draft.edit(WORKSHOP_WORLD, `${WORKSHOP_WORLD_TEXT}# Edited\n`);
    expect(draft.check().ok).toBe(true);
    expect(draft.isDirty()).toBe(true);
    draft.markExported();
    expect(draft.isDirty()).toBe(false);
    draft.undo();
    expect(draft.isDirty()).toBe(true);
    draft.redo();
    expect(draft.isDirty()).toBe(false);
  });

  it('preserves mixed line terminators around an unrelated source edit', () => {
    const original = '# First CRLF\r\n# Second LF\n# Third CRLF\r\n[global]\r\n[anchors]\n';
    const draft = new WorkshopDocument(createStoreZip([
      { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
      { path: WORKSHOP_WORLD, text: original },
    ]));
    const displayed = draft.read(WORKSHOP_WORLD).replace(/\r\n/g, '\n');
    expect(draft.edit(WORKSHOP_WORLD, displayed)).toBe(false);
    draft.edit(WORKSHOP_WORLD, displayed.replace('Second LF', 'Edited LF'));
    const exported = readStoreZipArchive(draft.check().zip).source.entries.find(entry => entry.path === WORKSHOP_WORLD);
    expect(exported.text).toBe(original.replace('Second LF', 'Edited LF'));
    draft.undo();
    const restored = readStoreZipArchive(draft.check().zip).source.entries.find(entry => entry.path === WORKSHOP_WORLD);
    expect(restored.bytes).toEqual(new TextEncoder().encode(original));
  });

  it('keeps a UTF-8 BOM in source even when the existing TOML gate refuses it', () => {
    const original = `\ufeff${WORKSHOP_WORLD_TEXT}`;
    const bytes = createStoreZip([
      { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
      { path: WORKSHOP_WORLD, text: original },
    ]);
    const draft = new WorkshopDocument(bytes);
    draft.edit(WORKSHOP_WORLD, draft.read(WORKSHOP_WORLD).replace('comments', 'notes'));
    expect(draft.read(WORKSHOP_WORLD)).toBe(original.replace('comments', 'notes'));
    expect(draft.check().ok).toBe(false);
    draft.undo();
    expect(draft.read(WORKSHOP_WORLD)).toBe(original);
    expect(draft.sourceBytes()).toEqual(bytes);
  });

  it('keeps invalid manifest source available for repair and refuses export until repaired', () => {
    const invalid = createStoreZip([
      { path: 'scenarios.toml', text: '[pack\n' },
      { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT },
    ]);
    const draft = new WorkshopDocument(invalid);
    expect(draft.check()).toMatchObject({ ok: false });
    expect(draft.read('scenarios.toml')).toBe('[pack\n');
    draft.edit('scenarios.toml', WORKSHOP_MANIFEST);
    expect(draft.check().ok).toBe(true);
    expect(draft.sourceBytes()).toEqual(invalid);
  });

  it('retains invalid member source after refusal and allows undo to restore a valid pack', () => {
    const draft = new WorkshopDocument(workshopPack());
    draft.edit(WORKSHOP_WORLD, '[global\n');
    expect(draft.check()).toMatchObject({ ok: false });
    expect(draft.read(WORKSHOP_WORLD)).toBe('[global\r\n');
    draft.undo();
    expect(draft.check().ok).toBe(true);
  });

  it('recovers invalid source, immutable original bytes, export baseline and cross-file undo/redo', () => {
    const draft = new WorkshopDocument(workshopPack());
    draft.edit(WORKSHOP_WORLD, `${WORKSHOP_WORLD_TEXT}# Exported\n`);
    draft.markExported();
    draft.edit('scenarios.toml', '[pack\n');
    draft.edit(WORKSHOP_WORLD, `${WORKSHOP_WORLD_TEXT}# Later\n`);
    draft.undo();
    const snapshot = draft.snapshot();
    const restored = WorkshopDocument.restore(snapshot);
    snapshot.files[0][1] = 'later mutation';
    snapshot.history.undo[0].changes[0].after = 'later mutation';
    expect(restored.sourceBytes()).toEqual(workshopPack());
    expect(restored.read('scenarios.toml')).toBe('[pack\r\n');
    expect(restored.isDirty()).toBe(true);
    expect(restored.redo()).toBe(WORKSHOP_WORLD);
    expect(restored.read(WORKSHOP_WORLD)).toContain('# Later');
    restored.undo();
    expect(restored.undo()).toBe('scenarios.toml');
    expect(restored.isDirty()).toBe(false);
  });

  it('recovers a draft that deleted or renamed an imported member', () => {
    // The guard that every imported path must still be present was only valid
    // while a member could not be removed. Refusing here would throw away the
    // whole crash draft over the very edit the operator most wants back.
    const draft = new WorkshopDocument(workshopPack());
    const before = draft.read(WORKSHOP_WORLD);
    draft.remove(WORKSHOP_WORLD);
    const recovered = WorkshopDocument.restore(draft.snapshot());
    expect(recovered.paths()).not.toContain(WORKSHOP_WORLD);
    // And the removal is still one undo away, with its exact bytes.
    expect(recovered.undo()).toBe(WORKSHOP_WORLD);
    expect(recovered.read(WORKSHOP_WORLD)).toBe(before);

    const moved = new WorkshopDocument(workshopPack());
    moved.rename(WORKSHOP_WORLD, 'assets/worlds/moved.toml');
    const back = WorkshopDocument.restore(moved.snapshot());
    expect(back.read('assets/worlds/moved.toml')).toBe(before);
    expect(back.paths()).not.toContain(WORKSHOP_WORLD);
    // One press returns BOTH halves of the rename.
    back.undo();
    expect(back.read(WORKSHOP_WORLD)).toBe(before);
    expect(back.paths()).not.toContain('assets/worlds/moved.toml');
  });

  it('exports a candidate that reflects a removal and a rename', () => {
    const draft = new WorkshopDocument(workshopPack());
    draft.rename(WORKSHOP_WORLD, 'assets/worlds/moved.toml');
    // The archive short-circuit returns the imported container only when the
    // members still match it; a rename keeps the COUNT equal, so this proves
    // the short-circuit is not fooled by size alone.
    const renamed = new WorkshopDocument(draft.archive());
    expect(renamed.paths()).toContain('assets/worlds/moved.toml');
    expect(renamed.paths()).not.toContain(WORKSHOP_WORLD);

    draft.remove('assets/worlds/moved.toml');
    const removed = new WorkshopDocument(draft.archive());
    expect(removed.paths()).not.toContain('assets/worlds/moved.toml');
  });

  it('omits a removed member from what a native save is given', () => {
    const draft = WorkshopDocument.fromNativeFiles({
      'assets/worlds/a.toml': new TextEncoder().encode('[global]\n'),
      'assets/worlds/b.toml': new TextEncoder().encode('[global]\n'),
    }, { kind: 'project' });
    draft.remove('assets/worlds/b.toml');
    // The native save sends the members that remain; the Rust side turns a
    // baseline path missing from that map into a deletion on disk.
    expect(Object.keys(draft.toNativeSources())).toEqual(['assets/worlds/a.toml']);
  });

  it('refuses corrupt recovery paths or history before creating an editable draft', () => {
    const draft = new WorkshopDocument(workshopPack());
    draft.edit(WORKSHOP_WORLD, '[invalid\n');
    const foreign = draft.snapshot();
    foreign.files[0][0] = '../outside.toml';
    expect(() => WorkshopDocument.restore(foreign)).toThrow('recovery document');
    const inconsistent = draft.snapshot();
    inconsistent.history.undo[0].changes[0].after = 'different source';
    expect(() => WorkshopDocument.restore(inconsistent)).toThrow('Inconsistent recovery history');
  });

  it('keeps native asset versions compact through replacement, save, undo and recovery without granting browser references', () => {
    const original = { asset: '0000000000000001-400000000', length: 400000000 };
    const next = { asset: '0000000000000002-123', length: 123 };
    const draft = WorkshopDocument.fromNativeFiles({
      'assets/worlds/test.toml': new TextEncoder().encode('# Exact\r\n[global]\n'),
      'assets/models/test.glb': original,
    }, { kind: 'project' });
    original.length = 5;
    expect(draft.byteLength('assets/models/test.glb')).toBe(400000000);
    expect(() => draft.bytes('assets/models/test.glb')).toThrow('private provider');
    draft.put('assets/models/test.glb', next); draft.markExported();
    draft.edit('assets/worlds/test.toml', '# Edited\n[global]\n');
    draft.undo(); draft.undo();
    expect(draft.isDirty()).toBe(true);
    const snapshot = draft.snapshot();
    expect(JSON.stringify(snapshot).length).toBeLessThan(2000);
    expect(() => WorkshopDocument.restore(snapshot)).toThrow('Unsupported');
    const recovered = WorkshopDocument.restore(snapshot, { native: true });
    expect(recovered.read('assets/worlds/test.toml')).toBe('# Exact\r\n[global]\n');
    recovered.redo(); expect(recovered.isDirty()).toBe(false);
    expect(recovered.toFiles()['assets/models/test.glb']).toEqual(next);
    expect(() => new WorkshopDocument(workshopPack()).put('assets/models/test.glb', next)).toThrow('Invalid Workshop document');
    snapshot.history.redo[0].changes[0].after = { ...next, asset: '../outside' };
    expect(() => WorkshopDocument.restore(snapshot, { native: true })).toThrow('recovery document');
  });
});
