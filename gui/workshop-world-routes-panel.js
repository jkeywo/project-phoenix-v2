import { inspectWorldRoutes, applyWorldRouteOperation } from '../editor/workshop-world-routes.js';
import { t } from './strings.js';

/** Basic route composition inside the existing spatial editor. No new history,
 * file store or runtime travel controls. Dependency targets remain read-only. */
export function mountWorkshopWorldRoutes({ root, runtime, provider, draft: getDraft, dependencies,
  busy, setBusy, changed = () => {} }) {
  const doc = root.ownerDocument;
  const el = (tag, id, textId) => {
    const node = doc.createElement(tag); if (id) node.id = `workshop-world-routes-${id}`;
    if (textId) node.textContent = t(textId); return node;
  };
  const node = el('fieldset', null); node.append(el('legend', null, 'workshop.world_routes.title'));
  const label = (control, id) => { const value = el('label', null, id); value.htmlFor = control.id; return value; };
  const select = id => el('select', id);
  const input = id => { const value = el('input', id); value.type = 'text'; value.maxLength = 512; return value; };
  const world = select('world'), route = select('route'), id = input('id'), text = input('label');
  const target = select('target'), anchor = select('anchor'), status = el('p', 'status'); status.setAttribute('role', 'status');
  const button = (id, textId) => { const value = el('button', id, textId); value.type = 'button'; return value; };
  const add = button('add', 'workshop.world_routes.add'), save = button('save', 'workshop.world_routes.save');
  const remove = button('remove', 'workshop.world_routes.remove');
  for (const [control, textId] of [[world, 'workshop.world_routes.world'], [route, 'workshop.world_routes.route'],
    [id, 'workshop.world_routes.id'], [text, 'workshop.world_routes.label'], [target, 'workshop.world_routes.target'], [anchor, 'workshop.world_routes.anchor']])
    node.append(label(control, textId), control);
  node.append(add, save, remove, status); root.append(node);
  const emptyDependencies = {};
  let inspection = null, observed = null, observedDraft = null, observedDependencies = null, disposed = false, generation = 0;
  const options = (control, rows, value, title) => {
    const selected = control.value;
    control.replaceChildren(...rows.map(row => { const node = el('option'); node.value = value(row); node.textContent = title(row); return node; }));
    if (rows.some(row => value(row) === selected)) control.value = selected;
  };
  function controls() {
    const held = disposed || busy() || !inspection?.editable || typeof runtime.compose !== 'function';
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = held;
    world.disabled = disposed || busy() || !inspection?.roots.length;
    const selected = inspection?.routes.some(row => row.id === route.value);
    add.disabled = held || !id.value.trim() || !text.value.trim() || !anchor.value;
    save.disabled = add.disabled || !selected; remove.disabled = held || !selected;
  }
  function anchors() {
    options(anchor, inspection?.worlds.find(row => row.path === target.value)?.anchors || [], row => row.name, row => row.name);
    controls();
  }
  function choose() {
    const selected = inspection?.routes.find(row => row.id === route.value);
    id.value = selected?.id || ''; text.value = selected?.label || '';
    target.value = selected?.target?.kind === 'layer' ? selected.target.path : world.value;
    anchors(); if (selected?.target?.anchor) anchor.value = selected.target.anchor; controls();
  }
  function refresh(force = false) {
    const draft = getDraft(), effective = dependencies?.() || emptyDependencies;
    const key = JSON.stringify([draft?.sourceRevision, world.value]);
    if (!force && key === observed && draft === observedDraft && effective === observedDependencies) { controls(); return; }
    try {
      inspection = draft ? inspectWorldRoutes(draft, effective, world.value) : null;
      options(world, inspection?.roots || [], row => row.path, row => row.path);
      inspection = draft ? inspectWorldRoutes(draft, effective, world.value) : null;
      options(route, inspection?.routes || [], row => row.id, row => row.id);
      options(target, inspection?.worlds || [], row => row.path, row => row.path);
      observed = JSON.stringify([draft?.sourceRevision, world.value]); observedDraft = draft; observedDependencies = effective;
      choose();
    } catch (error) { inspection = null; status.textContent = t('workshop.world_routes.refused', { reason: error.message }); }
    controls();
  }
  async function apply(type) {
    if (disposed || busy() || !inspection?.editable) return;
    const draft = getDraft(), token = ++generation, path = world.value;
    const operation = { type, path, original: route.value, id: id.value, label: text.value,
      target: target.value === path ? { kind: 'base', anchor: anchor.value } : { kind: 'layer', path: target.value, anchor: anchor.value } };
    const release = setBusy(true); if (release === false) return; controls();
    try {
      await applyWorldRouteOperation({ draft, runtime, provider, dependencies: dependencies?.() || {}, operation,
        current: () => !disposed && token === generation && getDraft() === draft });
      if (!disposed && token === generation) { changed(path); refresh(true); status.setAttribute('role', 'status'); status.textContent = t('workshop.changed'); }
    } catch (error) {
      if (!disposed && token === generation) {
        status.textContent = t('workshop.world_routes.refused', { reason: error.report?.findings?.[0]?.message || error.message });
        status.setAttribute('role', 'alert');
      }
    } finally {
      if (typeof release === 'function') release(); else setBusy(false);
      if (!disposed && token === generation) controls();
    }
  }
  world.addEventListener('change', () => refresh(true)); route.addEventListener('change', choose);
  target.addEventListener('change', anchors); anchor.addEventListener('change', controls);
  id.addEventListener('input', controls); text.addEventListener('input', controls);
  for (const [button, type] of [[add, 'route-add'], [save, 'route-update'], [remove, 'route-remove']])
    button.addEventListener('click', () => { if (!button.disabled) void apply(type); });
  refresh();
  return { node, refresh, controls, dispose() { disposed = true; generation++; node.remove(); } };
}
