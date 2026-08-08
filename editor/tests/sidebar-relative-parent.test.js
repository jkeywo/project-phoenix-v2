// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest';
import { PropertiesPanel } from '../sidebar.js';
import { getRelativeInfo } from '../toml-utils.js';

// Issue #969 made an unresolvable `relative_to` block the whole world, so the
// Relative To picker was narrowed to only what the runtime lookup table will
// contain. These cover the two ways that narrowing can go wrong in the pane:
// reading an already-authored reference back as unresolved, and offering the
// mode at all when the layer has nothing to be relative TO.

function mountPane() {
  document.body.innerHTML = '<div id="propertiesPanelContent"></div>';
}

function makePanel(layer) {
  const canvasManager = {
    spawnGroups: new Map(),
    renderAll() {},
    updateArrows() {},
    deselectSpawn() {},
    buildV2Layers() { return [layer]; },
  };
  const layerManager = { getLayers() { return [layer]; } };
  const undoController = { snapshots: 0, snapshotForUndo() { this.snapshots += 1; } };
  return {
    panel: new PropertiesPanel(canvasManager, layerManager, undoController),
    undoController,
  };
}

const layerOf = (...entity) => ({ isMap: true, path: 'worlds/test.toml', toml: { entity } });

function parentSelect() {
  return document.getElementById('propParent');
}

function relativeRadio() {
  return document.querySelector('input[name="posMode"][value="relative"]');
}

beforeEach(() => {
  mountPane();
});

// ── Finding 1: an `id`-authored reference must resolve ────────────────────────

describe('Relative To picker recognises a reference authored against `id`', () => {
  // Exactly combat_test.toml's shape: the landmark carries a short `id` for
  // authors and a strings.csv key as `name`, and the reference names the `id`.
  // default.toml's earth/luna pair is identical.
  const gasGiant = () => ({
    template_path: 'assets/entities/planet_gas_giant.toml',
    id: 'gas-giant',
    name: 'world.entity.gas_giant.name',
    transform: { position: [-1200, 0, 300] },
  });
  const iceMoon = () => ({
    template_path: 'assets/entities/moon_ice.toml',
    id: 'ice-moon',
    name: 'world.entity.ice_moon.name',
    transform: { relative_to: 'gas-giant', offset: [125, 0, 40] },
  });

  it('marks the real parent selected, with no unresolved pseudo-option', () => {
    const [parent, moon] = [gasGiant(), iceMoon()];
    const layer = layerOf(parent, moon);
    makePanel(layer).panel.render(moon, layer);

    const select = parentSelect();
    expect(select).not.toBeNull();
    expect(select.innerHTML).not.toContain('(unresolved)');

    const options = [...select.options];
    expect(options).toHaveLength(1);
    expect(options[0].selected).toBe(true);
    // The picker resolved the reference; it did not silently re-point it.
    expect(moon.transform.relative_to).toBe('gas-giant');
  });

  it('still selects the parent when the reference names its `name` instead', () => {
    const parent = gasGiant();
    const moon = iceMoon();
    moon.transform.relative_to = 'world.entity.gas_giant.name';
    const layer = layerOf(parent, moon);
    makePanel(layer).panel.render(moon, layer);

    const select = parentSelect();
    expect(select.innerHTML).not.toContain('(unresolved)');
    expect([...select.options].filter(o => o.selected)).toHaveLength(1);
  });

  it('still flags a reference no spawn in the layer answers to', () => {
    // The preserve-rather-than-re-point behaviour must survive the fix.
    const parent = gasGiant();
    const moon = iceMoon();
    moon.transform.relative_to = 'no-such-landmark';
    const layer = layerOf(parent, moon);
    makePanel(layer).panel.render(moon, layer);

    const select = parentSelect();
    expect(select.innerHTML).toContain('(unresolved)');
    expect(select.value).toBe('no-such-landmark');
    // The real candidate is still offered, just not selected.
    expect([...select.options]).toHaveLength(2);
  });
});

// ── Finding 2: no base means no `relative` mode ───────────────────────────────

describe('Relative To is unavailable when the layer offers no base', () => {
  // The only id/name-bearing, non-relative_to-positioned entity in its layer:
  // `getRelativeToCandidates` excludes the subject itself, so nothing remains.
  const loneSpawn = () => ({
    template_path: 'assets/entities/station_axiom.toml',
    id: 'lonely',
    transform: { position: [10, 0, 20] },
  });

  it('disables the radio rather than offering a mode it cannot satisfy', () => {
    const spawn = loneSpawn();
    const layer = layerOf(spawn);
    makePanel(layer).panel.render(spawn, layer);

    expect(relativeRadio().disabled).toBe(true);
    // Absolute and Anchor are still available.
    expect(document.querySelector('input[name="posMode"][value="absolute"]').disabled).toBe(false);
    expect(document.querySelector('input[name="posMode"][value="anchor"]').disabled).toBe(false);
  });

  it('writes no base-less transform even if the change event is forced', () => {
    const spawn = loneSpawn();
    const layer = layerOf(spawn);
    const { panel, undoController } = makePanel(layer);
    panel.render(spawn, layer);

    const radio = relativeRadio();
    radio.checked = true;
    radio.dispatchEvent(new Event('change', { bubbles: true }));

    // The transform is untouched: no `offset` with nothing to offset from.
    expect(spawn.transform).toEqual({ position: [10, 0, 20] });
    expect(spawn.transform.relative_to).toBeUndefined();
    expect(spawn.transform.offset).toBeUndefined();
    expect(getRelativeInfo(spawn)).toBeNull();
    // A refused switch costs no undo entry either.
    expect(undoController.snapshots).toBe(0);
  });

  it('keeps the radio available when an authored reference is being preserved', () => {
    // Nothing in the layer resolves the reference, but the spawn IS in relative
    // mode — disabling the radio would strand the pane on a checked, disabled
    // control.
    const spawn = {
      template_path: 'moon.toml',
      id: 'moon',
      transform: { relative_to: 'vanished-parent', offset: [1, 0, 2] },
    };
    const layer = layerOf(spawn);
    makePanel(layer).panel.render(spawn, layer);

    expect(relativeRadio().disabled).toBe(false);
    expect(relativeRadio().checked).toBe(true);
    expect(parentSelect().value).toBe('vanished-parent');
  });

  it('enables the radio as soon as one eligible parent exists', () => {
    const parent = { template_path: 'planet.toml', id: 'planet', transform: { position: [0, 0, 0] } };
    const spawn = loneSpawn();
    const layer = layerOf(parent, spawn);
    makePanel(layer).panel.render(spawn, layer);

    expect(relativeRadio().disabled).toBe(false);
    expect(parentSelect().value).toBe('planet');
  });
});
