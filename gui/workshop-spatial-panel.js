import { applySpatialOperation, spatialInventory } from '../editor/workshop-spatial.js';
import { t } from './strings.js';

/** Keyboard-operable visual placement and its equivalent structured form. */
export function mountWorkshopSpatial({ root, provider, runtime, draft: getDraft, busy, setBusy,
  changed = () => {}, attach = true }) {
  const doc = root.ownerDocument;
  const el = (tag, textId, attrs = {}) => {
    const node = doc.createElement(tag);
    if (textId) node.textContent = t(textId);
    Object.entries(attrs).forEach(([key, value]) => node.setAttribute(key, value));
    return node;
  };
  const section = el('section', null, { class: 'workshop-spatial', id: 'workshop-spatial' });
  section.append(el('h2', 'workshop.spatial.title'), el('p', 'workshop.spatial.scope'));
  const world = el('select', null, { id: 'workshop-spatial-world' });
  const layerPath = el('input', null, { id: 'workshop-spatial-layer-path', type: 'text', placeholder: 'assets/worlds/layer.toml' });
  const layers = el('select', null, { id: 'workshop-spatial-layers' });
  const anchors = el('select', null, { id: 'workshop-spatial-anchors', size: '5' });
  const entities = el('select', null, { id: 'workshop-spatial-entities', size: '7' });
  const x = el('input', null, { id: 'workshop-spatial-x', type: 'number', step: 'any' });
  const y = el('input', null, { id: 'workshop-spatial-y', type: 'number', step: 'any' });
  const z = el('input', null, { id: 'workshop-spatial-z', type: 'number', step: 'any' });
  const yaw = el('input', null, { id: 'workshop-spatial-yaw', type: 'number', step: 'any' });
  const name = el('input', null, { id: 'workshop-spatial-name', type: 'text' });
  const template = el('input', null, { id: 'workshop-spatial-template', type: 'text', placeholder: 'assets/entities/…toml' });
  const map = el('div', null, { class: 'workshop-spatial-map', id: 'workshop-spatial-map', role: 'group',
    'aria-label': t('workshop.spatial.canvas'), tabindex: '0' });
  const status = el('p', null, { id: 'workshop-spatial-status', role: 'status', tabindex: '-1' });
  const label = (target, id) => el('label', id, { for: target.id });
  const button = (id, textId, operation) => {
    const node = el('button', textId, { id, type: 'button' });
    node.addEventListener('click', () => void apply(operation()));
    return node;
  };
  const addLayer = button('workshop-spatial-layer-add', 'workshop.spatial.layer_add', () => ({
    type: 'layer-add', path: world.value, layer: layerPath.value.trim(),
  }));
  const removeLayer = button('workshop-spatial-layer-remove', 'workshop.spatial.layer_remove', () => ({
    type: 'layer-remove', path: world.value, layer: layers.value,
  }));
  const addAnchor = button('workshop-spatial-anchor-add', 'workshop.spatial.anchor_add', () => ({
    type: 'anchor-add', path: world.value, name: name.value.trim(), position: coordinates(),
  }));
  const moveAnchor = button('workshop-spatial-anchor-move', 'workshop.spatial.move', () => ({
    type: 'anchor-move', path: world.value, name: anchors.value, position: coordinates(),
  }));
  const removeAnchor = button('workshop-spatial-anchor-remove', 'workshop.spatial.remove', () => ({
    type: 'anchor-remove', path: world.value, name: anchors.value,
  }));
  const addEntity = button('workshop-spatial-entity-add', 'workshop.spatial.entity_add', () => ({
    type: 'entity-add', path: world.value, id: name.value.trim() || undefined,
    template_path: template.value.trim(), transform: { position: coordinates(), rotation: [0, number(yaw), 0] },
  }));
  const moveEntity = button('workshop-spatial-entity-move', 'workshop.spatial.move', () => ({
    type: 'entity-move', path: world.value, index: Number(entities.value),
    transform: { position: coordinates(), rotation: [0, number(yaw), 0] },
  }));
  const removeEntity = button('workshop-spatial-entity-remove', 'workshop.spatial.remove', () => ({
    type: 'entity-remove', path: world.value, index: Number(entities.value),
  }));
  section.append(label(world, 'workshop.spatial.world'), world,
    label(layers, 'workshop.spatial.layers'), layers, label(layerPath, 'workshop.spatial.layer_path'), layerPath,
    addLayer, removeLayer, map, label(anchors, 'workshop.spatial.anchors'), anchors,
    label(entities, 'workshop.spatial.entities'), entities,
    label(name, 'workshop.spatial.name'), name, label(template, 'workshop.spatial.template'), template,
    label(x, 'workshop.spatial.x'), x, label(y, 'workshop.spatial.y'), y, label(z, 'workshop.spatial.z'), z,
    label(yaw, 'workshop.spatial.yaw'), yaw,
    addAnchor, moveAnchor, removeAnchor, addEntity, moveEntity, removeEntity, status);
  if (attach) root.append(section);

  let inventory = [], dependencies = null, loading = null, dependencyAttempted = false,
    disposed = false, generation = 0;
  const number = input => { const value = Number(input.value); if (!Number.isFinite(value)) throw new Error('invalid-coordinate'); return value; };
  const coordinates = () => [number(x), number(y), number(z)];
  const option = (value, text) => { const node = el('option', null, { value }); node.textContent = text; return node; };
  const selectedWorld = () => inventory.find(row => row.path === world.value);
  const selectedEntity = () => selectedWorld()?.entities.find(row => row.index === Number(entities.value));
  const selectedAnchor = () => selectedWorld()?.anchors.find(row => row.name === anchors.value);
  const positionOf = row => row?.position || row?.transform?.position || [0, 0, 0];
  const show = (id, refused = false, detail = '') => {
    status.textContent = `${t(id)}${detail ? ` ${detail}` : ''}`;
    status.setAttribute('role', refused ? 'alert' : 'status');
    if (refused) status.focus();
  };

  function selectRow(row, kind) {
    if (kind === 'anchor') { anchors.value = row.name; entities.selectedIndex = -1; }
    else { entities.value = String(row.index); anchors.selectedIndex = -1; }
    const [px, py, pz] = positionOf(row); x.value = px; y.value = py; z.value = pz;
    yaw.value = row.transform?.rotation?.[1] || 0;
    name.value = kind === 'anchor' ? row.name : row.id || row.name || '';
    template.value = row.template_path || '';
    controls(); paintMap();
  }

  function paintMap() {
    const focusedKey = map.contains(doc.activeElement) ? doc.activeElement?.dataset?.key : null;
    const rows = [...(selectedWorld()?.anchors || []).map(row => ({ ...row, kind: 'anchor' })),
      ...(selectedWorld()?.entities || []).filter(row => row.transform.position)
        .map(row => ({ ...row, kind: row.region ? 'region' : 'entity' }))];
    const extent = Math.max(1, ...rows.flatMap(row => { const p = positionOf(row); return [Math.abs(p[0]), Math.abs(p[2])]; }));
    map.replaceChildren(...rows.map(row => {
      const p = positionOf(row), selected = row.kind === 'anchor' ? anchors.value === row.name : entities.value === String(row.index);
      const marker = el('button', null, { type: 'button', class: 'workshop-spatial-marker',
        'data-kind': row.kind, 'aria-pressed': String(selected),
        'data-key': row.kind === 'anchor' ? `anchor:${row.name}` : `entity:${row.index}`,
        'aria-label': `${row.kind}: ${row.name || row.id || row.template_path || row.index}` });
      marker.textContent = row.kind === 'anchor' ? '+' : row.region ? 'R' : 'E';
      marker.style.left = `${50 + (p[0] / extent) * 45}%`; marker.style.top = `${50 - (p[2] / extent) * 45}%`;
      marker.addEventListener('click', () => selectRow(row, row.kind === 'anchor' ? 'anchor' : 'entity'));
      marker.addEventListener('keydown', event => {
        const delta = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[event.key];
        if (!delta) return;
        event.preventDefault(); const position = positionOf(row);
        void apply(row.kind === 'anchor'
          ? { type: 'anchor-move', path: world.value, name: row.name, position: [position[0] + delta[0], position[1], position[2] + delta[1]] }
          : { type: 'entity-move', path: world.value, index: row.index,
            transform: { position: [position[0] + delta[0], position[1], position[2] + delta[1]] } });
      });
      return marker;
    }));
    if (focusedKey) [...map.querySelectorAll('[data-key]')].find(node => node.dataset.key === focusedKey)?.focus();
  }

  function controls() {
    const held = busy(), anchor = selectedAnchor(), entity = selectedEntity();
    [...section.querySelectorAll('input,select,button')].forEach(node => { node.disabled = held; });
    removeLayer.disabled = held || !layers.value; moveAnchor.disabled = removeAnchor.disabled = held || !anchor;
    moveEntity.disabled = removeEntity.disabled = held || !entity;
    addLayer.disabled = held || !layerPath.value.trim(); addAnchor.disabled = held || !name.value.trim();
    addEntity.disabled = held || !template.value.trim();
  }

  function paint() {
    const draft = getDraft();
    try { inventory = draft ? spatialInventory(draft, dependencies || undefined) : []; }
    catch { inventory = []; }
    const oldWorld = world.value; world.replaceChildren(...inventory.map(row => option(row.path, row.path)));
    if (inventory.some(row => row.path === oldWorld)) world.value = oldWorld;
    const row = selectedWorld(), oldLayer = layers.value, oldAnchor = anchors.value, oldEntity = entities.value;
    layers.replaceChildren(...(row?.layers || []).map(path => option(path, path)));
    if (row?.layers.includes(oldLayer)) layers.value = oldLayer;
    anchors.replaceChildren(...(row?.anchors || []).map(item => option(item.name, `${item.name} — ${item.position.join(', ')}`)));
    if (row?.anchors.some(item => item.name === oldAnchor)) anchors.value = oldAnchor; else anchors.selectedIndex = -1;
    entities.replaceChildren(...(row?.entities || []).map(item => option(item.index,
      `${item.region ? t('workshop.spatial.region') : t('workshop.spatial.entity')} — ${item.id || item.name || item.template_path}`)));
    if (row?.entities.some(item => String(item.index) === oldEntity)) entities.value = oldEntity; else entities.selectedIndex = -1;
    paintMap(); controls();
  }

  async function apply(operation) {
    if (busy()) return;
    const draft = getDraft(), token = ++generation;
    if (!draft) return;
    setBusy(true); controls(); show('workshop.spatial.checking');
    try {
      const result = await applySpatialOperation({ draft, provider, runtime, operation, dependencies,
        current: () => !disposed && token === generation && getDraft() === draft });
      if (disposed || token !== generation) return;
      changed(result.changes[0].path); show('workshop.spatial.applied'); paint();
    } catch (error) {
      if (!disposed && token === generation) show('workshop.spatial.refused', true,
        error?.report?.findings?.map(row => `${row.file || operation.path}: ${row.message}`).join(' ') || String(error?.message || error));
    } finally { if (!disposed && token === generation) { setBusy(false); controls(); } }
  }

  async function loadDependencies() {
    if (dependencies || loading || dependencyAttempted || !runtime.dependencies) return;
    const token = generation; dependencyAttempted = true; loading = runtime.dependencies(); controls();
    try {
      const result = await loading;
      if (!disposed && token === generation) dependencies = result;
    } catch (error) {
      if (!disposed && token === generation) show('workshop.spatial.refused', true, String(error?.message || error));
    } finally {
      if (!disposed && token === generation) { loading = null; paint(); }
    }
  }

  world.addEventListener('change', paint); layers.addEventListener('change', controls);
  layerPath.addEventListener('input', controls); name.addEventListener('input', controls); template.addEventListener('input', controls);
  anchors.addEventListener('change', () => { const row = selectedAnchor(); if (row) selectRow(row, 'anchor'); });
  entities.addEventListener('change', () => { const row = selectedEntity(); if (row) selectRow(row, 'entity'); });
  function refresh() { paint(); void loadDependencies(); }
  refresh();
  return { node: section, refresh, dispose() { disposed = true; generation += 1; section.remove(); } };
}
