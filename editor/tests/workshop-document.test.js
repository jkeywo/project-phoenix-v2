import { describe, expect, it } from 'vitest';
import { WorkshopDocument } from '../workshop-document.js';
import { readStoreZip, readStoreZipArchive, createStoreZip } from '../mod-pack-export.js';
import { workshopPack, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT, WORKSHOP_MANIFEST } from '../../tests/fixtures/workshop-pack.js';

describe('offline Workshop source documents', () => {
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
});
