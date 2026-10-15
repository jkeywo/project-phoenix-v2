import { expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { inspectWorldRoutes, applyWorldRouteOperation } from '../workshop-world-routes.js';
const root = 'assets/worlds/root.toml', outer = 'assets/worlds/outer.toml';
export function routeFixture() {
  const draft = new WorkshopDocument(createStoreZip([
    { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="routes"\nname="Routes"\nversion="1"\n[[scenario]]\nid="root"\nworld="assets/worlds/root.toml"\n' },
    { path: root, text: '# keep root\n[anchors]\nhome=[0,0,0]\n' },
  ]));
  const dependencies = { base_files: { [outer]: '[anchors]\narrival=[100,42,0]\ncollision=[0,9,0]\n' }, packs: [] };
  // This test adapter exercises packet planning/history; the Rust owner test
  // separately checks the real source-preserving editor.
  const runtime = { validate: vi.fn(async () => ({ accepted: true })), compose: vi.fn(async (files, request) => {
    const world = parse(files[request.document_path]);
    for (const edit of request.edits) {
      const value = edit.value_source === undefined ? undefined : parse('value=' + edit.value_source).value;
      if (edit.op === 'append_table') {
        (world.transit_destination ||= []).push(Object.fromEntries(edit.fields.map(([key, source]) => [key, parse('value=' + source).value])));
      } else if (edit.path.length === 1) {
        if (edit.op === 'put') world.transit_destination = value;
        else world.transit_destination.splice(edit.index, 0, value);
      } else if (edit.op === 'remove' && edit.path.length === 2) world.transit_destination.splice(edit.path[1], 1);
      else {
        let parent = world;
        for (const key of edit.path.slice(0, -1)) parent = parent[key];
        const key = edit.path.at(-1);
        if (edit.op === 'remove') delete parent[key]; else parent[key] = value;
      }
    }
    return stringify(world);
  }) };
  return { draft, dependencies, runtime };
}
const add = { type: 'route-add', path: root, id: 'outer', label: 'world.transit.outer', target: { kind: 'layer', path: outer, anchor: 'arrival' } };
it('reads actual selectable roots and finite read-only dependency anchors', () => {
  const f = routeFixture(), read = inspectWorldRoutes(f.draft, f.dependencies, root);
  expect(read.editable).toBe(true); expect(read.roots.map(row => row.path)).toEqual([root]);
  expect(read.worlds.find(row => row.path === outer).anchors[0]).toEqual({ name: 'arrival', point: [100, 42, 0] });
  expect(inspectWorldRoutes(f.draft, f.dependencies, outer).editable).toBe(false);
});
it('adds, edits and removes through actual compose packets, accepted history and export', async () => {
  const f = routeFixture(), before = f.draft.read(root);
  await applyWorldRouteOperation({ ...f, operation: add });
  expect(f.runtime.compose.mock.calls[0][1].edits[0]).toMatchObject({ op: 'append_table', path: ['transit_destination'] });
  const inserted = f.draft.read(root);
  f.draft.undo(); expect(f.draft.read(root)).toBe(before); f.draft.redo(); expect(f.draft.read(root)).toBe(inserted);
  await applyWorldRouteOperation({ ...f, operation: { ...add, type: 'route-update', original: 'outer', id: 'new-id' } });
  expect(f.runtime.compose.mock.lastCall[1].edits.map(row => row.path)).toEqual([
    ['transit_destination', 0, 'id'], ['transit_destination', 0, 'label'],
    ['transit_destination', 0, 'target', 'kind'], ['transit_destination', 0, 'target', 'path'], ['transit_destination', 0, 'target', 'anchor'],
  ]);
  expect(inspectWorldRoutes(new WorkshopDocument(f.draft.archive()), f.dependencies, root).routes[0].id).toBe('new-id');
  await applyWorldRouteOperation({ ...f, operation: { ...add, type: 'route-update', original: 'new-id', id: 'new-id', target: { kind: 'base', anchor: 'home' } } });
  expect(inspectWorldRoutes(f.draft, f.dependencies, root).routes[0].target).toEqual({ kind: 'base', anchor: 'home' });
  await applyWorldRouteOperation({ ...f, operation: { type: 'route-remove', path: root, original: 'new-id' } });
  expect(inspectWorldRoutes(f.draft, f.dependencies, root).routes).toEqual([]);
});
it('refuses duplicate x/z, unsupported targets and non-root mutation before compose', async () => {
  const f = routeFixture();
  await applyWorldRouteOperation({ ...f, operation: { ...add, id: 'home', target: { kind: 'base', anchor: 'home' } } });
  const before = f.draft.snapshot(), calls = f.runtime.compose.mock.calls.length;
  for (const operation of [
    { ...add, target: { kind: 'layer', path: outer, anchor: 'collision' } },
    { ...add, target: { kind: 'layer', path: outer, anchor: 'missing' } },
    { ...add, id: 'home' }, { ...add, path: outer },
    { ...add, target: { kind: 'layer', path: 'assets/worlds/../outer.toml', anchor: 'arrival' } },
  ]) await expect(applyWorldRouteOperation({ ...f, operation })).rejects.toThrow();
  expect(f.runtime.compose).toHaveBeenCalledTimes(calls); expect(f.draft.snapshot()).toEqual(before);
});
it('refuses failed and stale validation without touching accepted history', async () => {
  const f = routeFixture(), before = f.draft.snapshot();
  f.runtime.validate.mockResolvedValue({ accepted: false, findings: [{ message: 'bad selected scope' }] });
  await expect(applyWorldRouteOperation({ ...f, operation: add })).rejects.toThrow('runtime-validation-refused');
  expect(f.draft.snapshot()).toEqual(before);
  f.runtime.validate.mockResolvedValue({ accepted: true });
  await expect(applyWorldRouteOperation({ ...f, operation: add, current: () => false })).rejects.toThrow('stale-world-route-operation');
  expect(f.draft.snapshot()).toEqual(before);
});
