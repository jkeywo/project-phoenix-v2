import { describe, expect, it } from 'vitest';
import { proposeContentPinMigration, proposeWorkshopMigration } from '../workshop-migration.js';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';

const manifest = (epoch, { id = 'phoenix-base', extra = '' } = {}) =>
  `# Keep this note\r\n[pack]\r\nformat = 1\r\nid = "p"\r\nversion = "1.0.0"\r\nname = "P"\r\n`
  + `[pack.requires]\r\ncontent_id = "${id}"\r\ncontent_epoch = ${epoch} # pinned\r\n${extra}`
  + `[[scenario]]\r\nid = "s"\r\nworld = "assets/worlds/w.toml"\r\n`;

const pack = source => createStoreZip([
  { path: 'scenarios.toml', text: source },
  { path: 'assets/worlds/w.toml', text: '[global]\r\n' },
]);

describe('proposeContentPinMigration', () => {
  it('proposes moving a stale pin forward, rewriting one scalar and nothing else', () => {
    const before = manifest(1);
    const proposed = proposeContentPinMigration(before, { contentId: 'phoenix-base', contentEpoch: 4 });
    expect(proposed).toMatchObject({ kind: 'content-epoch', from: 1, to: 4 });
    expect(proposed.after).toContain('content_epoch = 4 # pinned');
    // Everything around it is byte-identical: the comment, the key order, the
    // CRLF terminators. A migration that reserialised would normalise source
    // the author never asked it to touch.
    expect(proposed.after.replace('content_epoch = 4', 'content_epoch = 1')).toBe(before);
    expect(proposed.before).toBe(before);
  });

  it('stays silent when there is nothing to propose', () => {
    // Already current.
    expect(proposeContentPinMigration(manifest(4), { contentId: 'phoenix-base', contentEpoch: 4 })).toBeNull();
    // No pin at all.
    expect(proposeContentPinMigration('[pack]\nid = "p"\n', { contentEpoch: 4 })).toBeNull();
    // Nothing to compare against.
    expect(proposeContentPinMigration(manifest(1), {})).toBeNull();
  });

  it('refuses to re-pin a pack built for different content', () => {
    // Not out of date — for something else. Re-pinning would quietly claim it
    // works here.
    expect(proposeContentPinMigration(manifest(1, { id: 'other-content' }),
      { contentId: 'phoenix-base', contentEpoch: 4 })).toBeNull();
  });

  it('never moves a pin backwards', () => {
    // A pack pinned ahead was built against something newer; dragging it back
    // would lose the authored intent rather than repair it.
    expect(proposeContentPinMigration(manifest(9), { contentId: 'phoenix-base', contentEpoch: 4 })).toBeNull();
  });
});

describe('accepting a proposal', () => {
  it('is one undoable history entry over the exact reviewed text', () => {
    const draft = new WorkshopDocument(pack(manifest(1)));
    const proposed = proposeWorkshopMigration(draft, { contentId: 'phoenix-base', contentEpoch: 4 });
    expect(proposed.path).toBe('scenarios.toml');
    // The draft is untouched until the operator accepts: proposing is not
    // applying.
    expect(draft.read('scenarios.toml')).toBe(proposed.before);
    expect(draft.canUndo()).toBe(false);

    draft.apply([{ path: proposed.path, before: proposed.before, after: proposed.after }]);
    expect(draft.read('scenarios.toml')).toBe(proposed.after);
    // ONE press puts it back, not one press per member.
    draft.undo();
    expect(draft.read('scenarios.toml')).toBe(proposed.before);
    expect(draft.canUndo()).toBe(false);
  });

  it('survives crash recovery with the accepted bytes intact', () => {
    const draft = new WorkshopDocument(pack(manifest(1)));
    const proposed = proposeWorkshopMigration(draft, { contentId: 'phoenix-base', contentEpoch: 4 });
    draft.apply([{ path: proposed.path, before: proposed.before, after: proposed.after }]);
    const recovered = WorkshopDocument.restore(draft.snapshot());
    expect(recovered.read('scenarios.toml')).toBe(proposed.after);
    expect(recovered.undo()).toBe('scenarios.toml');
    expect(recovered.read('scenarios.toml')).toBe(proposed.before);
  });
});
