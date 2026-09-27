// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createGmEntityTree } from '../../gui/gm-entity-tree.js';
import { setBaseCatalogue, setOverlayCatalogues, setLocale, t } from '../../gui/strings.js';

describe('keyed GM entity tree', () => {
  let root, tree, onSelect;
  const entities = [{ entity_id: 'one', name: 'Resolute', faction: { entity_id: 'f1', name: 'Alliance' } }];
  const state = () => ({ entities, worlds: { root: { label: 'Main world' }, empty: { label: 'Empty layer' } },
    membership: { one: 'root' }, stations: [{ ship_id: 'one', stations: [{ station_id: 'helm', name: 'Helm', rating: 'Backfill' }] }],
    slots: [{ id: 'slot1', label: 'Empty player slot', state: 'empty', can_backfill: true }] });
  beforeEach(() => {
    document.body.innerHTML = '<section></section>'; root = document.querySelector('section'); onSelect = vi.fn();
    tree = createGmEntityTree({ root, doc: document, t: id => id, onSelect });
  });
  const button = text => [...root.querySelectorAll('button')].find(node => node.textContent === text);
  it('includes empty worlds, stations, player slots and unassigned entities', () => {
    tree.update({ ...state(), entities: [...entities, { entity_id: 'runtime', name: 'Runtime' }] });
    expect(button('Empty layer')).toBeTruthy();
    expect(button('Helm · station.rating.backfill.name')).toBeTruthy();
    expect(button('server.gm.tree.unassigned')).toBeTruthy();
    button('Empty player slot').click(); expect(onSelect).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'slot' }));
    button('Helm · station.rating.backfill.name').click(); expect(onSelect).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'station' }));
  });
  it('retains row identity and selection through updates and faction moves', () => {
    tree.update(state()); tree.selectEntity('one');
    const row = button('Resolute').closest('[role=treeitem]');
    root.scrollTop = 23;
    tree.update({ ...state(), entities: [{ ...entities[0], faction: null }] });
    expect(button('Resolute').closest('[role=treeitem]')).toBe(row);
    expect(row.getAttribute('aria-selected')).toBe('true');
    expect(root.scrollTop).toBe(23);
    expect(row.parentElement.parentElement.textContent).toContain('server.gm.tree.no_faction');
  });
  it('does not mutate rows or reopen collapsed ancestors for unchanged selection', () => {
    tree.update(state()); tree.selectEntity('one');
    const world = button('Main world').closest('[role=treeitem]');
    world.querySelector('button').click();
    const observer = new MutationObserver(() => {});
    observer.observe(root, {subtree:true, attributes:true, childList:true, characterData:true});
    tree.selectEntity('one'); tree.update(state());
    expect(observer.takeRecords()).toHaveLength(0);
    expect(world.getAttribute('aria-expanded')).toBe('false');
    observer.disconnect();
  });
  it('reveals matching ancestors during search without losing manual expansion', () => {
    tree.update(state());
    const search = root.querySelector('input'); search.value = 'Helm'; search.dispatchEvent(new Event('input'));
    expect(button('Helm · station.rating.backfill.name').closest('[hidden]')).toBeNull();
    expect(button('Empty layer').closest('[hidden]')).not.toBeNull();
    search.value = ''; search.dispatchEvent(new Event('input'));
    expect(button('Empty layer').closest('[hidden]')).toBeNull();
    expect(button('Helm · station.rating.backfill.name').closest('[hidden]')).not.toBeNull();
  });
  it('supports keyboard navigation and removes despawned rows', () => {
    tree.update(state());
    const row = root.querySelector('[role=treeitem]'); row.focus();
    row.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    expect(document.activeElement).not.toBe(row);
    tree.update({ ...state(), entities: [] }); expect(button('Resolute')).toBeUndefined();
  });

  it('repaints authored ship and station ids without changing selection or literal names', () => {
    const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
    setBaseCatalogue(fs.readFileSync(path.join(repo, 'assets/strings/strings.csv'), 'utf8'));
    setOverlayCatalogues([{ source: 'gm-tree-de', csv: 'id,de,de_source\n'
      + 'entity.alliance_cruiser.display_name,AEV Phönix,[AEV Phoenix]\n'
      + 'station.helm.name,Ruder,Helm\n'
      + 'station.rating.backfill.name,KI-Besatzung,BACKFILL (AI)\n'
      + 'server.gm.tree.search,Einheiten und Stationen suchen,Search entities and stations\n'
      + 'server.gm.tree.title,Einheitenbaum,Entity Tree\n' }]);
    setLocale('en');
    tree = createGmEntityTree({ root, doc: document, t, onSelect });
    const raw = { entities: [{ entity_id: 'one', name: 'entity.alliance_cruiser.display_name',
      faction: { entity_id: 'f1', name: 'Captain’s own faction' } }],
    worlds: { root: { label: 'world.falling_skyway.global.title' } }, membership: { one: 'root' },
    stations: [{ ship_id: 'one', stations: [{ station_id: 'helm', name: 'station.helm.name', rating: 'Backfill' }] }],
    slots: [] };
    tree.update(raw);
    tree.selectEntity('one');
    expect(button('[AEV Phoenix]')).toBeTruthy();
    setLocale('de');
    tree.update(raw);
    expect(button('AEV Phönix').closest('[role=treeitem]').getAttribute('aria-selected')).toBe('true');
    expect(button('Ruder · KI-Besatzung')).toBeTruthy();
    expect(root.querySelector('input').placeholder).toBe('Einheiten und Stationen suchen');
    expect(root.querySelector('[role="tree"]').getAttribute('aria-label')).toBe('Einheitenbaum');
    expect(button('Captain’s own faction')).toBeTruthy();
    expect(raw.entities[0].name).toBe('entity.alliance_cruiser.display_name');
    expect(raw.stations[0].stations[0].station_id).toBe('helm');
    setOverlayCatalogues([]);
    setLocale('en');
  });
});
