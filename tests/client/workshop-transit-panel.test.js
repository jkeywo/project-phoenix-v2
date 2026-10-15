/** @vitest-environment jsdom */
import fs from 'node:fs';
import { posix } from 'node:path';
import { expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopTransit } from '../../gui/workshop-transit-panel.js';
const path = 'assets/entities/test-jump.toml', control = id => document.getElementById('workshop-transit-' + id);
const settle = async () => { for (let i = 0; i < 20; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
function fixture(child = false) {
  const base = 'assets/entities/local_jump_cruiser.toml', base_files = {};
  const collect = file => { base_files[file] = fs.readFileSync(file, 'utf8');
    for (const include of parse(base_files[file]).includes || []) collect(posix.normalize(posix.join(posix.dirname(file), include))); }; collect(base);
  const source = child ? '# own child\nincludes=["local_jump_cruiser.toml"]\n' : base_files[base];
  const make = text => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text }]));
  let draft = make(source); document.body.innerHTML = '<main></main>';
  const runtime = { dependencies: async () => ({ base_files, packs: [] }), validate: vi.fn(async () => ({ accepted: true })) };
  const preview = vi.fn(() => true), test = vi.fn(() => true), changed = vi.fn(); let busy = false;
  const panel = mountWorkshopTransit({ root: document.querySelector('main'), runtime, draft: () => draft,
    busy: () => busy, setBusy: value => { busy = value; }, preview, test, changed });
  return { panel, get draft() { return draft; }, source, runtime, preview, test, changed, replace(text) { draft = make(text); } };
}
it('saves actual finite drive fields with stable focus and normal Preview/Test/history', async () => {
  const f = fixture(); await settle();
  expect(control('owner').value).toBe('local-jump'); expect(control('fuel').value).toBe('local-transit-fuel');
  const distance = control('max_distance'); distance.focus(); distance.value = '400'; f.panel.refresh();
  expect(control('max_distance')).toBe(distance); expect(document.activeElement).toBe(distance); expect(distance.value).toBe('400');
  control('save').click(); await vi.waitFor(() => expect(f.runtime.validate).toHaveBeenCalledOnce()); await settle();
  expect(parse(f.draft.read(path)).system[0].component[1].config.max_distance).toBe(400);
  f.draft.undo(); f.panel.refresh(); expect(control('max_distance').value).toBe('500'); f.draft.redo(); f.panel.refresh();
  control('preview').click(); control('test').click(); expect(f.preview).toHaveBeenCalledWith(path); expect(f.test).toHaveBeenCalledWith(path); f.panel.dispose();
});
it('requires inherited consent and preserves the draft on actual acceptance refusal', async () => {
  const f = fixture(true); await settle(); control('max_distance').value = '300'; control('save').click(); await settle();
  expect(f.draft.read(path)).toBe(f.source); expect(f.runtime.validate).not.toHaveBeenCalled();
  control('materialise').checked = true; f.runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'real owner refused' }] });
  control('save').click(); await vi.waitFor(() => expect(control('status').textContent).toContain('real owner refused'));
  expect(f.draft.snapshot().history.undo).toHaveLength(0); control('save').click(); await settle(); expect(f.draft.snapshot().history.undo).toHaveLength(1); f.panel.dispose();
});
it('refreshes a replacement document of the same revision and refuses disposed completion', async () => {
  const f = fixture(); await settle();
  f.replace(f.source.replace('max_distance = 500', 'max_distance = 250')); f.panel.refresh();
  expect(control('max_distance').value).toBe('250');
  let answer; f.runtime.validate.mockImplementationOnce(() => new Promise(resolve => { answer = resolve; }));
  control('max_distance').value = '200'; control('save').click(); await vi.waitFor(() => expect(answer).toBeTypeOf('function'));
  f.panel.dispose(); answer({ accepted: true }); await settle();
  expect(f.draft.snapshot().history.undo).toHaveLength(0); expect(f.draft.read(path)).toContain('max_distance = 250');
});

it('refuses an old pending validation after the active same-revision document is replaced', async () => {
  const f = fixture(); await settle(); const old = f.draft;
  let answer; f.runtime.validate.mockImplementationOnce(() => new Promise(resolve => { answer = resolve; }));
  control('max_distance').value = '200'; control('save').click(); await vi.waitFor(() => expect(answer).toBeTypeOf('function'));
  f.replace(f.source.replace('max_distance = 500', 'max_distance = 250'));
  expect(f.draft.sourceRevision).toBe(old.sourceRevision);
  answer({ accepted: true }); await settle();
  expect(old.snapshot().history.undo).toHaveLength(0); expect(f.draft.snapshot().history.undo).toHaveLength(0);
  expect(f.changed).not.toHaveBeenCalled(); expect(old.read(path)).toBe(f.source);
  expect(f.draft.read(path)).toContain('max_distance = 250'); f.panel.dispose();
});
