import { applySlotOperation, inspectSlots, slotWorlds, slotHullCatalogue } from '../editor/workshop-slot-authoring.js';
import { has, t } from './strings.js';

/** Exact-source mission ship slots, checked by the same runtime gate as Save. */
export function mountWorkshopSlotAuthoring({ root, provider, runtime, draft: getDraft, busy, setBusy,
  changed = () => {}, attach = true }) {
  const doc = root.ownerDocument;
  const el = (tag, id, textId) => { const node = doc.createElement(tag); if (id) node.id = id;
    if (textId) node.textContent = t(textId); return node; };
  const section = el('section', 'workshop-slot-authoring');
  const world = el('select', 'workshop-slot-world');
  const slots = el('select', 'workshop-slot-list');
  const id = el('input', 'workshop-slot-id');
  const name = el('input', 'workshop-slot-name');
  const ships = el('select', 'workshop-slot-ships'); ships.multiple = true; ships.size = 5;
  const defaultShip = el('select', 'workshop-slot-default');
  const unclaimed = el('select', 'workshop-slot-unclaimed');
  const status = el('p', 'workshop-slot-status'); status.setAttribute('role', 'status'); status.tabIndex = -1;
  const label = (control, textId) => { const node = el('label', null, textId); node.htmlFor = control.id; return node; };
  const option = (value, title = value) => { const node = el('option'); node.value = value; node.textContent = title; return node; };
  unclaimed.append(option('backfill', t('workshop.slot.backfill')), option('absent', t('workshop.slot.absent')));
  const button = (elementId, textId, type) => { const node = el('button', elementId, textId); node.type = 'button';
    node.addEventListener('click', () => void apply(type)); return node; };
  const add = button('workshop-slot-add', 'workshop.slot.add', 'add');
  const update = button('workshop-slot-update', 'workshop.slot.update', 'update');
  const remove = button('workshop-slot-remove', 'workshop.slot.remove', 'remove');
  section.append(el('h2', null, 'workshop.slot.title'), el('p', null, 'workshop.slot.scope'),
    label(world, 'workshop.slot.world'), world, label(slots, 'workshop.slot.slot'), slots,
    label(id, 'workshop.slot.id'), id, label(name, 'workshop.slot.label'), name,
    label(ships, 'workshop.slot.ships'), ships, label(defaultShip, 'workshop.slot.default'), defaultShip,
    label(unclaimed, 'workshop.slot.unclaimed'), unclaimed, add, update, remove, status);
  if (attach) root.append(section);

  let disposed = false, generation = 0, catalogueGeneration = 0, hulls = [], cataloguePending = false;
  let catalogueDraft = null, catalogueRevision = -1;
  const hullLabel = path => {
    const origin = hulls.find(hull => hull.path === path)?.origin;
    return origin ? `${path} (${origin.kind === 'draft' ? t('workshop.entity.origin_draft')
      : t('workshop.entity.origin_dependency', { origin: origin.id || 'base' })})` : path;
  };
  const selectedRow = () => { try { return inspectSlots(getDraft(), world.value).find(row => row.id === slots.value); } catch { return null; } };
  function show(textId, refused = false, detail = '') {
    status.textContent = [t(textId), detail].filter(Boolean).join(' ');
    status.setAttribute('role', refused ? 'alert' : 'status');
    if (refused) status.focus();
  }
  function controls() {
    const held = busy() || cataloguePending || !getDraft() || !world.value;
    for (const control of section.querySelectorAll('input,select,button')) control.disabled = held;
    update.disabled = remove.disabled = held || !selectedRow();
  }
  function paintRow() {
    const row = selectedRow();
    id.value = row?.id || ''; name.value = row?.label || '';
    const shipPaths = hulls.map(hull => hull.path);
    ships.replaceChildren(...shipPaths.map(path => option(path, hullLabel(path))));
    for (const choice of ships.options) choice.selected = row?.ships.includes(choice.value) || false;
    defaultShip.replaceChildren(...(row?.ships || shipPaths).map(path => option(path, hullLabel(path))));
    defaultShip.value = row?.default_ship || defaultShip.options[0]?.value || '';
    unclaimed.value = row?.unclaimed || 'backfill';
    controls();
  }
  function paintSlots() {
    const previous = slots.value;
    let rows = []; try { rows = inspectSlots(getDraft(), world.value); } catch { /* validation shows invalid TOML */ }
    slots.replaceChildren(option('', t('workshop.slot.new')), ...rows.map(row => option(row.id, row.label || row.id)));
    slots.value = rows.some(row => row.id === previous) ? previous : '';
    paintRow();
  }
  function refresh() {
    const previous = world.value;
    world.replaceChildren(...slotWorlds(getDraft()).map(path => option(path)));
    world.value = slotWorlds(getDraft()).includes(previous) ? previous : (world.options[0]?.value || '');
    paintSlots();
    const draft = getDraft(), revision = draft?.sourceRevision;
    if (draft === catalogueDraft && revision === catalogueRevision) return;
    catalogueDraft = draft; catalogueRevision = revision;
    const token = ++catalogueGeneration;
    hulls = []; cataloguePending = Boolean(draft); paintRow();
    if (!draft) return;
    void slotHullCatalogue({ draft, provider, runtime }).then(value => {
      if (disposed || token !== catalogueGeneration || getDraft() !== draft || draft.sourceRevision !== revision) return;
      hulls = value; cataloguePending = false; paintRow();
    }).catch(error => {
      if (disposed || token !== catalogueGeneration || getDraft() !== draft || draft.sourceRevision !== revision) return;
      catalogueDraft = null; catalogueRevision = -1;
      cataloguePending = false; controls();
      show('workshop.slot.refused', true, String(error?.message || error));
    });
  }
  function rowValue() {
    const selected = [...ships.selectedOptions].map(node => node.value), prior = selectedRow()?.ships || [];
    return { id: id.value.trim(), label: name.value.trim(),
      ships: [...prior.filter(path => selected.includes(path)), ...selected.filter(path => !prior.includes(path))],
      default_ship: defaultShip.value, unclaimed: unclaimed.value };
  }
  async function apply(type) {
    if (busy() || cataloguePending || !getDraft() || !world.value) return;
    const draft = getDraft(), path = world.value, token = ++generation;
    const target = slots.value, operation = { type, target, row: rowValue() };
    if (type === 'remove' && !target) return;
    setBusy(true); show('workshop.slot.checking');
    try {
      await applySlotOperation({ draft, provider, runtime, path, operation,
        current: () => !disposed && generation === token && getDraft() === draft });
      if (!disposed && generation === token) { changed(path); refresh(); slots.value = type === 'remove' ? '' : operation.row.id;
        paintRow(); show('workshop.slot.applied'); }
    } catch (error) {
      if (!disposed && generation === token) show('workshop.slot.refused', true,
        error?.report?.findings?.map(row => `${row.file}${row.line ? `:${row.line}` : ''}: ${row.message}`).join(' ')
        || (has(String(error?.message)) ? t(String(error.message)) : String(error?.message || error)));
    } finally { if (!disposed && generation === token) { setBusy(false); controls(); } }
  }
  world.addEventListener('change', paintSlots);
  slots.addEventListener('change', paintRow);
  ships.addEventListener('change', () => { const selected = [...ships.selectedOptions].map(node => node.value);
    const previous = defaultShip.value;
    defaultShip.replaceChildren(...selected.map(path => option(path, hullLabel(path))));
    defaultShip.value = selected.includes(previous) ? previous : (selected[0] || ''); });
  refresh();
  return { node: section, refresh, dispose() { disposed = true; generation += 1; catalogueGeneration += 1; section.remove(); } };
}
