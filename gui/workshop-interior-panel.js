import { t, wireText } from './strings.js';
import { runWorkshopMutation } from './workshop-edit-session.js';
import { inspectInterior, initialInterior, applyInteriorOperation } from '../editor/workshop-interiors.js';

/** The map edits the same proposed source transaction as the property fields. */
export function mountWorkshopInterior({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument, clone = value => structuredClone(value);
  const el = (tag, id, key, attrs = {}) => { const n = doc.createElement(tag); if (id) n.id = 'workshop-interior-' + id;
    if (key) n.textContent = t('workshop.interior.' + key); for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v); return n; };
  const label = (control, key) => { const n = el('label', null, key, { for: control.id }); n.append(control); return n; };
  const input = (id, type = 'text') => el('input', id, null, { type, ...(type === 'number' ? { step: 'any' } : {}) });
  const option = (select, value, text = value) => { const n = el('option'); n.value = value; n.textContent = text; select.append(n); };
  const button = (id, action) => { const n = el('button', id, id.replaceAll('-', '_'), { type: 'button' }); n.addEventListener('click', action); return n; };
  const node = el('section', 'panel'), template = el('select', 'template'), decks = el('select', 'decks'), rooms = el('select', 'rooms');
  const connections = el('select', 'connections'), system = el('select', 'system'), placement = el('select', 'placement');
  const materialise = input('materialise', 'checkbox'), status = el('p', 'status', null, { role: 'status' });
  const deckLabel = input('deck-label'), weight = input('damage-weight', 'number');
  const roomDeck = el('select', 'room-deck');
  const roomFields = Object.fromEntries(['label', 'x', 'z', 'width', 'depth', 'material'].map(key => [key, input(key, ['label', 'material'].includes(key) ? 'text' : 'number')]));
  const from = el('select', 'from'), to = el('select', 'to'), travel = input('travel-secs', 'number'), boundary = el('select', 'boundary');
  const homes = el('div', 'homes');
  const svg = doc.createElementNS('http://www.w3.org/2000/svg', 'svg'); svg.id = 'workshop-interior-map';
  svg.setAttribute('role', 'group'); svg.setAttribute('aria-label', t('workshop.interior.map')); svg.style.cssText = 'width:100%;height:320px;touch-action:none;border:1px solid currentColor';
  const save = button('save', () => void apply());
  const create = button('create', () => { if (!inspection) return; form = initialInterior(inspection.systems, inspection.teamCount); deckId = form.deck[0].id; roomId = form.compartment[0].id; connectionId = null; paint(); });
  const freshId = (rows, prefix) => { let i = 1; while (rows.some(row => row.id === prefix + i)) i += 1; return prefix + i; };
  const addDeck = button('add-deck', () => { if (!form) return; deckId = freshId(form.deck, 'deck-'); form.deck.push({ id: deckId, label: deckId, damage_weight: 1 }); roomId = null; paint(); });
  const removeDeck = button('remove-deck', () => {
    if (form?.compartment.some(room => room.deck === deckId)) { status.textContent = t('workshop.interior.deck_used'); return; }
    if (form?.deck.length > 1) { form.deck = form.deck.filter(deck => deck.id !== deckId); deckId = form.deck[0].id; paint(); }
  });
  const addRoom = button('add-room', () => { if (!form || !deckId) return; roomId = freshId(form.compartment, 'room-');
    const sameDeck = form.compartment.filter(room => room.deck === deckId), x = sameDeck.length ? Math.max(...sameDeck.map(room => room.x + room.width)) : 0;
    form.compartment.push({ id: roomId, deck: deckId, label: roomId, x, z: 0, width: 10, depth: 10, material: 'bulkhead' }); paint(); });
  const removeRoom = button('remove-room', () => {
    if (!form || !roomId) return;
    if (Object.values(form.placements).includes(roomId) || form.team_homes.includes(roomId) || form.connection.some(c => c.from === roomId || c.to === roomId)) {
      status.textContent = t('workshop.interior.room_used'); return;
    }
    form.compartment = form.compartment.filter(room => room.id !== roomId); roomId = null; paint();
  });
  const addConnection = button('add-connection', () => { if (!form || form.compartment.length < 2) return;
    connectionId = freshId(form.connection, 'connection-'); form.connection.push({ id: connectionId, from: form.compartment[0].id, to: form.compartment[1].id, travel_secs: 5 }); paint(); });
  const removeConnection = button('remove-connection', () => { if (!form) return; form.connection = form.connection.filter(c => c.id !== connectionId); connectionId = null; paint(); });
  const addTeam = button('add-team', () => { if (form) { form.team_homes.push(''); paint(); } });
  const removeTeam = button('remove-team', () => { if (form) { form.team_homes.pop(); paint(); } });
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), create, label(materialise, 'materialise'), status,
    label(decks, 'decks'), label(deckLabel, 'deck_label'), label(weight, 'damage_weight'), addDeck, removeDeck,
    svg, label(rooms, 'rooms'), label(roomDeck, 'room_deck'), ...Object.entries(roomFields).map(([key, control]) => label(control, key)), addRoom, removeRoom,
    label(connections, 'connections'), label(from, 'from'), label(to, 'to'), label(travel, 'travel_secs'), label(boundary, 'boundary'), addConnection, removeConnection,
    label(system, 'system'), label(placement, 'placement'), el('h4', null, 'homes'), homes, addTeam, removeTeam, save);
  if (attach) root.append(node);
  let dependencies = null, inspection = null, form = null, observed = null, deckId = null, roomId = null, connectionId = null;
  let disposed = false, loading = false, generation = 0, drag = null;
  const room = () => form?.compartment.find(r => r.id === roomId), connection = () => form?.connection.find(c => c.id === connectionId);
  function controls() {
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = disposed || loading || busy() || !inspection || (!form && control !== create && control !== template);
    for (const control of [...Object.values(roomFields), roomDeck]) control.disabled ||= !room();
    for (const control of [from, to, travel, boundary, removeConnection]) control.disabled ||= !connection();
    removeRoom.disabled ||= !room(); removeDeck.disabled ||= !form || form.deck.length < 2;
    addConnection.disabled ||= !form || form.compartment.length < 2;
    svg.setAttribute('aria-disabled', String(disposed || loading || busy() || !form));
  }
  function fill(select, rows, selected) { select.replaceChildren(); for (const row of rows) option(select, row.id, wireText(row.label || row.id)); select.value = selected || ''; }
  function paintMap() {
    svg.replaceChildren(); if (!form) return;
    const rows = form.compartment.filter(r => r.deck === deckId && [r.x, r.z, r.width, r.depth].every(Number.isFinite) && r.width > 0 && r.depth > 0);
    const minX = Math.min(0, ...rows.map(r => r.x)), minZ = Math.min(0, ...rows.map(r => r.z));
    const width = Math.max(10, ...rows.map(r => r.x + r.width)) - minX, depth = Math.max(10, ...rows.map(r => r.z + r.depth)) - minZ;
    if (!drag) svg.setAttribute('viewBox', `${minX - 2} ${minZ - 2} ${width + 4} ${depth + 4}`);
    const draw = tag => doc.createElementNS(svg.namespaceURI, tag);
    for (const c of form.connection) {
      const a = rows.find(r => r.id === c.from), b = rows.find(r => r.id === c.to); if (!a || !b) continue;
      const line = draw('line'); for (const [key, value] of Object.entries({ x1: a.x + a.width / 2, y1: a.z + a.depth / 2, x2: b.x + b.width / 2, y2: b.z + b.depth / 2, stroke: 'currentColor', 'stroke-width': 0.25 })) line.setAttribute(key, value); svg.append(line);
    }
    for (const r of rows) {
      const rect = draw('rect'); for (const [key, value] of Object.entries({ x: r.x, y: r.z, width: r.width, height: r.depth, fill: r.id === roomId ? '#336688' : '#223344', stroke: 'currentColor', 'stroke-width': 0.2, tabindex: 0, role: 'button', 'data-room': r.id, 'aria-label': `${wireText(r.label || r.id)}: ${r.width} × ${r.depth} m` })) rect.setAttribute(key, value);
      const select = () => { roomId = r.id; paintMap(); paintProperties(); };
      rect.addEventListener('click', select); rect.addEventListener('keydown', event => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); select();
        [...svg.querySelectorAll('[role="button"][data-room]')].find(node => node.dataset.room === r.id)?.focus?.(); } });
      svg.append(rect); const text = draw('text'); text.setAttribute('x', r.x + 0.5); text.setAttribute('y', r.z + 1.5); text.setAttribute('font-size', 1); text.setAttribute('fill', 'white'); text.setAttribute('pointer-events', 'none'); text.textContent = wireText(r.label || r.id); svg.append(text);
      if (r.id === roomId) { const handle = draw('rect'); for (const [key, value] of Object.entries({ x: r.x + r.width - 0.5, y: r.z + r.depth - 0.5, width: 1, height: 1, fill: 'white', 'data-room': r.id, 'data-resize': 'true', 'aria-label': t('workshop.interior.resize') })) handle.setAttribute(key, value); svg.append(handle); }
    }
  }
  function paintProperties() {
    for (const [key, control] of Object.entries(roomFields)) control.value = room()?.[key] ?? '';
    fill(roomDeck, form?.deck || [], room()?.deck);
    for (const rect of svg.querySelectorAll('rect[role="button"][data-room]')) rect.setAttribute('fill', rect.dataset.room === roomId ? '#336688' : '#223344');
    rooms.value = roomId || ''; controls();
  }
  function paint() {
    if (!form) { svg.replaceChildren(); controls(); return; }
    if (!form.deck.some(d => d.id === deckId)) deckId = form.deck[0]?.id;
    const deck = form.deck.find(d => d.id === deckId); fill(decks, form.deck, deckId); deckLabel.value = deck?.label || ''; weight.value = deck?.damage_weight ?? '';
    const candidates = form.compartment.filter(r => r.deck === deckId); if (!candidates.some(r => r.id === roomId)) roomId = candidates[0]?.id;
    fill(rooms, candidates, roomId); paintMap(); paintProperties();
    if (!form.connection.some(c => c.id === connectionId)) connectionId = form.connection[0]?.id;
    fill(connections, form.connection, connectionId); fill(from, form.compartment, connection()?.from); fill(to, form.compartment, connection()?.to); travel.value = connection()?.travel_secs ?? '';
    boundary.replaceChildren(); option(boundary, '', t('workshop.interior.fixed')); for (const s of inspection.systems.filter(s => s.kind === 'access-gate')) option(boundary, s.id); boundary.value = connection()?.boundary || '';
    const previous = system.value; fill(system, inspection.systems, previous || inspection.systems[0]?.id); fill(placement, form.compartment, form.placements[system.value]);
    homes.replaceChildren(); form.team_homes.forEach((home, index) => { const select = el('select', 'home-' + index); fill(select, [{ id: '', label: 'workshop.interior.choose_home' }, ...form.compartment], home); select.addEventListener('change', () => { form.team_homes[index] = select.value; }); const row = label(select, 'home'); row.prepend(`${index + 1} `); homes.append(row); }); controls();
  }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const previous = template.value, paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    template.replaceChildren(); for (const path of paths) option(template, path); template.value = paths.includes(previous) ? previous : paths[0] || '';
    const revision = JSON.stringify([draft.sourceRevision, template.value, dependencies]); if (!force && observed === revision) { controls(); return; }
    if (!template.value) { inspection = null; controls(); return; }
    try { inspection = inspectInterior(draft, dependencies, template.value); form = clone(inspection.interior); observed = revision; materialise.checked = false; status.textContent = t(form ? 'workshop.interior.ready' : 'workshop.interior.empty'); paint(); }
    catch (error) { inspection = null; status.textContent = t('workshop.interior.refused', { reason: error.message }); controls(); }
  }
  async function load() { if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const result = await runtime.dependencies(); if (!disposed && token === generation) dependencies = result; }
    catch (error) { if (!disposed) status.textContent = t('workshop.interior.refused', { reason: error.message }); }
    finally { loading = false; if (!disposed) inspect(); }
  }
  async function apply() { if (busy() || !form || loading) return;
    const token = ++generation, draft = getDraft(), operation = { path: template.value, interior: clone(form), materialise: materialise.checked, teamCount: form.team_homes.length };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyInteriorOperation({ draft, provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.interior.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  template.addEventListener('change', () => { deckId = roomId = connectionId = null; inspect(true); });
  decks.addEventListener('change', () => { deckId = decks.value; roomId = null; paint(); }); rooms.addEventListener('change', () => { roomId = rooms.value; paintMap(); paintProperties(); });
  roomDeck.addEventListener('change', () => { if (!room()) return; room().deck = roomDeck.value; deckId = roomDeck.value; paint(); });
  connections.addEventListener('change', () => { connectionId = connections.value; paint(); });
  deckLabel.addEventListener('input', () => { const deck = form?.deck.find(d => d.id === deckId); if (deck) deck.label = deckLabel.value; });
  weight.addEventListener('input', () => { const deck = form?.deck.find(d => d.id === deckId); if (deck) deck.damage_weight = weight.valueAsNumber; });
  for (const [key, control] of Object.entries(roomFields)) control.addEventListener('input', () => { if (!room()) return; room()[key] = control.type === 'number' ? control.valueAsNumber : control.value; paintMap(); });
  for (const [control, key] of [[from, 'from'], [to, 'to'], [travel, 'travel_secs'], [boundary, 'boundary']]) control.addEventListener('change', () => { if (!connection()) return;
    if (key === 'boundary' && !control.value) delete connection()[key]; else connection()[key] = key === 'travel_secs' ? control.valueAsNumber : control.value; paintMap(); });
  system.addEventListener('change', () => { placement.value = form?.placements[system.value] || ''; }); placement.addEventListener('change', () => { if (form) form.placements[system.value] = placement.value; });
  svg.addEventListener('pointerdown', event => { if (busy() || !form || !event.target.dataset.room) return; const selected = form.compartment.find(r => r.id === event.target.dataset.room); if (!selected) return;
    roomId = selected.id; paintProperties(); const bounds = svg.getBoundingClientRect(), view = svg.getAttribute('viewBox').split(' ').map(Number);
    if (!bounds.width || !bounds.height) return; const scale = Math.min(bounds.width / view[2], bounds.height / view[3]);
    drag = { pointer: event.pointerId, x: selected.x, z: selected.z, width: selected.width, depth: selected.depth,
      resize: event.target.dataset.resize === 'true', clientX: event.clientX, clientY: event.clientY, scale };
    try { svg.setPointerCapture(event.pointerId); } catch (_) { /* synthetic pointer */ } event.preventDefault();
  });
  svg.addEventListener('pointermove', event => { if (busy() || disposed || loading || !drag || drag.pointer !== event.pointerId || !room()) return;
    const dx = (event.clientX - drag.clientX) / drag.scale, dz = (event.clientY - drag.clientY) / drag.scale;
    if (drag.resize) { room().width = Math.max(0.1, drag.width + dx); room().depth = Math.max(0.1, drag.depth + dz); }
    else { room().x = drag.x + dx; room().z = drag.z + dz; } paintMap(); paintProperties(); });
  const endDrag = () => { drag = null; paintMap(); }; svg.addEventListener('pointerup', endDrag); svg.addEventListener('pointercancel', endDrag);
  inspect(); void load(); return { node, refresh() { inspect(); void load(); }, dispose() { disposed = true; generation += 1; node.remove(); } };
}
