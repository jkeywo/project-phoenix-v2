import { describe, expect, it, vi } from 'vitest';
import { parse } from 'smol-toml';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { applyShipOperation, inspectShipAuthoring, prepareShipOperation } from '../../editor/workshop-ship-authoring.js';

const path = 'assets/entities/ship.toml';
const source = `# Keep my comments
 tags=["ship"]
[[station]]
id="bridge"
name="Bridge"
[[station.rating]]
name="Assisted"
automated_systems=["drive"]
[[console]]
id="flight"
home_station="bridge"
screen="gui/helm.html"
[[console]]
id="navigator"
home_station="bridge"
screen="gui/helm.html"
[[system]]
id="drive"
kind="helm_thrust"
console="flight"
`;
const dependencies = { base_files: {}, packs: [] };
const schema = { system_kinds: ['helm_thrust'], directive_kinds: [], console_feed_kinds: ['HelmEngine', 'Sensors'] };
const draftOf = (text = source) => new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nversion="1"\nname="Mine"\n[pack.requires]\ncontent_id="phoenix-base"\ncontent_epoch=1\n' },
  { path, text },
]));
const prepare = (draft, operation) => prepareShipOperation(draft, dependencies, schema, { path, ...operation });

describe('Console authoring', () => {
  it('supports independent identities and several Consoles on the same Station/screen', () => {
    const draft = draftOf();
    const result = prepare(draft, { type: 'console-update', id: 'navigator', fields: { name: 'Navigation', human_seeking: true, host_order: ['bridge'] } });
    const value = parse(result.changes[0].after);
    expect(value.console.map(row => row.id)).toEqual(['flight', 'navigator']);
    expect(value.console.every(row => row.home_station === 'bridge')).toBe(true);
    expect(value.system[0].console).toBe('flight');
    expect(result.changes[0].after.startsWith('# Keep my comments')).toBe(true);
  });
  it('creates, reorders and removes Consoles without rewriting neighbouring source', () => {
    const draft = draftOf();
    draft.apply(prepare(draft, { type: 'console-add', id: 'spare', fields: { home_station: 'bridge' } }).changes);
    draft.apply(prepare(draft, { type: 'console-move', id: 'spare', direction: -1 }).changes);
    expect(parse(draft.read(path)).console.map(row => row.id)).toEqual(['flight', 'spare', 'navigator']);
    draft.apply(prepare(draft, { type: 'console-remove', id: 'spare' }).changes);
    expect(parse(draft.read(path)).console.map(row => row.id)).toEqual(['flight', 'navigator']);
    expect(draft.read(path)).toContain('# Keep my comments');
  });
  it.each([
    [{ type: 'console-add', id: 'flight', fields: {} }, /unique/],
    [{ type: 'console-update', id: 'flight', fields: { home_station: 'missing' } }, /home Station/],
    [{ type: 'console-update', id: 'flight', fields: { host_order: ['missing'], human_seeking: true } }, /host Station/],
    [{ type: 'system-update', id: 'drive', fields: { console: 'missing' } }, /unknown Console/],
    [{ type: 'console-remove', id: 'flight' }, /unknown Console/],
    [{ type: 'console-feed-add', id: 'flight', source: 'missing', kind: 'Sensors' }, /feed source/],
    [{ type: 'console-feed-add', id: 'flight', source: 'drive', kind: 'Everything' }, /feed kind/],
  ])('refuses invalid composition %j without changing the draft', (operation, message) => {
    const draft = draftOf();
    expect(() => prepare(draft, operation)).toThrow(message);
    expect(draft.read(path)).toBe(source);
  });
  it('roundtrips typed feeds through accepted edits, undo/redo and draft recovery', async () => {
    const draft = draftOf();
    const runtime = { shipSchema: async () => schema, validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    await applyShipOperation({ draft, runtime, dependencies, operation: { path, type: 'console-feed-add', id: 'navigator', source: 'drive', kind: 'HelmEngine' } });
    const accepted = draft.read(path);
    expect(runtime.validate).toHaveBeenCalledOnce();
    expect(parse(accepted).console[1].feed).toEqual([{ source: 'drive', kind: 'HelmEngine' }]);
    draft.undo(); expect(draft.read(path)).toBe(source);
    draft.redo(); expect(draft.read(path)).toBe(accepted);
    const restored = WorkshopDocument.restore(draft.snapshot());
    expect(inspectShipAuthoring(restored, dependencies, path).consoles[1].feed).toEqual([{ source: 'drive', kind: 'HelmEngine' }]);
    restored.apply(prepare(restored, { type: 'console-feed-remove', id: 'navigator', index: 0 }).changes);
    expect(parse(restored.read(path)).console[1].feed).toBeUndefined();
  });
  it('refuses inherited Console edits instead of appending a shadow definition', () => {
    const draft = draftOf('includes=["base.toml"]\ntags=["ship"]\n');
    const inherited = { base_files: { 'assets/entities/base.toml': source }, packs: [] };
    expect(() => prepareShipOperation(draft, inherited, schema, { path, type: 'console-update', id: 'flight', fields: { name: 'Flight' } })).toThrow(/included from another source/);
  });
  it('adds a feed when the last Console has no trailing newline', () => {
    const draft = draftOf(source + '\n[[console]]\nid="last"');
    const result = prepare(draft, { type: 'console-feed-add', id: 'last', source: 'drive', kind: 'HelmEngine' });
    expect(parse(result.changes[0].after).console.at(-1).feed).toEqual([{ source: 'drive', kind: 'HelmEngine' }]);
  });
  it('preserves AI-only control independently from screenless Console ownership', () => {
    const draft = draftOf();
    draft.apply(prepare(draft, { type: 'console-add', id: 'machinery', fields: {} }).changes);
    draft.apply(prepare(draft, { type: 'system-add', id: 'autodrive', kind: 'helm_thrust', console: 'machinery', ai_only: true }).changes);
    expect(parse(draft.read(path)).system[1]).toMatchObject({ console: 'machinery', ai_only: true });
    expect(() => prepare(draft, { type: 'system-update', id: 'autodrive', fields: { console: undefined } })).toThrow(/unknown Console/);
  });
  it('runtime rejection leaves exact source and history unchanged', async () => {
    const draft = draftOf();
    await expect(applyShipOperation({ draft, dependencies, runtime: { shipSchema: async () => schema, validate: async () => ({ accepted: false, findings: [] }) },
      operation: { path, type: 'console-update', id: 'flight', fields: { name: 'Changed' } } })).rejects.toThrow('runtime-validation-refused');
    expect(draft.read(path)).toBe(source); expect(draft.canUndo()).toBe(false);
  });
  it('edits detached Console ratings independently from the hosting Station', () => {
    const draft = draftOf(source + '\n[[console]]\nid="aux"\nhuman_seeking=true\nhost_order=["bridge"]\n[[system]]\nid="auxdrive"\nkind="helm_thrust"\nconsole="aux"\n');
    draft.apply(prepare(draft, { type: 'rating-add', console: 'aux', name: 'Std' }).changes);
    draft.apply(prepare(draft, { type: 'rating-add', console: 'aux', name: 'Simplified' }).changes);
    draft.apply(prepare(draft, { type: 'rating-update', console: 'aux', rating: 'Simplified', name: 'Simplified', systems: ['auxdrive'] }).changes);
    expect(parse(draft.read(path)).console.at(-1).rating[1].automated_systems).toEqual(['auxdrive']);
    expect(parse(draft.read(path)).station[0].rating[0].automated_systems).toEqual(['drive']);
    expect(() => prepare(draft, { type: 'rating-set-systems', console: 'aux', rating: 'Std', systems: ['drive'] })).toThrow(/owned by Console aux/);
    draft.apply(prepare(draft, { type: 'rating-move', console: 'aux', rating: 'Simplified', direction: -1 }).changes);
    expect(parse(draft.read(path)).console.at(-1).rating[0].name).toBe('Simplified');
    draft.apply(prepare(draft, { type: 'rating-remove', console: 'aux', rating: 'Std' }).changes);
    expect(parse(draft.read(path)).console.at(-1).rating).toHaveLength(1);
  });
  it('accepts references only to generated arcs actually authored on this hull', () => {
    const draft = draftOf(source + '\n[[system]]\nid="shield-controller"\nkind="shields"\nconsole="flight"\n[[shield_arc]]\nid="fore"\n');
    const withShields = { ...schema, system_kinds: [...schema.system_kinds, 'shields'], console_feed_kinds: [...schema.console_feed_kinds, 'ShieldArc'] };
    const result = prepareShipOperation(draft, dependencies, withShields, { path, type: 'console-feed-add', id: 'flight', source: 'shield-arc-fore', kind: 'ShieldArc' });
    expect(result.changes).toHaveLength(1);
    expect(() => prepareShipOperation(draft, dependencies, withShields, { path, type: 'console-feed-add', id: 'flight', source: 'shield-arc-missing', kind: 'ShieldArc' })).toThrow(/feed source/);
  });

  it('edits and reorders a replacement Console without modifying its removal marker', () => {
    const replacement = source.replace('[[console]]\nid="flight"', '[[console]]\nid="flight"\n_remove=true\n[[console]]\nid="flight"');
    const draft = draftOf(replacement);
    draft.apply(prepare(draft, { type: 'console-update', id: 'flight', fields: { name: 'Replacement' } }).changes);
    expect(parse(draft.read(path)).console[0]).toEqual({ id: 'flight', _remove: true });
    expect(parse(draft.read(path)).console[1].name).toBe('Replacement');
    draft.apply(prepare(draft, { type: 'console-move', id: 'flight', direction: 1 }).changes);
    expect(parse(draft.read(path)).console.map(row => row.id)).toEqual(['flight', 'navigator', 'flight']);
    expect(parse(draft.read(path)).console[0]._remove).toBe(true);
  });

  it.each([
    [{ type: 'console-update', id: 'flight', fields: { host_order: ['bridge', 'bridge'], human_seeking: true } }, /repeats a host/],
    [{ type: 'console-update', id: 'flight', fields: { host_order: ['bridge'] } }, /Fixed Console/],
    [{ type: 'console-update', id: 'flight', fields: { visiting_rating: 'Absent', human_seeking: true } }, /unknown visiting rating/],
    [{ type: 'rating-add', console: 'flight', name: '' }, /nonempty and unique/],
  ])('rejects invalid hosting/rating edits %j before acceptance', (operation, message) => {
    const draft = draftOf(); expect(() => prepare(draft, operation)).toThrow(message); expect(draft.read(path)).toBe(source);
  });
  it('keeps visiting references in step with detached and home-Station rating renames', () => {
    const draft = draftOf();
    draft.apply(prepare(draft, { type: 'console-update', id: 'flight', fields: { human_seeking: true, visiting_rating: 'Assisted' } }).changes);
    draft.apply(prepare(draft, { type: 'rating-update', station: 'bridge', rating: 'Assisted', name: 'Guided', systems: ['drive'] }).changes);
    expect(parse(draft.read(path)).console[0].visiting_rating).toBe('Guided');
    draft.apply(prepare(draft, { type: 'rating-add', console: 'flight', name: 'Guided' }).changes);
    expect(() => prepare(draft, { type: 'rating-add', console: 'flight', name: 'Guided' })).toThrow(/nonempty and unique/);
    draft.apply(prepare(draft, { type: 'rating-update', console: 'flight', rating: 'Guided', name: 'Manual', systems: [] }).changes);
    expect(parse(draft.read(path)).console[0].visiting_rating).toBe('Manual');
    expect(() => prepare(draft, { type: 'rating-remove', console: 'flight', rating: 'Manual' })).toThrow(/choose another visiting rating/);
  });

});
