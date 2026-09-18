import { definitionsSnapshot, snapshotIsCurrent, factionSlugPath, enemyChoices, factionForm, complianceForm,
  complianceIsSeconds, complianceIsResponse, planFactionEdits, ratingForm, planRatingEdits, newRung, rungNames, findingsAt,
  draftFirst } from '../editor/workshop-definitions.js';
import { t } from './strings.js';

const PREFIX = 'workshop-definitions';
/** Labels only; which keys exist and what kind each is comes from the catalog. */
const COMPLIANCE_LABELS = { ack_secs: 'workshop.definitions.ack_secs', decide_secs: 'workshop.definitions.decide_secs',
  hold: 'workshop.definitions.hold', divert: 'workshop.definitions.divert', dock: 'workshop.definitions.dock',
  refusal: 'workshop.definitions.refusal', refusal_reason: 'workshop.definitions.refusal' };

/** Specialised runtime-backed forms over faction and console-complexity
 * definitions (issue #1474), shaped like the model form: a reading of the draft
 * through the runtime, edits planned in the pure module, and ONE exact-source
 * edit per member per Apply that lands in the ordinary Workshop history. The
 * selections are local presentation, never simulation inputs. */
export function mountWorkshopDefinitions({ root, runtime, draft, busy, setBusy, changed, win, attach = true }) {
  const doc = root.ownerDocument;
  const node = (tag, id, attrs = {}) => {
    const value = doc.createElement(tag);
    if (id) value.textContent = t(id);
    for (const [key, item] of Object.entries(attrs)) value.setAttribute(key, item);
    return value;
  };
  const labelled = (textId, control, extra = null) => {
    const label = node('label', textId, { for: control.id });
    if (extra) label.textContent = extra;
    return label;
  };
  const option = (value, label) => { const item = node('option', null, { value }); item.textContent = label; return item; };
  const fieldset = legendId => { const set = node('fieldset'); set.append(node('legend', legendId)); return set; };
  const knownId = (error, fallback = 'workshop.inspector_refused') =>
    (typeof error?.message === 'string' && error.message.startsWith('workshop.') ? error.message : fallback);

  // A dock panel carries its own header, so this is a plain grouping element.
  const section = node('section', null, { class: 'workshop-definitions', id: PREFIX });
  const refreshButton = node('button', 'workshop.definitions.refresh', { type: 'button', id: `${PREFIX}-refresh` });
  const status = node('p', null, { role: 'status', tabindex: '-1', id: `${PREFIX}-status` });
  const factionsSet = fieldset('workshop.definitions.factions');
  const faction = node('select', null, { id: `${PREFIX}-faction` });
  const factionFormNode = node('div', null, { class: 'workshop-definitions-form', id: `${PREFIX}-faction-form` });
  const applyFactionButton = node('button', 'workshop.definitions.apply', { type: 'button', id: `${PREFIX}-apply-faction` });
  const deleteFactionButton = node('button', 'workshop.definitions.delete_faction', { type: 'button', id: `${PREFIX}-delete-faction` });
  const newFactionSet = fieldset('workshop.definitions.new_faction');
  const newName = node('input', null, { type: 'text', maxlength: '64', id: `${PREFIX}-new-faction`, spellcheck: 'false' });
  const createButton = node('button', 'workshop.definitions.create', { type: 'button', id: `${PREFIX}-create` });
  newFactionSet.append(labelled('workshop.definitions.name', newName), newName, createButton);
  const factionActions = node('div', null, { class: 'workshop-definitions-row' });
  factionActions.append(applyFactionButton, deleteFactionButton);
  factionsSet.append(labelled('workshop.definitions.faction', faction), faction, factionFormNode, factionActions, newFactionSet);
  const hullsSet = fieldset('workshop.definitions.hulls');
  const hull = node('select', null, { id: `${PREFIX}-hull` });
  const station = node('select', null, { id: `${PREFIX}-station` });
  const stationFormNode = node('div', null, { class: 'workshop-definitions-form', id: `${PREFIX}-station-form` });
  const applyHullButton = node('button', 'workshop.definitions.apply', { type: 'button', id: `${PREFIX}-apply-hull` });
  hullsSet.append(labelled('workshop.definitions.hull', hull), hull, labelled('workshop.definitions.station', station), station,
    stationFormNode, applyHullButton);
  const findingsSet = fieldset('workshop.definitions.findings');
  const findingsList = node('ul', null, { id: `${PREFIX}-findings`, class: 'workshop-definitions-findings' });
  findingsSet.append(findingsList);
  section.append(node('p', 'workshop.definitions.hint'), refreshButton, status, factionsSet, hullsSet, findingsSet);
  // A docked panel is placed by the renderer, which moves the node into its frame.
  // Attaching here as well would strand the node in `root` whenever the stored
  // layout has the panel closed, because a closed panel is never framed.
  if (attach) root.append(section);

  let disposed = false, reading = null, testHidden = false, factionState = null, stationState = null;
  let preferredFaction = null;
  const show = (id, error = false, params = undefined) => {
    status.textContent = t(id, params);
    status.setAttribute('role', error ? 'alert' : 'status');
    if (error) status.focus();
  };
  /** A removal rebuilds its form, which takes the pressed control out of the
   * document; focus would fall to the body and a keyboard user would Tab from
   * the top again. The first present candidate takes it instead. */
  const focusFirst = (...ids) => {
    for (const id of ids) {
      const target = id && section.querySelector(`#${PREFIX}-${id}`);
      if (target) { target.focus(); return; }
    }
  };
  const fresh = () => Boolean(reading) && snapshotIsCurrent(reading, draft());
  const catalog = () => reading?.catalog;
  const selectedFaction = () => (catalog()?.factions || []).find(entry => entry.path === faction.value) || null;
  const selectedHull = () => (catalog()?.hulls || []).find(entry => entry.path === hull.value) || null;
  const stationIndex = () => (station.value === '' ? null : Number(station.value));
  const selectedStation = () => selectedHull()?.stations?.[stationIndex()] || null;
  const complianceDefaults = () => catalog()?.defaults?.compliance || {};

  function refresh({ hidden = testHidden } = {}) {
    testHidden = hidden;
    section.hidden = hidden;
    const current = draft();
    const held = busy();
    const live = fresh();
    refreshButton.disabled = held || !current || typeof runtime?.definitions !== 'function';
    if (reading && !live) show('workshop.inspector_stale');
    if (!current) show('workshop.definitions.empty');
    faction.disabled = held || !live || !faction.options.length;
    const editableFaction = live && selectedFaction()?.origin === 'draft';
    for (const control of factionFormNode.querySelectorAll('input, select, button')) control.disabled = held || !editableFaction;
    applyFactionButton.disabled = deleteFactionButton.disabled = held || !editableFaction;
    newName.disabled = createButton.disabled = held || !live || typeof runtime?.newFaction !== 'function';
    hull.disabled = held || !live || !hull.options.length;
    station.disabled = held || !live || !station.options.length;
    const editableHull = live && selectedHull()?.origin === 'draft' && Boolean(selectedStation());
    for (const control of stationFormNode.querySelectorAll('input, select, button')) {
      control.disabled = held || !editableHull || control.dataset.readonly === 'true';
    }
    applyHullButton.disabled = held || !editableHull;
  }

  function renderSelectors() {
    const wantedFaction = preferredFaction || faction.value;
    preferredFaction = null;
    const factions = draftFirst(catalog()?.factions);
    faction.replaceChildren(...factions.map(entry => option(entry.path,
      entry.origin === 'draft' ? `${entry.name ?? entry.path}` : `${entry.name ?? entry.path} (${entry.origin})`)));
    if (factions.some(entry => entry.path === wantedFaction)) faction.value = wantedFaction;
    const wantedHull = hull.value;
    const hulls = draftFirst(catalog()?.hulls);
    hull.replaceChildren(...hulls.map(entry => option(entry.path, entry.origin === 'draft' ? entry.path : `${entry.path} (${entry.origin})`)));
    if (hulls.some(entry => entry.path === wantedHull)) hull.value = wantedHull;
    renderStations();
    resetFactionForm();
    renderFindings();
  }

  function renderStations() {
    const wanted = station.value;
    const stations = selectedHull()?.stations || [];
    station.replaceChildren(...stations.map((entry, index) => option(String(index), entry.name ? `${entry.id} — ${entry.name}` : entry.id)));
    if (stations.some((_entry, index) => String(index) === wanted)) station.value = wanted;
    resetStationForm();
  }

  function resetFactionForm() {
    const definition = selectedFaction();
    factionState = definition ? factionForm(definition, complianceDefaults()) : null;
    renderFactionForm();
  }

  function resetStationForm() {
    const current = selectedStation();
    stationState = current ? ratingForm(current) : null;
    renderStationForm();
  }

  function renderFactionForm() {
    factionFormNode.replaceChildren();
    const definition = selectedFaction();
    if (!definition || !factionState) return;
    const form = factionState;
    if (definition.origin !== 'draft') {
      const note = node('p', null, { class: 'workshop-definitions-origin' });
      note.textContent = t('workshop.definitions.read_only_origin', { origin: definition.origin });
      factionFormNode.append(note);
    }
    const name = node('input', null, { type: 'text', id: `${PREFIX}-name`, spellcheck: 'false' });
    name.value = form.name;
    name.addEventListener('input', () => { form.name = name.value; });
    const display = node('input', null, { type: 'text', id: `${PREFIX}-display-name`, spellcheck: 'false' });
    display.value = form.display_name;
    display.addEventListener('input', () => { form.display_name = display.value; });
    factionFormNode.append(labelled('workshop.definitions.name', name), name,
      labelled('workshop.definitions.display_name', display), display);
    // Enemies: the list the form will write, each entry removable, plus the
    // factions it could still add — never itself, never one already listed.
    const enemies = fieldset('workshop.definitions.enemies');
    const list = node('ul', null, { id: `${PREFIX}-enemies` });
    const choices = catalog()?.choices || {};
    form.enemies.forEach((uuid, index) => {
      const row = node('li');
      const known = (choices.factions || []).find(entry => entry.uuid === uuid);
      const label = node('span');
      label.textContent = known?.name ? `${known.name} (${uuid})` : uuid;
      const remove = node('button', 'workshop.definitions.remove', { type: 'button', id: `${PREFIX}-enemy-remove-${index}`,
        'aria-label': `${t('workshop.definitions.remove')}: ${known?.name || uuid}` });
      remove.addEventListener('click', () => {
        form.enemies.splice(index, 1); renderFactionForm(); refresh();
        // The entry that took this place, else the last one left, else the add control.
        focusFirst(`enemy-remove-${index}`, `enemy-remove-${index - 1}`, 'add-enemy');
      });
      row.append(label, remove); list.append(row);
    });
    const addSelect = node('select', null, { id: `${PREFIX}-add-enemy` });
    addSelect.replaceChildren(...enemyChoices(choices, definition, form.enemies).map(entry => option(entry.uuid,
      entry.name ? `${entry.name} (${entry.origin})` : entry.uuid)));
    const addButton = node('button', 'workshop.definitions.add', { type: 'button', id: `${PREFIX}-add-enemy-button` });
    addButton.addEventListener('click', () => {
      if (!addSelect.value) return;
      form.enemies.push(addSelect.value); renderFactionForm(); refresh();
      factionFormNode.querySelector(`#${PREFIX}-add-enemy`)?.focus();
    });
    const addRow = node('div', null, { class: 'workshop-definitions-row' });
    addRow.append(labelled('workshop.definitions.add_enemy', addSelect), addSelect, addButton);
    enemies.append(list, addRow);
    factionFormNode.append(enemies);
    // Compliance: absent until materialised from the runtime's own defaults.
    const compliance = fieldset('workshop.definitions.compliance');
    if (!form.compliance) {
      const materialise = node('button', 'workshop.definitions.materialise_compliance', { type: 'button', id: `${PREFIX}-materialise-compliance` });
      materialise.addEventListener('click', () => {
        form.compliance = complianceForm(null, complianceDefaults()); renderFactionForm(); refresh();
        factionFormNode.querySelector(`#${PREFIX}-ack-secs`)?.focus();
      });
      compliance.append(materialise);
    } else {
      const defaults = complianceDefaults();
      const responses = choices.order_responses || [];
      for (const key of Object.keys(form.compliance)) {
        const id = `${PREFIX}-${key.replace(/_/g, '-')}`;
        let control;
        if (complianceIsSeconds(key, defaults)) {
          control = node('input', null, { type: 'number', step: '1', id });
          control.value = form.compliance[key];
        } else if (complianceIsResponse(key, defaults, responses)) {
          control = node('select', null, { id });
          const values = responses.includes(form.compliance[key]) ? responses : [form.compliance[key], ...responses];
          control.replaceChildren(...values.map(value => option(value, value)));
          control.value = form.compliance[key];
        } else {
          control = node('input', null, { type: 'text', id, spellcheck: 'false' });
          control.value = form.compliance[key];
        }
        control.addEventListener('input', () => { form.compliance[key] = control.value; });
        control.addEventListener('change', () => { form.compliance[key] = control.value; });
        const label = node('label', null, { for: id });
        label.textContent = COMPLIANCE_LABELS[key] ? t(COMPLIANCE_LABELS[key]) : key;
        compliance.append(label, control);
      }
    }
    factionFormNode.append(compliance);
    if (definition.unknown_keys?.length) {
      const unknown = fieldset('workshop.definitions.unknown_keys');
      const keys = node('ul', null, { id: `${PREFIX}-unknown-keys` });
      for (const key of definition.unknown_keys) { const row = node('li'); row.textContent = key; keys.append(row); }
      unknown.append(keys); factionFormNode.append(unknown);
    }
  }

  function renderStationForm() {
    stationFormNode.replaceChildren();
    const current = selectedHull(), target = selectedStation();
    if (!current || !target || !stationState) {
      if (current && !(current.stations || []).length) stationFormNode.append(node('p', 'workshop.definitions.no_stations'));
      return;
    }
    const form = stationState;
    if (current.origin !== 'draft') {
      const note = node('p', null, { class: 'workshop-definitions-origin' });
      note.textContent = t('workshop.definitions.read_only_origin', { origin: current.origin });
      stationFormNode.append(note);
    }
    // Whether the station seats a human is #1481's to author; shown, not edited.
    const seeking = node('input', null, { type: 'checkbox', id: `${PREFIX}-human-seeking`, 'data-readonly': 'true' });
    seeking.checked = Boolean(target.human_seeking);
    stationFormNode.append(seeking, labelled('workshop.definitions.human_seeking', seeking));
    // A visiting rating only means something on a station that seats a human;
    // the runtime refuses the hull otherwise. A station that seats none offers
    // only what it already has, so an authored one can be cleared but none set.
    const visiting = node('select', null, { id: `${PREFIX}-visiting-rating` });
    const names = target.human_seeking ? rungNames(form).filter(Boolean) : [];
    const values = form.visiting_rating && !names.includes(form.visiting_rating) ? [form.visiting_rating, ...names] : names;
    visiting.replaceChildren(option('', t('workshop.definitions.none')), ...values.map(name => option(name, name)));
    visiting.value = form.visiting_rating;
    if (!values.length) visiting.dataset.readonly = 'true';
    visiting.addEventListener('change', () => { form.visiting_rating = visiting.value; });
    stationFormNode.append(labelled('workshop.definitions.visiting_rating', visiting), visiting);
    const rungs = fieldset('workshop.definitions.rungs');
    const grid = node('div', null, { class: 'workshop-definitions-rungs' });
    const rules = catalog()?.choices?.ai_rules || [];
    form.ratings.forEach((rung, position) => {
      if (rung.removed) return;
      const original = rung.index == null ? null : (target.ratings || []).find(entry => entry.index === rung.index);
      const set = node('fieldset', null, { class: 'workshop-definitions-rung' });
      const legend = node('legend'); legend.textContent = t('workshop.definitions.rung', { name: rung.name }); set.append(legend);
      const name = node('input', null, { type: 'text', id: `${PREFIX}-rung-${position}-name`, spellcheck: 'false' });
      name.value = rung.name;
      name.addEventListener('input', () => { rung.name = name.value; });
      set.append(labelled('workshop.definitions.rung_name', name), name);
      const systems = fieldset('workshop.definitions.automated_systems');
      const systemList = node('ul');
      (target.systems || []).forEach((system, index) => {
        const row = node('li');
        const box = node('input', null, { type: 'checkbox', id: `${PREFIX}-rung-${position}-system-${index}` });
        box.checked = rung.automated_systems.includes(system.id);
        box.addEventListener('change', () => {
          rung.automated_systems = box.checked ? [...rung.automated_systems.filter(id => id !== system.id), system.id]
            : rung.automated_systems.filter(id => id !== system.id);
        });
        row.append(box, labelled(null, box, system.kind ? `${system.id} (${system.kind})` : system.id));
        systemList.append(row);
      });
      // An automated id the station does not own stays visible as a checked
      // row the author can clear, carrying the finding that explains it.
      const owned = new Set((target.systems || []).map(system => system.id));
      rung.automated_systems.filter(id => !owned.has(id)).forEach((id, index) => {
        const row = node('li', null, { class: 'workshop-definitions-unavailable' });
        const box = node('input', null, { type: 'checkbox', id: `${PREFIX}-rung-${position}-unowned-${index}` });
        box.checked = true;
        box.addEventListener('change', () => {
          rung.automated_systems = rung.automated_systems.filter(entry => entry !== id); renderStationForm(); refresh();
          focusFirst(`rung-${position}-unowned-${index}`, `rung-${position}-unowned-${index - 1}`, `rung-${position}-system-0`,
            `rung-${position}-name`);
        });
        const line = original?.automated_systems?.find(entry => entry.id === id)?.line;
        const messages = findingsAt(catalog(), current.path, line).map(finding => finding.message);
        row.append(box, labelled(null, box, [`${id} — ${t('workshop.definitions.unavailable_system')}`, ...messages].join(' ')));
        systemList.append(row);
      });
      systems.append(systemList); set.append(systems);
      const tuning = fieldset('workshop.definitions.ai_rules');
      const ruleList = node('ul');
      // A rule the reading lists but the runtime no longer names stays visible
      // as a checked row, so it can be cleared rather than silently kept.
      const known = Array.isArray(rules) ? rules : [];
      [...known, ...rung.ai_rules.filter(rule => !known.includes(rule))].forEach((rule, index) => {
        const row = node('li');
        const box = node('input', null, { type: 'checkbox', id: `${PREFIX}-rung-${position}-rule-${index}` });
        box.checked = rung.ai_rules.includes(rule);
        box.addEventListener('change', () => {
          rung.ai_rules = box.checked ? [...rung.ai_rules.filter(entry => entry !== rule), rule] : rung.ai_rules.filter(entry => entry !== rule);
        });
        row.append(box, labelled(null, box, rule));
        ruleList.append(row);
      });
      tuning.append(ruleList); set.append(tuning);
      const remove = node('button', 'workshop.definitions.remove', { type: 'button', id: `${PREFIX}-rung-${position}-remove`,
        'aria-label': `${t('workshop.definitions.remove')}: ${rung.name}` });
      remove.addEventListener('click', () => {
        if (rung.index == null) form.ratings.splice(position, 1); else rung.removed = true;
        renderStationForm(); refresh();
        // Rung ids follow the form's positions: the next rung left, else the
        // one before, else the add control.
        const left = form.ratings.map((entry, at) => (entry.removed ? null : at)).filter(at => at != null);
        const nearest = left.find(at => at > position) ?? left.filter(at => at < position).pop();
        focusFirst(nearest == null ? null : `rung-${nearest}-name`, 'new-rung');
      });
      set.append(remove);
      grid.append(set);
    });
    rungs.append(grid);
    const newRungName = node('input', null, { type: 'text', maxlength: '64', id: `${PREFIX}-new-rung`, spellcheck: 'false' });
    const addRung = node('button', 'workshop.definitions.add_rung_button', { type: 'button', id: `${PREFIX}-add-rung` });
    addRung.addEventListener('click', () => {
      const name = newRungName.value.trim();
      if (!name) { show('workshop.definitions.invalid_name', true); return; }
      if (rungNames(form).includes(name)) { show('workshop.definitions.rung_exists', true); return; }
      form.ratings.push(newRung(name)); renderStationForm(); refresh();
      stationFormNode.querySelector(`#${PREFIX}-new-rung`)?.focus();
    });
    const addRow = node('div', null, { class: 'workshop-definitions-row' });
    addRow.append(labelled('workshop.definitions.add_rung', newRungName), newRungName, addRung);
    rungs.append(addRow);
    stationFormNode.append(rungs);
  }

  function renderFindings() {
    const records = catalog()?.findings || [];
    findingsList.replaceChildren(...records.map(record => {
      const row = node('li', null, { 'data-severity': record.severity });
      const location = `${record.file}${record.line ? `:${record.line}` : ''}`;
      // The severity is a word, never a colour alone.
      row.textContent = `${location} — ${t(`workshop.severity.${record.severity}`)}: ${record.message}`;
      return row;
    }));
    if (!records.length) findingsList.append(node('li', 'workshop.definitions.no_findings'));
  }

  /** Read the draft through the runtime. An answer for a draft that moved while
   * it was being read is discarded rather than shown against newer source. A
   * form holding UNAPPLIED edits over a member that is byte-for-byte what the
   * previous reading held keeps them: applying the faction form must not throw
   * away the rung edits typed beside it, nor the reverse. An untouched form is
   * rebuilt from the new reading like everything else. */
  async function reload({ announce = true } = {}) {
    const candidate = draft();
    if (!candidate) return;
    const snapshot = definitionsSnapshot(candidate);
    const result = await runtime.definitions(snapshot.files);
    if (disposed || !snapshotIsCurrent(snapshot, draft())) return;
    if (!result || !Array.isArray(result.factions) || !Array.isArray(result.hulls)) throw new Error('workshop.inspector_refused');
    const dirty = (state, fresh) => Boolean(state) && Boolean(fresh) && JSON.stringify(state) !== JSON.stringify(fresh);
    const previousFaction = selectedFaction(), previousStation = selectedStation();
    const pending = {
      faction: dirty(factionState, previousFaction && factionForm(previousFaction, complianceDefaults())) ? factionState : null,
      factionPath: faction.value, factionSource: reading?.files?.[faction.value],
      station: dirty(stationState, previousStation && ratingForm(previousStation)) ? stationState : null,
      hullPath: hull.value, stationIndex: station.value, hullSource: reading?.files?.[hull.value],
    };
    reading = { ...snapshot, catalog: result };
    renderSelectors();
    if (pending.faction && faction.value === pending.factionPath && snapshot.files[faction.value] === pending.factionSource) {
      factionState = pending.faction; renderFactionForm();
    }
    if (pending.station && hull.value === pending.hullPath && station.value === pending.stationIndex
      && snapshot.files[hull.value] === pending.hullSource) {
      stationState = pending.station; renderStationForm();
    }
    if (announce) show('workshop.definitions.refreshed');
  }

  async function guarded(action) {
    setBusy(true);
    try { await action(); }
    catch (error) { if (!disposed) show(knownId(error), true); }
    finally { if (!disposed) { setBusy(false); refresh(); } }
  }

  /** ONE runtime edit for ONE member, then ONE draft edit. The draft is never
   * touched before the runtime answers, and an answer for a draft that moved in
   * the meantime is refused as stale rather than written over newer source. */
  async function commit(read, path, edits) {
    const source = read.files[path];
    const result = await runtime.edit(source, { document_path: path, expected_source: source, edits });
    if (disposed) return;
    if (!snapshotIsCurrent(read, draft())) throw new Error('workshop.inspector_stale');
    if (typeof result !== 'string') throw new Error('workshop.inspector_refused');
    const current = draft();
    if (current.edit(path, result)) changed(path);
    show('workshop.changed');
    // Re-read so the forms show what was written; a failed re-read leaves the
    // reading honestly stale rather than reporting the landed edit as refused.
    await reload({ announce: false }).catch(() => {});
  }

  refreshButton.addEventListener('click', () => {
    if (busy() || refreshButton.disabled) return;
    void guarded(() => reload());
  });
  applyFactionButton.addEventListener('click', () => {
    if (busy() || applyFactionButton.disabled || !fresh()) return;
    const read = reading, definition = selectedFaction();
    if (!definition || !factionState) return;
    let edits;
    try { edits = planFactionEdits({ definition, form: factionState, defaults: complianceDefaults() }); }
    catch (error) { show(knownId(error), true); return; }
    if (!edits.length) { show('workshop.definitions.unchanged'); return; }
    void guarded(() => commit(read, definition.path, edits));
  });
  applyHullButton.addEventListener('click', () => {
    if (busy() || applyHullButton.disabled || !fresh()) return;
    const read = reading, current = selectedHull(), target = selectedStation();
    if (!current || !target || !stationState) return;
    let edits;
    try { edits = planRatingEdits({ station: target, stationIndex: stationIndex(), form: stationState }); }
    catch (error) { show(knownId(error), true); return; }
    if (!edits.length) { show('workshop.definitions.unchanged'); return; }
    void guarded(() => commit(read, current.path, edits));
  });
  createButton.addEventListener('click', () => {
    if (busy() || createButton.disabled) return;
    const current = draft(), name = newName.value.trim();
    let path;
    try { path = factionSlugPath(name); }
    catch (error) { show(knownId(error), true); return; }
    if (current.paths().includes(path)) { show('workshop.definitions.faction_exists', true); return; }
    const uuid = win.crypto.randomUUID();
    void guarded(async () => {
      const source = await runtime.newFaction(name, uuid);
      if (disposed) return;
      // The draft may have gained that member while the runtime was answering.
      if (draft() !== current || current.paths().includes(path)) throw new Error('workshop.definitions.faction_exists');
      if (typeof source !== 'string') throw new Error('workshop.inspector_refused');
      current.put(path, source);
      newName.value = '';
      preferredFaction = path;
      changed(path);
      show('workshop.changed');
      await reload({ announce: false }).catch(() => {});
    });
  });
  /** Confirmed, because one press otherwise takes away source whose only other
   * copy may be the imported archive — and undone by the ordinary history. The
   * references it leaves dangling become findings, which is the honest state. */
  deleteFactionButton.addEventListener('click', () => {
    if (busy() || deleteFactionButton.disabled) return;
    const current = draft(), definition = selectedFaction();
    if (!current || definition?.origin !== 'draft') return;
    if (!win.confirm(t('workshop.definitions.delete_confirm', { path: definition.path }))) return;
    if (!current.remove(definition.path)) return;
    changed(current.paths()[0] ?? definition.path);
    show('workshop.changed');
    void guarded(() => reload({ announce: false }));
  });
  faction.addEventListener('change', () => { resetFactionForm(); refresh(); });
  hull.addEventListener('change', () => { renderStations(); refresh(); });
  station.addEventListener('change', () => { resetStationForm(); refresh(); });
  show('workshop.definitions.empty');
  refresh();
  return { refresh, node: section, dispose() { disposed = true; section.remove(); } };
}
