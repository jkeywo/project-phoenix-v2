import { definitionsSnapshot, snapshotIsCurrent, findingsAt, draftFirst } from '../editor/workshop-definitions.js';
import { rootsForm, newRoot, moveRoot, offeredShips, planScenarioEdits, worldForm, extraWorldChoices, planExtraWorldEdits,
  worldSlugPath, worldTitle, refusalMessage, refusalStringId } from '../editor/workshop-composition.js';
import { t } from './strings.js';

const PREFIX = 'workshop-composition';

/** Runtime-backed forms over world composition and the manifest's scenario
 * entry points (issue #1475), shaped like the definitions form: a reading of
 * the draft through the runtime, edits planned in the pure module, and ONE
 * compose call per member per Apply that the runtime either lands as exact
 * source or refuses — a missing, cyclic, duplicate or disallowed reference
 * never reaches the draft. Dependency worlds are listed read-only with their
 * origin; script-driven load and unload references are listed, not edited.
 * The selections are local presentation, never simulation inputs. */
export function mountWorkshopComposition({ root, runtime, draft, busy, setBusy, changed, win, attach = true }) {
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
  const originText = origin => (origin ? String(origin) : t('workshop.composition.origin_missing'));
  const worldOption = entry => option(entry.path, entry.origin === 'draft' ? entry.path : `${entry.path} (${entry.origin})`);

  // A dock panel carries its own header, so this is a plain grouping element.
  const section = node('section', null, { class: 'workshop-composition', id: PREFIX });
  const refreshButton = node('button', 'workshop.composition.refresh', { type: 'button', id: `${PREFIX}-refresh` });
  const status = node('p', null, { role: 'status', tabindex: '-1', id: `${PREFIX}-status` });
  const rootsSet = fieldset('workshop.composition.roots');
  const manifestInfo = node('p', null, { id: `${PREFIX}-manifest`, class: 'workshop-composition-origin' });
  const rootsList = node('div', null, { class: 'workshop-composition-form', id: `${PREFIX}-roots` });
  const addRootSet = fieldset('workshop.composition.add_root');
  const addRootId = node('input', null, { type: 'text', maxlength: '64', id: `${PREFIX}-add-root-id`, spellcheck: 'false' });
  const addRootWorld = node('select', null, { id: `${PREFIX}-add-root-world` });
  const addRootButton = node('button', 'workshop.composition.add', { type: 'button', id: `${PREFIX}-add-root-button` });
  addRootSet.append(labelled('workshop.composition.add_root_id', addRootId), addRootId,
    labelled('workshop.composition.add_root_world', addRootWorld), addRootWorld, addRootButton);
  const applyRootsButton = node('button', 'workshop.composition.apply', { type: 'button', id: `${PREFIX}-apply-roots` });
  rootsSet.append(manifestInfo, rootsList, addRootSet, applyRootsButton);
  const worldsSet = fieldset('workshop.composition.worlds');
  const world = node('select', null, { id: `${PREFIX}-world` });
  const worldFormNode = node('div', null, { class: 'workshop-composition-form', id: `${PREFIX}-world-form` });
  const applyWorldButton = node('button', 'workshop.composition.apply', { type: 'button', id: `${PREFIX}-apply-world` });
  const newWorldSet = fieldset('workshop.composition.new_world');
  const newWorldTitle = node('input', null, { type: 'text', maxlength: '64', id: `${PREFIX}-new-world-title`, spellcheck: 'false' });
  const createWorldButton = node('button', 'workshop.composition.create_world', { type: 'button', id: `${PREFIX}-create-world` });
  newWorldSet.append(labelled('workshop.composition.new_world_title', newWorldTitle), newWorldTitle, createWorldButton);
  worldsSet.append(labelled('workshop.composition.world', world), world, worldFormNode, applyWorldButton, newWorldSet);
  const membersSet = fieldset('workshop.composition.members');
  const membersList = node('ul', null, { id: `${PREFIX}-members` });
  membersSet.append(membersList);
  const catalogueSet = fieldset('workshop.composition.catalogue');
  const catalogueList = node('ul', null, { id: `${PREFIX}-catalogue` });
  catalogueSet.append(catalogueList);
  const findingsSet = fieldset('workshop.composition.findings');
  const findingsList = node('ul', null, { id: `${PREFIX}-findings`, class: 'workshop-composition-findings' });
  findingsSet.append(findingsList);
  section.append(node('p', 'workshop.composition.hint'), refreshButton, status, rootsSet, worldsSet, membersSet, catalogueSet, findingsSet);
  // A docked panel is placed by the renderer, which moves the node into its frame.
  // Attaching here as well would strand the node in `root` whenever the stored
  // layout has the panel closed, because a closed panel is never framed.
  if (attach) root.append(section);

  let disposed = false, reading = null, testHidden = false, rootsState = null, worldState = null;
  let preferredWorld = null;
  const show = (id, error = false, params = undefined) => {
    status.textContent = t(id, params);
    status.setAttribute('role', error ? 'alert' : 'status');
    if (error) status.focus();
  };
  /** A removal or a move rebuilds its form, which takes the pressed control out
   * of the document; focus would fall to the body and a keyboard user would Tab
   * from the top again. The first present, enabled candidate takes it instead. */
  const focusFirst = (...ids) => {
    for (const id of ids) {
      const target = id && section.querySelector(`#${PREFIX}-${id}`);
      if (target && !target.disabled) { target.focus(); return; }
    }
  };
  const fresh = () => Boolean(reading) && snapshotIsCurrent(reading, draft());
  const catalog = () => reading?.catalog;
  const manifest = () => catalog()?.manifest || null;
  const choices = () => catalog()?.choices || {};
  const selectedWorld = () => (catalog()?.worlds || []).find(entry => entry.path === world.value) || null;

  function refresh({ hidden = testHidden } = {}) {
    testHidden = hidden;
    section.hidden = hidden;
    const current = draft();
    const held = busy();
    const live = fresh();
    refreshButton.disabled = held || !current || typeof runtime?.composition !== 'function';
    if (reading && !live) show('workshop.inspector_stale');
    if (!current) show('workshop.composition.empty');
    // The manifest is always the draft's own member: it is what a pack or a
    // project workspace IS, so its roots are editable whenever the reading is.
    const editableRoots = live && Boolean(manifest()) && Boolean(rootsState);
    for (const control of rootsList.querySelectorAll('input, select, button')) {
      control.disabled = held || !editableRoots || control.dataset.readonly === 'true';
    }
    addRootId.disabled = addRootWorld.disabled = addRootButton.disabled = held || !editableRoots;
    applyRootsButton.disabled = held || !editableRoots;
    world.disabled = held || !live || !world.options.length;
    const editableWorld = live && selectedWorld()?.origin === 'draft' && Boolean(worldState);
    for (const control of worldFormNode.querySelectorAll('input, select, button')) {
      control.disabled = held || !editableWorld || control.dataset.readonly === 'true';
    }
    applyWorldButton.disabled = held || !editableWorld;
    newWorldTitle.disabled = createWorldButton.disabled = held || !live || typeof runtime?.newWorld !== 'function';
  }

  function renderAll() {
    renderManifest();
    resetRootsForm();
    renderWorldSelector();
    renderMembers();
    renderCatalogue();
    renderFindings();
  }

  function renderManifest() {
    const current = manifest();
    if (!current) { manifestInfo.textContent = t('workshop.composition.no_manifest'); return; }
    if (current.kind === 'pack' && current.pack) {
      manifestInfo.textContent = t('workshop.composition.manifest_pack', { path: current.path, id: current.pack.id ?? '',
        version: current.pack.version ?? '', name: current.pack.name ?? '' });
    } else if (current.content) {
      manifestInfo.textContent = t('workshop.composition.manifest_project', { path: current.path, id: current.content.id ?? '',
        epoch: String(current.content.epoch ?? '') });
    } else manifestInfo.textContent = t('workshop.composition.manifest_plain', { path: current.path });
  }

  function resetRootsForm() {
    const current = manifest();
    rootsState = current ? rootsForm(current) : null;
    renderRoots();
  }

  function renderRoots() {
    rootsList.replaceChildren();
    const current = manifest();
    const worlds = draftFirst(choices().worlds);
    // Rebuilt on every root change; the world chosen for the next root survives it.
    const wantedAdd = addRootWorld.value;
    addRootWorld.replaceChildren(...worlds.map(worldOption));
    if (worlds.some(entry => entry.path === wantedAdd)) addRootWorld.value = wantedAdd;
    if (!current || !rootsState) return;
    const form = rootsState;
    const live = form.map((entry, at) => (entry.removed ? null : at)).filter(at => at != null);
    form.forEach((entry, position) => {
      if (entry.removed) return;
      const original = entry.index == null ? null : (current.scenarios || []).find(candidate => candidate.index === entry.index);
      const set = node('fieldset', null, { class: 'workshop-composition-root' });
      const legend = node('legend'); set.append(legend);
      const up = node('button', 'workshop.composition.move_up', { type: 'button', id: `${PREFIX}-root-${position}-up` });
      const down = node('button', 'workshop.composition.move_down', { type: 'button', id: `${PREFIX}-root-${position}-down` });
      const remove = node('button', 'workshop.composition.remove', { type: 'button', id: `${PREFIX}-root-${position}-remove` });
      // The legend and the buttons' accessible names carry the scenario id
      // and FOLLOW it as it is typed: nothing re-renders on a keystroke, so a
      // name fixed at render time would announce "Remove: alpha" over a root
      // already renamed to beta.
      const relabel = () => {
        legend.textContent = t('workshop.composition.root', { id: entry.id });
        for (const [control, textId] of [[up, 'workshop.composition.move_up'], [down, 'workshop.composition.move_down'],
          [remove, 'workshop.composition.remove']]) control.setAttribute('aria-label', `${t(textId)}: ${entry.id}`);
      };
      relabel();
      const id = node('input', null, { type: 'text', id: `${PREFIX}-root-${position}-id`, spellcheck: 'false' });
      id.value = entry.id;
      id.addEventListener('input', () => { entry.id = id.value; relabel(); });
      set.append(labelled('workshop.composition.root_id', id), id);
      // The reading's own world stays selectable even when no world of that
      // path exists, so an unresolved root is shown as it is, not moved.
      const worldSelect = node('select', null, { id: `${PREFIX}-root-${position}-world` });
      const known = worlds.some(candidate => candidate.path === entry.world);
      worldSelect.replaceChildren(...(known ? [] : [option(entry.world, entry.world
        ? `${entry.world} (${t('workshop.composition.origin_missing')})` : t('workshop.composition.none'))]), ...worlds.map(worldOption));
      worldSelect.value = entry.world;
      worldSelect.addEventListener('change', () => {
        entry.world = worldSelect.value;
        // The offered ships follow the world: a ship the new world does not
        // offer would be refused, so the boxes are redrawn from its list.
        renderRoots(); refresh();
        focusFirst(`root-${position}-world`);
      });
      set.append(labelled('workshop.composition.root_world', worldSelect), worldSelect);
      for (const message of findingsAt(catalog(), current.path, original?.world_line).map(finding => finding.message)) {
        const note = node('p', null, { class: 'workshop-composition-finding' }); note.textContent = message; set.append(note);
      }
      const label = node('input', null, { type: 'text', id: `${PREFIX}-root-${position}-label`, spellcheck: 'false' });
      label.value = entry.label;
      label.addEventListener('input', () => { entry.label = label.value; });
      set.append(labelled('workshop.composition.root_label', label), label);
      const ships = fieldset('workshop.composition.ships');
      const shipList = node('ul');
      const offered = offeredShips(catalog(), entry.world, original);
      offered.forEach((template, index) => {
        const row = node('li');
        const box = node('input', null, { type: 'checkbox', id: `${PREFIX}-root-${position}-ship-${index}` });
        box.checked = entry.ships.includes(template);
        box.addEventListener('change', () => {
          entry.ships = box.checked ? [...entry.ships.filter(path => path !== template), template] : entry.ships.filter(path => path !== template);
        });
        row.append(box, labelled(null, box, template));
        shipList.append(row);
      });
      // A ship the world does not offer stays visible as a checked row the
      // author can clear, carrying the finding that explains it.
      entry.ships.filter(template => !offered.includes(template)).forEach((template, index) => {
        const row = node('li', null, { class: 'workshop-composition-unavailable' });
        const box = node('input', null, { type: 'checkbox', id: `${PREFIX}-root-${position}-unoffered-${index}` });
        box.checked = true;
        box.addEventListener('change', () => {
          entry.ships = entry.ships.filter(path => path !== template); renderRoots(); refresh();
          focusFirst(`root-${position}-unoffered-${index}`, `root-${position}-unoffered-${index - 1}`, `root-${position}-ship-0`,
            `root-${position}-label`);
        });
        const line = original?.ships?.find(ship => ship.path === template)?.line;
        const messages = findingsAt(catalog(), current.path, line).map(finding => finding.message);
        row.append(box, labelled(null, box, [`${template} — ${t('workshop.composition.ship_not_offered')}`, ...messages].join(' ')));
        shipList.append(row);
      });
      if (!shipList.children.length) shipList.append(node('li', 'workshop.composition.no_ships'));
      ships.append(shipList); set.append(ships);
      const actions = node('div', null, { class: 'workshop-composition-row' });
      const rank = live.indexOf(position);
      // An end root has nowhere to go in that direction: the control stays in
      // place, disabled, so the row keeps the same shape for a keyboard user.
      if (rank === 0) up.dataset.readonly = 'true';
      if (rank === live.length - 1) down.dataset.readonly = 'true';
      const move = direction => () => {
        const target = moveRoot(form, position, direction);
        if (target == null) return;
        renderRoots(); refresh();
        const kind = direction < 0 ? 'up' : 'down', other = direction < 0 ? 'down' : 'up';
        focusFirst(`root-${target}-${kind}`, `root-${target}-${other}`, `root-${target}-id`);
      };
      up.addEventListener('click', move(-1));
      down.addEventListener('click', move(1));
      remove.addEventListener('click', () => {
        if (entry.index == null) form.splice(position, 1); else entry.removed = true;
        renderRoots(); refresh();
        // Root ids follow the form's positions: the next root left, else the
        // one before, else the add control.
        const left = form.map((candidate, at) => (candidate.removed ? null : at)).filter(at => at != null);
        const nearest = left.find(at => at >= position) ?? left.filter(at => at < position).pop();
        focusFirst(nearest == null ? null : `root-${nearest}-id`, 'add-root-id');
      });
      actions.append(up, down, remove);
      set.append(actions);
      rootsList.append(set);
    });
    if (!live.length) rootsList.append(node('p', 'workshop.composition.no_roots'));
    if (current.unknown_keys?.length) {
      const unknown = fieldset('workshop.composition.unknown_keys');
      const keys = node('ul', null, { id: `${PREFIX}-unknown-keys` });
      for (const key of current.unknown_keys) { const row = node('li'); row.textContent = key; keys.append(row); }
      unknown.append(keys); rootsList.append(unknown);
    }
  }

  function renderWorldSelector() {
    const wanted = preferredWorld || world.value;
    preferredWorld = null;
    const worlds = draftFirst(catalog()?.worlds);
    world.replaceChildren(...worlds.map(worldOption));
    if (worlds.some(entry => entry.path === wanted)) world.value = wanted;
    resetWorldForm();
  }

  function resetWorldForm() {
    const current = selectedWorld();
    worldState = current ? worldForm(current) : null;
    renderWorldForm();
  }

  function renderWorldForm() {
    worldFormNode.replaceChildren();
    const current = selectedWorld();
    if (!current || !worldState) return;
    const form = worldState;
    if (current.origin !== 'draft') {
      const note = node('p', null, { class: 'workshop-composition-origin' });
      note.textContent = t('workshop.composition.read_only_origin', { origin: current.origin });
      worldFormNode.append(note);
    }
    const title = node('p', null, { id: `${PREFIX}-world-title` });
    title.textContent = t('workshop.composition.world_title', { title: worldTitle(current) });
    worldFormNode.append(title);
    // Extra worlds: the list the form will write, each entry removable with its
    // origin beside it, plus the worlds it could still add — never itself,
    // never one already listed.
    const extras = fieldset('workshop.composition.extra_worlds');
    const list = node('ul', null, { id: `${PREFIX}-extra-worlds` });
    form.extra_worlds.forEach((path, index) => {
      const row = node('li');
      const read = (current.extra_worlds || []).find(entry => entry.path === path);
      const origin = read ? read.origin : (choices().worlds || []).find(entry => entry.path === path)?.origin ?? null;
      const label = node('span');
      const messages = findingsAt(catalog(), current.path, read?.line).map(finding => finding.message);
      label.textContent = [`${path} (${originText(origin)})`, ...messages].join(' ');
      const remove = node('button', 'workshop.composition.remove', { type: 'button', id: `${PREFIX}-extra-${index}-remove`,
        'aria-label': `${t('workshop.composition.remove')}: ${path}` });
      remove.addEventListener('click', () => {
        form.extra_worlds.splice(index, 1); renderWorldForm(); refresh();
        // The entry that took this place, else the last one left, else the add control.
        focusFirst(`extra-${index}-remove`, `extra-${index - 1}-remove`, 'add-extra');
      });
      row.append(label, remove); list.append(row);
    });
    if (!form.extra_worlds.length) list.append(node('li', 'workshop.composition.no_extra_worlds'));
    const addSelect = node('select', null, { id: `${PREFIX}-add-extra` });
    addSelect.replaceChildren(...draftFirst(extraWorldChoices(choices(), current, form.extra_worlds)).map(worldOption));
    const addButton = node('button', 'workshop.composition.add', { type: 'button', id: `${PREFIX}-add-extra-button` });
    addButton.addEventListener('click', () => {
      if (!addSelect.value) return;
      form.extra_worlds.push(addSelect.value); renderWorldForm(); refresh();
      worldFormNode.querySelector(`#${PREFIX}-add-extra`)?.focus();
    });
    const addRow = node('div', null, { class: 'workshop-composition-row' });
    addRow.append(labelled('workshop.composition.add_extra', addSelect), addSelect, addButton);
    extras.append(list, addRow);
    worldFormNode.append(extras);
    // Script-driven references are read-only here: editing Rhai is #1478's.
    const refs = fieldset('workshop.composition.script_refs');
    const refList = node('ul', null, { id: `${PREFIX}-script-refs` });
    for (const ref of current.script_refs || []) {
      const row = node('li');
      const source = ref.source === 'trigger' ? t('workshop.composition.source_trigger')
        : ref.source === 'inline-script' ? t('workshop.composition.source_inline') : String(ref.source ?? '');
      const messages = findingsAt(catalog(), current.path, ref.line).map(finding => finding.message);
      row.textContent = [t('workshop.composition.script_ref', {
        kind: t(ref.kind === 'unload' ? 'workshop.composition.ref_unload' : 'workshop.composition.ref_load'),
        path: ref.path, line: String(ref.line ?? ''), source, origin: originText(ref.origin) }), ...messages].join(' ');
      refList.append(row);
    }
    if (!refList.children.length) refList.append(node('li', 'workshop.composition.no_script_refs'));
    refs.append(refList);
    worldFormNode.append(refs);
  }

  function renderMembers() {
    const records = catalog()?.members || [];
    membersList.replaceChildren(...records.map(record => {
      const row = node('li', null, { 'data-origin': record.origin });
      row.textContent = t('workshop.composition.member', { path: record.path, origin: originText(record.origin),
        allowed: t(record.allowed ? 'workshop.composition.allowed' : 'workshop.composition.not_allowed'),
        references: record.referenced_by?.length
          ? t('workshop.composition.referenced_by', { paths: record.referenced_by.join(', ') }) : t('workshop.composition.unreferenced') });
      return row;
    }));
    if (!records.length) membersList.append(node('li', 'workshop.composition.no_members'));
  }

  /** What Test and the lobby would list: `id — label — world — ships`. The
   * runtime's catalogue entry carries the origin the MERGED lobby catalogue
   * stamps on a pack's scenario, which a single manifest never sets, so it is
   * not shown here — the world's origin is on the root's own select. */
  function renderCatalogue() {
    const records = catalog()?.catalogue || [];
    catalogueList.replaceChildren(...records.map(record => {
      const row = node('li');
      row.textContent = t('workshop.composition.catalogue_entry', { id: record.id, label: record.label ?? t('workshop.composition.no_label'),
        world: record.world, ships: (record.ships || []).map(ship => ship.template_path).join(', ') || t('workshop.composition.no_ships') });
      return row;
    }));
    if (!records.length) catalogueList.append(node('li', 'workshop.composition.no_catalogue'));
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
    if (!records.length) findingsList.append(node('li', 'workshop.composition.no_findings'));
  }

  /** Read the draft through the runtime. An answer for a draft that moved while
   * it was being read is discarded rather than shown against newer source. A
   * form holding UNAPPLIED edits over a member that is byte-for-byte what the
   * previous reading held keeps them: applying the roots must not throw away
   * the extra world chosen beside them, nor the reverse. An untouched form is
   * rebuilt from the new reading like everything else. */
  async function reload({ announce = true } = {}) {
    const candidate = draft();
    if (!candidate) return;
    const snapshot = definitionsSnapshot(candidate);
    const result = await runtime.composition(snapshot.files);
    if (disposed || !snapshotIsCurrent(snapshot, draft())) return;
    if (!result || !Array.isArray(result.worlds) || !Array.isArray(result.members)
      || (result.manifest !== null && typeof result.manifest !== 'object')) throw new Error('workshop.inspector_refused');
    const dirty = (state, fresh) => Boolean(state) && Boolean(fresh) && JSON.stringify(state) !== JSON.stringify(fresh);
    const previousManifest = manifest(), previousWorld = selectedWorld();
    const pending = {
      roots: dirty(rootsState, previousManifest && rootsForm(previousManifest)) ? rootsState : null,
      manifestPath: previousManifest?.path, manifestSource: reading?.files?.[previousManifest?.path],
      world: dirty(worldState, previousWorld && worldForm(previousWorld)) ? worldState : null,
      worldPath: world.value, worldSource: reading?.files?.[world.value],
    };
    reading = { ...snapshot, catalog: result };
    renderAll();
    if (pending.roots && manifest()?.path === pending.manifestPath && snapshot.files[pending.manifestPath] === pending.manifestSource) {
      rootsState = pending.roots; renderRoots();
    }
    if (pending.world && world.value === pending.worldPath && snapshot.files[world.value] === pending.worldSource) {
      worldState = pending.world; renderWorldForm();
    }
    if (announce) show('workshop.composition.refreshed');
  }

  async function guarded(action) {
    setBusy(true);
    try { await action(); }
    catch (error) { if (!disposed) show(knownId(error), true, { detail: error?.detail ?? '' }); }
    finally { if (!disposed) { setBusy(false); refresh(); } }
  }

  /** ONE compose call for ONE member, then ONE draft edit. The draft is never
   * touched before the runtime answers; a refusal is shown by its category
   * with the runtime's own words as the detail, and an answer for a draft that
   * moved in the meantime is refused as stale rather than written over newer
   * source. */
  async function commit(read, path, edits) {
    const source = read.files[path];
    let result;
    try { result = await runtime.compose(read.files, { document_path: path, expected_source: source, edits }); }
    catch (error) {
      const message = refusalMessage(error);
      const refused = new Error(refusalStringId(message));
      refused.detail = message;
      throw refused;
    }
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
  applyRootsButton.addEventListener('click', () => {
    if (busy() || applyRootsButton.disabled || !fresh()) return;
    const read = reading, current = manifest();
    if (!current || !rootsState) return;
    let edits;
    try { edits = planScenarioEdits({ manifest: current, form: rootsState }); }
    catch (error) { show(knownId(error), true, { detail: error?.detail ?? '' }); return; }
    if (!edits.length) { show('workshop.composition.unchanged'); return; }
    void guarded(() => commit(read, current.path, edits));
  });
  addRootButton.addEventListener('click', () => {
    if (busy() || addRootButton.disabled || !rootsState) return;
    const id = addRootId.value.trim();
    if (!id) { show('workshop.composition.refused.empty', true, { detail: 'id' }); return; }
    if (!addRootWorld.value) { show('workshop.composition.refused.empty', true, { detail: 'world' }); return; }
    if (rootsState.some(entry => !entry.removed && entry.id.trim() === id)) {
      show('workshop.composition.refused.duplicate', true, { detail: id }); return;
    }
    rootsState.push(newRoot(id, addRootWorld.value));
    addRootId.value = '';
    renderRoots(); refresh();
    focusFirst(`root-${rootsState.length - 1}-id`);
  });
  applyWorldButton.addEventListener('click', () => {
    if (busy() || applyWorldButton.disabled || !fresh()) return;
    const read = reading, current = selectedWorld();
    if (!current || !worldState) return;
    let edits;
    try { edits = planExtraWorldEdits({ world: current, form: worldState }); }
    catch (error) { show(knownId(error), true, { detail: error?.detail ?? '' }); return; }
    if (!edits.length) { show('workshop.composition.unchanged'); return; }
    void guarded(() => commit(read, current.path, edits));
  });
  createWorldButton.addEventListener('click', () => {
    if (busy() || createWorldButton.disabled) return;
    const current = draft(), title = newWorldTitle.value.trim();
    let path;
    try { path = worldSlugPath(title); }
    catch (error) { show(knownId(error), true); return; }
    if (current.paths().includes(path)) { show('workshop.composition.world_exists', true, { detail: path }); return; }
    void guarded(async () => {
      const source = await runtime.newWorld(title);
      if (disposed) return;
      // The draft may have gained that member while the runtime was answering.
      if (draft() !== current || current.paths().includes(path)) {
        const error = new Error('workshop.composition.world_exists'); error.detail = path; throw error;
      }
      if (typeof source !== 'string') throw new Error('workshop.inspector_refused');
      current.put(path, source);
      newWorldTitle.value = '';
      preferredWorld = path;
      changed(path);
      show('workshop.changed');
      await reload({ announce: false }).catch(() => {});
    });
  });
  world.addEventListener('change', () => { resetWorldForm(); refresh(); });
  show('workshop.composition.empty');
  refresh();
  return { refresh, node: section, dispose() { disposed = true; section.remove(); } };
}
