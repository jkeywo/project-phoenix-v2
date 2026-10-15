// @vitest-environment jsdom
import { expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopWorldRoutes } from '../../gui/workshop-world-routes-panel.js';
const path = 'assets/worlds/root.toml', outer = 'assets/worlds/outer.toml';
function fixture() {
  document.body.innerHTML = '<main></main>';
  const draft = new WorkshopDocument(createStoreZip([
    { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="routes"\nname="Routes"\nversion="1"\n[[scenario]]\nid="root"\nworld="assets/worlds/root.toml"\n' },
    { path, text: '[anchors]\nhome=[0,0,0]\n' },
  ]));
  const dependencies = { base_files: { [outer]: '[anchors]\narrival=[100,42,0]\n' }, packs: [] };
  const runtime = { validate: vi.fn(async () => ({ accepted: true })), compose: vi.fn(async (files, request) => {
    const value = parse(files[path]); value.transit_destination = parse('value=' + request.edits[0].value_source).value;
    return stringify(value);
  }) };
  let held = false;
  const changed = vi.fn(), panel = mountWorkshopWorldRoutes({ root: document.querySelector('main'), draft: () => draft,
    dependencies: () => dependencies, runtime, busy: () => held, setBusy: value => { held = value; }, changed });
  const el = id => document.getElementById('workshop-world-routes-' + id);
  const change = (id, value) => { el(id).value = value; el(id).dispatchEvent(new Event(el(id).tagName === 'INPUT' ? 'input' : 'change')); };
  const fill = () => { change('id', 'outer'); change('label', 'world.transit.outer'); change('target', outer); };
  return { panel, draft, runtime, changed, el, fill, dependencies, held: () => held };
}
it('edits only a selectable draft root and references actual read-only target anchors', async () => {
  const f = fixture();
  expect([...f.el('world').options].map(row => row.value)).toEqual([path]);
  f.fill(); expect(f.el('anchor').value).toBe('arrival'); f.el('add').click();
  await vi.waitFor(() => expect(f.changed).toHaveBeenCalledWith(path));
  expect(f.runtime.compose).toHaveBeenCalledOnce(); expect(f.runtime.validate).toHaveBeenCalledOnce();
  expect(parse(f.draft.read(path)).transit_destination[0]).toEqual({ id: 'outer', label: 'world.transit.outer',
    target: { kind: 'layer', path: outer, anchor: 'arrival' } });
  expect(f.draft.read(outer)).toBeUndefined();
  f.draft.undo(); f.panel.refresh(); expect(f.el('route').options).toHaveLength(0);
});
it('shows backend refusal, releases busy state and preserves the accepted document', async () => {
  const f = fixture(), before = f.draft.snapshot(); f.fill();
  f.runtime.validate.mockResolvedValue({ accepted: false, findings: [{ message: 'selected gravity scope is unsupported' }] });
  f.el('add').click();
  await vi.waitFor(() => expect(f.el('status').getAttribute('role')).toBe('alert'));
  expect(f.el('status').textContent).toContain('selected gravity scope is unsupported');
  expect(f.held()).toBe(false); expect(f.changed).not.toHaveBeenCalled(); expect(f.draft.snapshot()).toEqual(before);
});
it('cannot land a pending route edit after disposal', async () => {
  const f = fixture(), before = f.draft.snapshot(); f.fill(); let finish;
  f.runtime.compose.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  f.el('add').click(); await vi.waitFor(() => expect(finish).toBeTypeOf('function'));
  f.panel.dispose(); finish('[anchors]\nhome=[0,0,0]\n');
  await vi.waitFor(() => expect(f.held()).toBe(false));
  expect(f.draft.snapshot()).toEqual(before); expect(f.changed).not.toHaveBeenCalled();
});
