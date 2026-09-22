// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createGmEntityTree } from '../../gui/gm-entity-tree.js';

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
    expect(button('Helm · Backfill')).toBeTruthy();
    expect(button('server.gm.tree.unassigned')).toBeTruthy();
    button('Empty player slot').click(); expect(onSelect).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'slot' }));
    button('Helm · Backfill').click(); expect(onSelect).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'station' }));
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
    expect(button('Helm · Backfill').closest('[hidden]')).toBeNull();
    expect(button('Empty layer').closest('[hidden]')).not.toBeNull();
    search.value = ''; search.dispatchEvent(new Event('input'));
    expect(button('Empty layer').closest('[hidden]')).toBeNull();
    expect(button('Helm · Backfill').closest('[hidden]')).not.toBeNull();
  });
  it('supports keyboard navigation and removes despawned rows', () => {
    tree.update(state());
    const row = root.querySelector('[role=treeitem]'); row.focus();
    row.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    expect(document.activeElement).not.toBe(row);
    tree.update({ ...state(), entities: [] }); expect(button('Resolute')).toBeUndefined();
  });
});
