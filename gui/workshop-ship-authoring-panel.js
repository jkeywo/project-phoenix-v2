import { entityInventory } from '../editor/workshop-entity-composition.js';
import { applyShipOperation, inspectShipAuthoring } from '../editor/workshop-ship-authoring.js';
import { t } from './strings.js';

/** Keyboard-native exact-source Stations, Systems and rating automation form. */
export function mountWorkshopShipAuthoring({ root, provider, runtime, draft: getDraft, busy, setBusy,
  changed = () => {}, attach = true }) {
  const doc = root.ownerDocument;
  const el = (tag, textId, attrs = {}) => { const node = doc.createElement(tag); if (textId) node.textContent = t(textId);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value); return node; };
  const section = el('section', null, { id: 'workshop-ship-authoring', class: 'workshop-ship-authoring' });
  const template = el('select', null, { id: 'workshop-ship-template' });
  const stations = el('select', null, { id: 'workshop-ship-stations', size: '5' });
  const stationId = el('input', null, { id: 'workshop-ship-station-id' });
  const stationName = el('input', null, { id: 'workshop-ship-station-name' });
  const stationDescription = el('input', null, { id: 'workshop-ship-station-description' });
  const stationRank = el('input', null, { id: 'workshop-ship-station-rank' });
  const stationConsole = el('input', null, { id: 'workshop-ship-station-console' });
  const systems = el('select', null, { id: 'workshop-ship-systems', size: '6' });
  const systemId = el('input', null, { id: 'workshop-ship-system-id' });
  const systemKind = el('select', null, { id: 'workshop-ship-system-kind' });
  const systemStation = el('select', null, { id: 'workshop-ship-system-station' });
  const ratings = el('select', null, { id: 'workshop-ship-ratings', size: '4' });
  const ratingName = el('input', null, { id: 'workshop-ship-rating-name' });
  const automated = el('select', null, { id: 'workshop-ship-rating-systems', size: '6', multiple: '' });
  const doctrines = el('select', null, { id: 'workshop-ship-doctrines', size: '5' });
  const doctrineId = el('input', null, { id: 'workshop-ship-doctrine-id' });
  const doctrineKind = el('select', null, { id: 'workshop-ship-doctrine-kind' });
  const doctrinePriority = el('input', null, { id: 'workshop-ship-doctrine-priority', type: 'number', step: 'any' });
  const doctrineReference = el('input', null, { id: 'workshop-ship-doctrine-reference' });
  const doctrineRoute = el('input', null, { id: 'workshop-ship-doctrine-route' });
  const doctrineLoop = el('input', null, { id: 'workshop-ship-doctrine-loop', type: 'checkbox' });
  const status = el('p', null, { id: 'workshop-ship-status', role: 'status', tabindex: '-1' });
  const label = (node, id) => el('label', id, { for: node.id });
  const button = (id, textId, operation) => { const node = el('button', textId, { id, type: 'button' });
    node.addEventListener('click', () => void apply(operation)); return node; };
  const stationAdd = button('workshop-ship-station-add', 'workshop.ship.add_station', () => ({ type: 'station-add', id: stationId.value,
    name: stationName.value, description: stationDescription.value, rank: stationRank.value, console: stationConsole.value }));
  const stationApply = button('workshop-ship-station-apply', 'workshop.ship.apply_station', () => ({ type: 'station-update', id: stations.value,
    fields: { name: stationName.value, description: stationDescription.value, rank: stationRank.value, console: stationConsole.value || undefined } }));
  const stationUp = button('workshop-ship-station-up', 'workshop.ship.move_up', () => ({ type: 'station-move', id: stations.value, direction: -1 }));
  const stationDown = button('workshop-ship-station-down', 'workshop.ship.move_down', () => ({ type: 'station-move', id: stations.value, direction: 1 }));
  const stationRemove = button('workshop-ship-station-remove', 'workshop.ship.remove_station', () => ({ type: 'station-remove', id: stations.value }));
  const systemAdd = button('workshop-ship-system-add', 'workshop.ship.add_system', () => ({ type: 'system-add', id: systemId.value,
    kind: systemKind.value, station: systemStation.value }));
  const systemApply = button('workshop-ship-system-apply', 'workshop.ship.apply_system', () => ({ type: 'system-update', id: systems.value,
    fields: { kind: systemKind.value, station: systemStation.value || undefined, ai_only: !systemStation.value } }));
  const systemUp = button('workshop-ship-system-up', 'workshop.ship.move_up', () => ({ type: 'system-move', id: systems.value, direction: -1 }));
  const systemDown = button('workshop-ship-system-down', 'workshop.ship.move_down', () => ({ type: 'system-move', id: systems.value, direction: 1 }));
  const systemRemove = button('workshop-ship-system-remove', 'workshop.ship.remove_system', () => ({ type: 'system-remove', id: systems.value }));
  const ratingAdd = button('workshop-ship-rating-add', 'workshop.ship.add_rating', () => ({ type: 'rating-add', station: stations.value, name: ratingName.value }));
  const ratingApply = button('workshop-ship-rating-apply', 'workshop.ship.apply_rating', () => ({ type: 'rating-update', station: stations.value,
    rating: ratings.value, name: ratingName.value, systems: [...automated.selectedOptions].map(option => option.value) }));
  const ratingUp = button('workshop-ship-rating-up', 'workshop.ship.move_up', () => ({ type: 'rating-move', station: stations.value, rating: ratings.value, direction: -1 }));
  const ratingDown = button('workshop-ship-rating-down', 'workshop.ship.move_down', () => ({ type: 'rating-move', station: stations.value, rating: ratings.value, direction: 1 }));
  const ratingRemove = button('workshop-ship-rating-remove', 'workshop.ship.remove_rating', () => ({ type: 'rating-remove', station: stations.value, rating: ratings.value }));
  const doctrineOperation = type => ({ type, id: type === 'doctrine-add' ? doctrineId.value : doctrines.value,
    kind: doctrineKind.value, base_priority: Number(doctrinePriority.value), references: doctrineReferences() });
  const doctrineAdd = button('workshop-ship-doctrine-add', 'workshop.ship.add_doctrine', () => doctrineOperation('doctrine-add'));
  const doctrineApply = button('workshop-ship-doctrine-apply', 'workshop.ship.apply_doctrine', () => doctrineOperation('doctrine-update'));
  const doctrineUp = button('workshop-ship-doctrine-up', 'workshop.ship.move_up', () => ({ type: 'doctrine-move', id: doctrines.value, direction: -1 }));
  const doctrineDown = button('workshop-ship-doctrine-down', 'workshop.ship.move_down', () => ({ type: 'doctrine-move', id: doctrines.value, direction: 1 }));
  const doctrineRemove = button('workshop-ship-doctrine-remove', 'workshop.ship.remove_doctrine', () => ({ type: 'doctrine-remove', id: doctrines.value }));
  section.append(el('h2', 'workshop.ship.title'), el('p', 'workshop.ship.scope'), label(template, 'workshop.ship.template'), template,
    el('h3', 'workshop.ship.stations'), label(stations, 'workshop.ship.stations'), stations, label(stationId, 'workshop.ship.id'), stationId,
    label(stationName, 'workshop.ship.name'), stationName, label(stationDescription, 'workshop.ship.description'), stationDescription,
    label(stationRank, 'workshop.ship.rank'), stationRank, label(stationConsole, 'workshop.ship.console'), stationConsole,
    stationAdd, stationApply, stationUp, stationDown, stationRemove,
    el('h3', 'workshop.ship.systems'), label(systems, 'workshop.ship.systems'), systems, label(systemId, 'workshop.ship.id'), systemId,
    label(systemKind, 'workshop.ship.kind'), systemKind, label(systemStation, 'workshop.ship.owner'), systemStation,
    systemAdd, systemApply, systemUp, systemDown, systemRemove,
    el('h3', 'workshop.ship.ratings'), label(ratings, 'workshop.ship.ratings'), ratings, label(ratingName, 'workshop.ship.rating_name'), ratingName,
    label(automated, 'workshop.ship.automated_systems'), automated, ratingAdd, ratingApply, ratingUp, ratingDown, ratingRemove,
    el('h3', 'workshop.ship.doctrines'), label(doctrines, 'workshop.ship.doctrines'), doctrines,
    label(doctrineId, 'workshop.ship.id'), doctrineId, label(doctrineKind, 'workshop.ship.directive_kind'), doctrineKind,
    label(doctrinePriority, 'workshop.ship.priority'), doctrinePriority,
    label(doctrineReference, 'workshop.ship.directive_reference'), doctrineReference,
    label(doctrineLoop, 'workshop.ship.directive_loop'), doctrineLoop,
    label(doctrineRoute, 'workshop.ship.directive_route'), doctrineRoute,
    doctrineAdd, doctrineApply, doctrineUp, doctrineDown, doctrineRemove, status);
  if (attach) root.append(section);

  let dependencies = null, schema = null, inspection = null, loading = null, generation = 0, disposed = false;
  const option = (value, text = value) => { const node = el('option', null, { value }); node.textContent = text; return node; };
  const show = (id, refused = false, detail = '') => { status.textContent = `${t(id)}${detail ? ` ${detail}` : ''}`; status.setAttribute('role', refused ? 'alert' : 'status'); if (refused) status.focus(); };
  const selectedStation = () => inspection?.stations.find(row => row.id === stations.value);
  const selectedSystem = () => inspection?.systems.find(row => row.id === systems.value);
  const selectedRating = () => selectedStation()?.rating?.find(row => row.name === ratings.value);
  const selectedDoctrine = () => inspection?.doctrines.find(row => row.id === doctrines.value);
  function doctrineReferences() {
    const value = doctrineReference.value.trim(), route = doctrineRoute.value.trim();
    if (doctrineKind.value === 'Patrol') return { ...(value ? { directive_anchors: value.split(',').map(item => item.trim()).filter(Boolean) } : {}),
      ...(doctrineLoop.checked ? { directive_loop: true } : {}) };
    if (['Reach', 'Retreat'].includes(doctrineKind.value)) return value ? { directive_anchor: value } : {};
    if (doctrineKind.value === 'Destroy') return value ? { directive_target: value } : {};
    if (doctrineKind.value === 'Hail') return value ? { directive_hail_target: value } : {};
    if (doctrineKind.value === 'Scan') return value ? { directive_scan_target: value } : {};
    if (doctrineKind.value === 'Dock') return value ? { directive_dock_target: value } : {};
    if (['Tow', 'Stabilise', 'Escort', 'Transfer', 'FieldRepair', 'Secure', 'Rescue'].includes(doctrineKind.value)) return value ? { directive_operate_target: value } : {};
    if (doctrineKind.value === 'Order') return { ...(value ? { directive_order_target: value } : {}), ...(route ? { directive_order_route: route } : {}) };
    return {};
  }
  function doctrineReferenceOf(row) {
    if (!row) return '';
    return (row.directive_anchors || []).join(', ') || row.directive_anchor || row.directive_target || row.directive_hail_target
      || row.directive_scan_target || row.directive_dock_target || row.directive_operate_target || row.directive_order_target || '';
  }

  function paint() {
    const draft = getDraft(), inventory = draft ? entityInventory(draft, dependencies || undefined).filter(row => row.editable) : [];
    const selectedPath = template.value; template.replaceChildren(...inventory.map(row => option(row.path)));
    template.value = inventory.some(row => row.path === selectedPath) ? selectedPath : (inventory[0]?.path || '');
    const stationValue = stations.value; stations.replaceChildren(...(inspection?.stations || []).map(row => option(row.id, `${row.id} — ${row.local ? t('workshop.ship.local') : t('workshop.ship.included', { origin: row.owner })}`)));
    stations.value = inspection?.stations.some(row => row.id === stationValue) ? stationValue : (inspection?.stations[0]?.id || '');
    const systemValue = systems.value; systems.replaceChildren(...(inspection?.systems || []).map(row => option(row.id, `${row.id} (${row.kind}) — ${row.local ? t('workshop.ship.local') : t('workshop.ship.included', { origin: row.owner })}`)));
    systems.value = inspection?.systems.some(row => row.id === systemValue) ? systemValue : (inspection?.systems[0]?.id || '');
    const station = selectedStation(), system = selectedSystem();
    stationId.value = station?.id || ''; stationName.value = station?.name || ''; stationDescription.value = station?.description || '';
    stationRank.value = station?.rank || ''; stationConsole.value = station?.console || '';
    systemId.value = system?.id || ''; systemKind.replaceChildren(...(schema?.system_kinds || []).map(kind => option(kind)));
    if (system?.kind) systemKind.value = system.kind;
    systemStation.replaceChildren(option('', t('workshop.ship.ai_only')), ...(inspection?.stations || []).map(row => option(row.id)));
    systemStation.value = system?.station || '';
    const ratingValue = ratings.value; ratings.replaceChildren(...(station?.rating || []).map(row => option(row.name)));
    ratings.value = (station?.rating || []).some(row => row.name === ratingValue) ? ratingValue : (station?.rating?.[0]?.name || '');
    const rating = selectedRating(); ratingName.value = rating?.name || '';
    automated.replaceChildren(...(inspection?.systems || []).filter(row => row.station === station?.id).map(row => option(row.id)));
    for (const choice of automated.options) choice.selected = (rating?.automated_systems || []).includes(choice.value);
    const doctrineValue = doctrines.value;
    doctrines.replaceChildren(...(inspection?.doctrines || []).map(row => option(row.id,
      `${row.id} (${row.directive_kind || 'None'}) — ${row.local ? t('workshop.ship.local') : t('workshop.ship.included', { origin: row.owner })}`)));
    doctrines.value = inspection?.doctrines.some(row => row.id === doctrineValue) ? doctrineValue : (inspection?.doctrines[0]?.id || '');
    const doctrine = selectedDoctrine(); doctrineId.value = doctrine?.id || '';
    doctrineKind.replaceChildren(...(schema?.directive_kinds || []).map(kind => option(kind)));
    doctrineKind.value = doctrine?.directive_kind || 'None'; doctrinePriority.value = String(doctrine?.base_priority ?? 0);
    doctrineReference.value = doctrineReferenceOf(doctrine); doctrineRoute.value = doctrine?.directive_order_route || '';
    doctrineLoop.checked = doctrine?.directive_loop === true;
  }
  function controls() {
    const held = busy() || !!loading || !schema, station = selectedStation(), system = selectedSystem(), rating = selectedRating(), doctrine = selectedDoctrine();
    for (const node of section.querySelectorAll('input,select,button')) node.disabled = held;
    stationApply.disabled = stationUp.disabled = stationDown.disabled = stationRemove.disabled = held || !station?.local;
    systemApply.disabled = systemUp.disabled = systemDown.disabled = systemRemove.disabled = held || !system?.local;
    ratingAdd.disabled = held || !station?.local;
    ratingApply.disabled = ratingUp.disabled = ratingDown.disabled = ratingRemove.disabled = held || !station?.local || !rating;
    doctrineApply.disabled = doctrineUp.disabled = doctrineDown.disabled = doctrineRemove.disabled = held || !doctrine?.local;
  }
  function inspect() {
    const draft = getDraft(); inspection = null;
    if (draft && template.value && dependencies) try { inspection = inspectShipAuthoring(draft, dependencies, template.value); }
    catch (error) { if (String(error?.message) !== 'entity-is-not-a-ship') show('workshop.ship.refused', true, String(error?.message || error)); }
    paint(); controls();
  }
  async function load() {
    if (typeof runtime.dependencies !== 'function' || typeof runtime.shipSchema !== 'function'
        || loading || (dependencies && schema)) return;
    const token = ++generation;
    loading = Promise.all([runtime.dependencies(), runtime.shipSchema()]); controls();
    try { [dependencies, schema] = await loading; } catch (error) { if (!disposed && token === generation) show('workshop.ship.refused', true, String(error?.message || error)); }
    finally { if (!disposed && token === generation) { loading = null; inspect(); } }
  }
  async function apply(factory) {
    if (busy() || loading || !inspection) return; const draft = getDraft(), token = ++generation, operation = { ...factory(), path: template.value };
    setBusy(true); controls(); show('workshop.ship.checking');
    try { const result = await applyShipOperation({ draft, provider, runtime, dependencies, operation,
      current: () => !disposed && token === generation && getDraft() === draft });
      if (!disposed && token === generation) { inspection = result.inspection; changed(operation.path); show('workshop.ship.applied'); paint(); } }
    catch (error) { if (!disposed && token === generation) show('workshop.ship.refused', true,
      error?.report?.findings?.map(row => `${row.file}${row.line ? `:${row.line}` : ''}: ${row.message}`).join(' ') || String(error?.message || error)); }
    finally { if (!disposed && token === generation) { setBusy(false); controls(); } }
  }
  template.addEventListener('change', inspect); stations.addEventListener('change', () => { paint(); controls(); });
  systems.addEventListener('change', () => { paint(); controls(); }); ratings.addEventListener('change', () => { paint(); controls(); });
  doctrines.addEventListener('change', () => { paint(); controls(); }); doctrineKind.addEventListener('change', controls);
  function refresh() { paint(); inspect(); void load(); }
  refresh();
  return { node: section, refresh, dispose() { disposed = true; generation += 1; section.remove(); } };
}
