import { definitionsSnapshot, snapshotIsCurrent, draftFirst } from '../editor/workshop-definitions.js';
import { presetsForm, planPresetEdits, movePreset, moveWidget, newWidget, presetMovable, widgetMovable,
  widgetOwns, widgetTypeChoices, widgetActionChoices, contactChoices, worldChoices, presetFindings, findingsOn,
  appendPresetBlock, refusalMessage, refusalStringId, RESERVED_PRESET_ID, DRAWN_PANEL_IDS,
  DRAWN_QUICK_ACTION_IDS } from '../editor/workshop-presets.js';
import { t } from './strings.js';

const PREFIX = 'workshop-presets';

/** Runtime-backed forms over GM role presets, panel assignments, quick actions
 * and typed mission widgets (issue #1477), shaped like the definitions,
 * composition and entity forms: a reading of the draft through the runtime, edits
 * planned in the pure module, and ONE edit call per Apply that the runtime either
 * lands as exact source or refuses — a reserved or duplicate id, an unknown
 * widget type, a key on a type that does not own it or a reference the world does
 * not have never reaches the draft. Dependency worlds are listed read-only with
 * their origin.
 *
 * Every per-widget control follows the widget's TYPE, so a key is never offered
 * on a type that does not own it. The panel ids and quick-action ids are the
 * BROWSER's vocabulary and come from gui/gm-role-presets.js; an authored value
 * this build does not draw is shown as such, can be removed, and carries a
 * warning in the same findings list as the runtime's own — never a refusal,
 * because Rust keeps those vocabularies open on purpose.
 *
 * What is authored here is presentation only (criterion 4): nothing reaches a
 * GmOperator, a GmAction, a snapshot or the sim digest, and the action ids a
 * widget may repeat are the buttons this build already draws. */
export function mountWorkshopPresets({ root, runtime, draft, busy, setBusy, changed, attach = true }) {
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
  const originText = origin => (origin ? String(origin) : t('workshop.presets.origin_missing'));
  const worldOption = entry => option(entry.path, entry.origin === 'draft' ? entry.path : `${entry.path} (${originText(entry.origin)})`);
  /** A finding carries either the runtime's own sentence or a string id this
   * panel localises — the browser-owned warnings are the second kind. */
  const findingText = record => record.message ?? t(record.string, record.params);

  // A dock panel carries its own header, so this is a plain grouping element.
  const section = node('section', null, { class: 'workshop-presets', id: PREFIX });
  const refreshButton = node('button', 'workshop.presets.refresh', { type: 'button', id: `${PREFIX}-refresh` });
  const status = node('p', null, { role: 'status', tabindex: '-1', id: `${PREFIX}-status` });
  const worldsSet = fieldset('workshop.presets.worlds');
  const world = node('select', null, { id: `${PREFIX}-world` });
  const originNode = node('div', null, { id: `${PREFIX}-origin`, class: 'workshop-presets-origin' });
  worldsSet.append(labelled('workshop.presets.world', world), world, originNode);
  const presetsSet = fieldset('workshop.presets.presets');
  const presetsList = node('div', null, { class: 'workshop-presets-form', id: `${PREFIX}-presets` });
  const addPresetSet = fieldset('workshop.presets.add_preset');
  const addPresetId = node('input', null, { type: 'text', maxlength: '64', spellcheck: 'false', id: `${PREFIX}-add-preset-id` });
  const addPresetLabel = node('input', null, { type: 'text', maxlength: '96', spellcheck: 'false', id: `${PREFIX}-add-preset-label` });
  const addPresetButton = node('button', 'workshop.presets.add', { type: 'button', id: `${PREFIX}-add-preset-button` });
  addPresetSet.append(labelled('workshop.presets.add_preset_id', addPresetId), addPresetId,
    labelled('workshop.presets.add_preset_label', addPresetLabel), addPresetLabel, addPresetButton);
  presetsSet.append(presetsList, addPresetSet);
  const selectedSet = fieldset('workshop.presets.selected');
  const presetSelect = node('select', null, { id: `${PREFIX}-preset` });
  const presetFormNode = node('div', null, { class: 'workshop-presets-form', id: `${PREFIX}-preset-form` });
  selectedSet.append(labelled('workshop.presets.select_preset', presetSelect), presetSelect, presetFormNode);
  const findingsSet = fieldset('workshop.presets.findings');
  const findingsList = node('ul', null, { id: `${PREFIX}-findings`, class: 'workshop-presets-findings' });
  findingsSet.append(findingsList);
  const applyButton = node('button', 'workshop.presets.apply', { type: 'button', id: `${PREFIX}-apply` });
  section.append(node('p', 'workshop.presets.hint'), node('p', 'workshop.presets.presentation_only'),
    refreshButton, status, worldsSet, presetsSet, selectedSet, findingsSet, applyButton);
  // A docked panel is placed by the renderer, which moves the node into its frame.
  // Attaching here as well would strand the node in `root` whenever the stored
  // layout has the panel closed, because a closed panel is never framed.
  if (attach) root.append(section);

  let disposed = false, reading = null, testHidden = false, formState = null;
  let previousDraft = null, previousPaths = '', preferredWorld = null, worldOptions = [];
  const show = (id, error = false, params = undefined) => {
    status.textContent = t(id, params);
    status.setAttribute('role', error ? 'alert' : 'status');
    if (error) status.focus();
  };
  /** A removal or a move rebuilds its form, which takes the pressed control out
   * of the document; focus would fall to the body and a keyboard user would Tab
   * from the top again. The first present, enabled candidate takes it instead —
   * and the status line, which is always focusable and carries what just
   * happened, when none of them is. */
  const focusFirst = (...ids) => {
    for (const id of ids) {
      const target = id && section.querySelector(`#${PREFIX}-${id}`);
      if (target && !target.disabled) { target.focus(); return; }
    }
    status.focus();
  };
  const fresh = () => Boolean(reading) && snapshotIsCurrent(reading, draft());
  const catalog = () => reading?.catalog;
  const findings = () => reading?.findings || [];
  const editable = () => fresh() && catalog()?.origin === 'draft' && Boolean(formState);
  const livePresets = () => (formState?.presets || []).filter(entry => !entry.removed);
  const selectedPreset = () => livePresets().find(entry => String(entry.index) === presetSelect.value) || null;

  function refresh({ hidden = testHidden } = {}) {
    testHidden = hidden;
    section.hidden = hidden;
    const current = draft(), paths = current?.paths().join('\n') || '';
    // The selector is offered before any reading exists: its whole job is to say
    // which world the first read is for.
    if (current !== previousDraft || paths !== previousPaths) {
      previousDraft = current; previousPaths = paths; renderWorlds(); renderOrigin();
    }
    const held = busy();
    const live = fresh();
    const open = editable();
    refreshButton.disabled = held || !current || !world.options.length || typeof runtime?.presets !== 'function';
    if (reading && !live) show('workshop.inspector_stale');
    if (!current) show('workshop.presets.empty');
    world.disabled = held || !current || !world.options.length;
    presetSelect.disabled = held || !live || !presetSelect.options.length;
    for (const control of presetsList.querySelectorAll('input, select, button')) {
      control.disabled = held || !open || control.dataset.readonly === 'true';
    }
    for (const control of presetFormNode.querySelectorAll('input, select, button')) {
      control.disabled = held || !open || control.dataset.readonly === 'true';
    }
    addPresetId.disabled = addPresetLabel.disabled = held || !open || typeof runtime?.newPreset !== 'function';
    addPresetButton.disabled = addPresetId.disabled;
    applyButton.disabled = held || !open;
  }

  function renderAll() {
    renderWorlds();
    renderOrigin();
    resetForm();
    renderFindings();
  }

  /** Every world the panel can read: the draft's own world members plus the ones
   * the runtime named. The runtime's list survives a cleared reading, so choosing
   * a dependency world does not remove it from the selector it was chosen in. */
  function renderWorlds() {
    const wanted = preferredWorld || world.value;
    preferredWorld = null;
    const paths = draft()?.paths() || [];
    const known = new Map(worldOptions.map(entry => [entry.path, entry]));
    for (const entry of worldChoices(paths, catalog())) known.set(entry.path, entry);
    // A member the draft no longer has stops being offered; a dependency world is
    // not in the draft to begin with, so it stays.
    worldOptions = [...known.values()].filter(entry => entry.origin !== 'draft' || paths.includes(entry.path));
    const entries = draftFirst(worldOptions);
    world.replaceChildren(...entries.map(worldOption));
    if (entries.some(entry => entry.path === wanted)) world.value = wanted;
  }

  function renderOrigin() {
    originNode.replaceChildren();
    const current = catalog();
    if (!current) {
      originNode.append(node('p', world.options.length ? 'workshop.presets.no_reading' : 'workshop.presets.no_worlds'));
      return;
    }
    const origin = node('p');
    origin.textContent = current.origin === 'draft'
      ? t('workshop.presets.world_draft', { path: current.path })
      : t('workshop.presets.read_only_origin', { origin: originText(current.origin) });
    originNode.append(origin);
  }

  function resetForm() {
    const current = catalog();
    formState = current ? presetsForm(current) : null;
    renderPresets();
  }

  /** The whole form for the selected world: the preset list, then the facets of
   * whichever preset the selector names. Rebuilt on every structural change. */
  function renderPresets() {
    presetsList.replaceChildren();
    const current = catalog();
    if (!current || !formState) { renderPresetSelector(); return; }
    const form = formState;
    const live = form.presets.map((entry, at) => (entry.removed ? null : at)).filter(at => at != null);
    form.presets.forEach((entry, position) => {
      if (entry.removed) return;
      const set = node('fieldset', null, { class: 'workshop-presets-preset', 'data-preset': String(entry.index) });
      const legend = node('legend'); set.append(legend);
      const up = node('button', 'workshop.presets.move_up', { type: 'button', id: `${PREFIX}-preset-${position}-up` });
      const down = node('button', 'workshop.presets.move_down', { type: 'button', id: `${PREFIX}-preset-${position}-down` });
      const remove = node('button', 'workshop.presets.remove', { type: 'button', id: `${PREFIX}-preset-${position}-remove` });
      // The legend and the buttons' accessible names carry the preset id and
      // FOLLOW it as it is typed: nothing re-renders on a keystroke, so a name
      // fixed at render time would announce "Remove: alpha" over a preset
      // already renamed to beta.
      const relabel = () => {
        legend.textContent = t('workshop.presets.preset', { id: entry.id });
        for (const [control, textId] of [[up, 'workshop.presets.move_up'], [down, 'workshop.presets.move_down'],
          [remove, 'workshop.presets.remove']]) control.setAttribute('aria-label', `${t(textId)}: ${entry.id}`);
      };
      relabel();
      const id = node('input', null, { type: 'text', spellcheck: 'false', id: `${PREFIX}-preset-${position}-id` });
      id.value = entry.id;
      id.addEventListener('input', () => {
        entry.id = id.value; relabel();
        // The selector names the presets by id and follows this one as it is
        // typed, without rebuilding the facet form beneath it: a rebuild would
        // discard the widget id half-typed beside it.
        const named = [...presetSelect.options].find(candidate => candidate.value === String(entry.index));
        if (named) named.textContent = entry.id || t('workshop.presets.unnamed', { index: String(entry.index) });
      });
      set.append(labelled('workshop.presets.preset_id', id), id);
      const label = node('input', null, { type: 'text', spellcheck: 'false', id: `${PREFIX}-preset-${position}-label` });
      label.value = entry.label;
      label.addEventListener('input', () => { entry.label = label.value; });
      set.append(labelled('workshop.presets.preset_label', label), label);
      for (const line of [entry.id_line, entry.label_line]) {
        for (const record of findingsOn(findings(), current.path, line)) {
          const note = node('p', null, { class: 'workshop-presets-finding' });
          note.textContent = findingText(record);
          set.append(note);
        }
      }
      const actions = node('div', null, { class: 'workshop-presets-row' });
      const rank = live.indexOf(position);
      // An end preset has nowhere to go in that direction, and a swap needs BOTH
      // presets to be carriable: one holding widgets or an unknown key is not, so
      // its neighbour's control is off too rather than being a control whose press
      // does nothing. Each stays in place, disabled, so the row keeps the same
      // shape for a keyboard user, and the row says WHY.
      const carriable = other => other != null && presetMovable(entry) && presetMovable(form.presets[other]);
      if (rank <= 0 || !carriable(live[rank - 1])) up.dataset.readonly = 'true';
      if (rank < 0 || rank === live.length - 1 || !carriable(live[rank + 1])) down.dataset.readonly = 'true';
      // A row whose OWN content cannot be carried says so; a row that can be
      // carried but whose neighbour cannot says THAT, on its own row. The reason
      // living only on the neighbour's row is a disabled control with no stated
      // cause — state without a word, which is the colour-only problem again.
      const blocked = other => other != null && !presetMovable(form.presets[other]);
      if (!presetMovable(entry)) set.append(node('p', 'workshop.presets.immovable'));
      else if (blocked(live[rank - 1]) || blocked(live[rank + 1])) {
        set.append(node('p', 'workshop.presets.immovable_neighbour'));
      }
      const move = direction => () => {
        const target = movePreset(form, position, direction);
        if (target == null) return;
        renderPresets(); refresh();
        const kind = direction < 0 ? 'up' : 'down', other = direction < 0 ? 'down' : 'up';
        focusFirst(`preset-${target}-${kind}`, `preset-${target}-${other}`, `preset-${target}-id`);
      };
      up.addEventListener('click', move(-1));
      down.addEventListener('click', move(1));
      remove.addEventListener('click', () => {
        entry.removed = true;
        renderPresets(); refresh();
        // The next preset left, else the one before, else the add control.
        const left = form.presets.map((candidate, at) => (candidate.removed ? null : at)).filter(at => at != null);
        const nearest = left.find(at => at >= position) ?? left.filter(at => at < position).pop();
        focusFirst(nearest == null ? null : `preset-${nearest}-id`, 'add-preset-id');
      });
      actions.append(up, down, remove);
      set.append(actions);
      if (entry.unknown_keys.length) {
        const unknown = node('p');
        unknown.textContent = t('workshop.presets.unknown_keys', { keys: entry.unknown_keys.join(', ') });
        set.append(unknown);
      }
      presetsList.append(set);
    });
    if (!live.length) presetsList.append(node('p', 'workshop.presets.no_presets'));
    renderPresetSelector();
  }

  function renderPresetSelector() {
    const wanted = presetSelect.value;
    const entries = livePresets();
    presetSelect.replaceChildren(...entries.map(entry => option(String(entry.index),
      entry.id || t('workshop.presets.unnamed', { index: String(entry.index) }))));
    if (entries.some(entry => String(entry.index) === wanted)) presetSelect.value = wanted;
    renderPresetForm();
  }

  function renderPresetForm() {
    presetFormNode.replaceChildren();
    const current = catalog(), preset = selectedPreset();
    if (!current || !preset) { presetFormNode.append(node('p', 'workshop.presets.no_selection')); return; }
    presetFormNode.append(renderFacet('panels', preset, DRAWN_PANEL_IDS, 'workshop.presets.panels', 'panel'));
    presetFormNode.append(renderFacet('quick_actions', preset, DRAWN_QUICK_ACTION_IDS,
      'workshop.presets.quick_actions', 'action'));
    presetFormNode.append(renderContacts(current, preset));
    presetFormNode.append(renderWidgets(current, preset));
  }

  /** One checkbox list over a BROWSER-owned vocabulary, plus a row for every
   * authored value this build does not draw — kept, removable, and saying so,
   * because that vocabulary is open on purpose and an authored id nobody draws
   * yet is not an error. */
  function renderFacet(key, preset, drawn, legendId, prefix) {
    const set = fieldset(legendId);
    const list = node('ul', null, { id: `${PREFIX}-${prefix}s` });
    drawn.forEach((value, index) => {
      const row = node('li');
      const box = node('input', null, { type: 'checkbox', id: `${PREFIX}-${prefix}-${index}` });
      box.checked = preset[key].includes(value);
      box.addEventListener('change', () => {
        preset[key] = box.checked ? [...preset[key].filter(entry => entry !== value), value]
          : preset[key].filter(entry => entry !== value);
      });
      row.append(box, labelled(null, box, value));
      list.append(row);
    });
    preset[key].filter(value => !drawn.includes(value)).forEach((value, index) => {
      const row = node('li', null, { class: 'workshop-presets-undrawn' });
      const box = node('input', null, { type: 'checkbox', id: `${PREFIX}-${prefix}-extra-${index}` });
      box.checked = true;
      box.addEventListener('change', () => {
        preset[key] = preset[key].filter(entry => entry !== value);
        renderPresetForm(); refresh();
        focusFirst(`${prefix}-extra-${index}`, `${prefix}-extra-${index - 1}`, `${prefix}-0`);
      });
      row.append(box, labelled(null, box, `${value} — ${t('workshop.presets.not_drawn')}`));
      list.append(row);
    });
    set.append(list);
    return set;
  }

  function renderContacts(current, preset) {
    const set = fieldset('workshop.presets.contacts');
    const list = node('ul', null, { id: `${PREFIX}-contacts` });
    const read = (current.presets || []).find(entry => entry.index === preset.index);
    preset.contacts.forEach((name, index) => {
      const row = node('li');
      const entry = (read?.contacts || []).find(candidate => candidate.value === name);
      const messages = findingsOn(findings(), current.path, entry?.line).map(findingText);
      const label = node('span');
      // The runtime's own judgement of the reference where it has one (contract
      // A's `known`), and the world's entity names for a contact the form has
      // only just added.
      const known = entry ? entry.known !== false : (current.choices?.entities || []).includes(name);
      label.textContent = [known ? name : `${name} — ${t('workshop.presets.unknown_contact')}`, ...messages].join(' ');
      const remove = node('button', 'workshop.presets.remove', { type: 'button', id: `${PREFIX}-contact-${index}-remove`,
        'aria-label': `${t('workshop.presets.remove')}: ${name}` });
      remove.addEventListener('click', () => {
        preset.contacts.splice(index, 1);
        renderPresetForm(); refresh();
        focusFirst(`contact-${index}-remove`, `contact-${index - 1}-remove`, 'add-contact');
      });
      row.append(label, remove);
      list.append(row);
    });
    if (!preset.contacts.length) list.append(node('li', 'workshop.presets.no_contacts'));
    const addSelect = node('select', null, { id: `${PREFIX}-add-contact` });
    const choices = contactChoices(current, preset.contacts);
    addSelect.replaceChildren(...choices.map(name => option(name, name)));
    const addButton = node('button', 'workshop.presets.add', { type: 'button', id: `${PREFIX}-add-contact-button` });
    addButton.addEventListener('click', () => {
      if (!addSelect.value) return;
      preset.contacts.push(addSelect.value);
      renderPresetForm(); refresh();
      presetFormNode.querySelector(`#${PREFIX}-add-contact`)?.focus();
    });
    const addRow = node('div', null, { class: 'workshop-presets-row' });
    addRow.append(labelled('workshop.presets.add_contact', addSelect), addSelect, addButton);
    if (!choices.length) addRow.append(node('span', 'workshop.presets.no_entities'));
    set.append(list, addRow);
    return set;
  }

  /** One group per widget, whose controls follow the widget's TYPE: band and
   * category for attention, ship for attention and workload, action checkboxes
   * for actions, text for note. A key the type does not own is not rendered at
   * all, so it cannot be authored onto a card that has no place for it. */
  function renderWidgets(current, preset) {
    const set = fieldset('workshop.presets.widgets');
    const list = node('div', null, { id: `${PREFIX}-widgets`, class: 'workshop-presets-form' });
    const read = (current.presets || []).find(entry => entry.index === preset.index);
    const bands = current.choices?.bands || [];
    const categories = current.choices?.categories || [];
    const entities = current.choices?.entities || [];
    const actionIds = widgetActionChoices(current);
    preset.widgets.forEach((widget, index) => {
      const group = node('fieldset', null, { class: 'workshop-presets-widget', 'data-kind': widget.kind });
      const legend = node('legend'); group.append(legend);
      const up = node('button', 'workshop.presets.move_up', { type: 'button', id: `${PREFIX}-widget-${index}-up` });
      const down = node('button', 'workshop.presets.move_down', { type: 'button', id: `${PREFIX}-widget-${index}-down` });
      const remove = node('button', 'workshop.presets.remove', { type: 'button', id: `${PREFIX}-widget-${index}-remove` });
      const relabel = () => {
        legend.textContent = t('workshop.presets.widget', { id: widget.id, type: widget.kind });
        for (const [control, textId] of [[up, 'workshop.presets.move_up'], [down, 'workshop.presets.move_down'],
          [remove, 'workshop.presets.remove']]) control.setAttribute('aria-label', `${t(textId)}: ${widget.id}`);
      };
      relabel();
      const id = node('input', null, { type: 'text', spellcheck: 'false', id: `${PREFIX}-widget-${index}-id` });
      id.value = widget.id;
      id.addEventListener('input', () => { widget.id = id.value; relabel(); });
      group.append(labelled('workshop.presets.widget_id', id), id);
      const label = node('input', null, { type: 'text', spellcheck: 'false', id: `${PREFIX}-widget-${index}-label` });
      label.value = widget.label;
      label.addEventListener('input', () => { widget.label = label.value; });
      group.append(labelled('workshop.presets.widget_label', label), label);
      // The type is editable per widget and not only at Add. A card authored as
      // the wrong surface would otherwise be a remove-and-add — two history
      // entries and a new id — while the planner already rewrites the type and
      // drops the old type's keys as ONE group. Changing it re-renders the row,
      // because every control beneath follows the type.
      const kinds = widgetTypeChoices(current);
      const kindSelect = node('select', null, { id: `${PREFIX}-widget-${index}-type` });
      kindSelect.replaceChildren(
        ...(kinds.includes(widget.kind) ? [] : [option(widget.kind,
          widget.kind ? `${widget.kind} — ${t('workshop.presets.not_drawn')}` : t('workshop.presets.none'))]),
        ...kinds.map(kind => option(kind, kind)));
      kindSelect.value = widget.kind;
      kindSelect.addEventListener('change', () => {
        widget.kind = kindSelect.value;
        renderPresetForm(); refresh();
        focusFirst(`widget-${index}-type`, `widget-${index}-id`);
      });
      group.append(labelled('workshop.presets.widget_type', kindSelect), kindSelect);
      const readWidget = (read?.widgets || []).find(entry => entry.index === widget.index);
      for (const line of [readWidget?.id_line, readWidget?.kind_line, readWidget?.label_line]) {
        for (const record of findingsOn(findings(), current.path, line)) {
          const note = node('p', null, { class: 'workshop-presets-finding' });
          note.textContent = findingText(record);
          group.append(note);
        }
      }
      const choice = (key, values, textId) => {
        const select = node('select', null, { id: `${PREFIX}-widget-${index}-${key}` });
        select.replaceChildren(option('', t('workshop.presets.none')),
          ...(values.includes(widget[key]) || !widget[key] ? [] : [option(widget[key],
            `${widget[key]} (${t('workshop.presets.origin_missing')})`)]),
          ...values.map(value => option(value, value)));
        select.value = widget[key];
        select.addEventListener('change', () => { widget[key] = select.value; });
        group.append(labelled(textId, select), select);
      };
      if (widgetOwns(widget.kind, 'band')) choice('band', bands, 'workshop.presets.widget_band');
      if (widgetOwns(widget.kind, 'category')) choice('category', categories, 'workshop.presets.widget_category');
      if (widgetOwns(widget.kind, 'ship')) choice('ship', entities, 'workshop.presets.widget_ship');
      if (widgetOwns(widget.kind, 'actions')) {
        const actionsSet = fieldset('workshop.presets.widget_actions');
        const rows = node('ul');
        actionIds.forEach((value, at) => {
          const row = node('li');
          const box = node('input', null, { type: 'checkbox', id: `${PREFIX}-widget-${index}-action-${at}` });
          box.checked = widget.actions.includes(value);
          box.addEventListener('change', () => {
            widget.actions = box.checked ? [...widget.actions.filter(entry => entry !== value), value]
              : widget.actions.filter(entry => entry !== value);
          });
          row.append(box, labelled(null, box, value));
          rows.append(row);
        });
        widget.actions.filter(value => !actionIds.includes(value)).forEach((value, at) => {
          const row = node('li', null, { class: 'workshop-presets-undrawn' });
          const box = node('input', null, { type: 'checkbox', id: `${PREFIX}-widget-${index}-action-extra-${at}` });
          box.checked = true;
          box.addEventListener('change', () => {
            widget.actions = widget.actions.filter(entry => entry !== value);
            renderPresetForm(); refresh();
            focusFirst(`widget-${index}-action-extra-${at}`, `widget-${index}-action-0`, `widget-${index}-id`);
          });
          row.append(box, labelled(null, box, `${value} — ${t('workshop.presets.not_drawn')}`));
          rows.append(row);
        });
        if (!rows.children.length) rows.append(node('li', 'workshop.presets.no_actions'));
        actionsSet.append(rows);
        group.append(actionsSet);
      }
      if (widgetOwns(widget.kind, 'text')) {
        const text = node('input', null, { type: 'text', spellcheck: 'false', id: `${PREFIX}-widget-${index}-text` });
        text.value = widget.text;
        text.addEventListener('input', () => { widget.text = text.value; });
        group.append(labelled('workshop.presets.widget_text', text), text);
      }
      if (!kinds.includes(widget.kind)) group.append(node('p', 'workshop.presets.widget_unknown_type'));
      // The same rule one level down: a swap needs both widgets to be carriable.
      const carriable = other => Boolean(other) && widgetMovable(widget) && widgetMovable(other);
      if (index === 0 || !carriable(preset.widgets[index - 1])) up.dataset.readonly = 'true';
      if (index === preset.widgets.length - 1 || !carriable(preset.widgets[index + 1])) down.dataset.readonly = 'true';
      const blocked = other => Boolean(other) && !widgetMovable(other);
      if (!widgetMovable(widget)) group.append(node('p', 'workshop.presets.immovable_widget'));
      else if (blocked(preset.widgets[index - 1]) || blocked(preset.widgets[index + 1])) {
        group.append(node('p', 'workshop.presets.immovable_neighbour_widget'));
      }
      const move = direction => () => {
        const target = moveWidget(preset.widgets, index, direction);
        if (target == null) return;
        renderPresetForm(); refresh();
        const kind = direction < 0 ? 'up' : 'down', other = direction < 0 ? 'down' : 'up';
        focusFirst(`widget-${target}-${kind}`, `widget-${target}-${other}`, `widget-${target}-id`);
      };
      up.addEventListener('click', move(-1));
      down.addEventListener('click', move(1));
      remove.addEventListener('click', () => {
        preset.widgets.splice(index, 1);
        renderPresetForm(); refresh();
        focusFirst(`widget-${index}-remove`, `widget-${index - 1}-remove`, 'add-widget-id');
      });
      const row = node('div', null, { class: 'workshop-presets-row' });
      row.append(up, down, remove);
      group.append(row);
      if (widget.unknown_keys.length) {
        const unknown = node('p');
        unknown.textContent = t('workshop.presets.unknown_keys', { keys: widget.unknown_keys.join(', ') });
        group.append(unknown);
      }
      list.append(group);
    });
    if (!preset.widgets.length) list.append(node('p', 'workshop.presets.no_widgets'));
    const addSet = fieldset('workshop.presets.add_widget');
    const addId = node('input', null, { type: 'text', maxlength: '64', spellcheck: 'false', id: `${PREFIX}-add-widget-id` });
    const addType = node('select', null, { id: `${PREFIX}-add-widget-type` });
    addType.replaceChildren(...widgetTypeChoices(current).map(kind => option(kind, kind)));
    const addButton = node('button', 'workshop.presets.add', { type: 'button', id: `${PREFIX}-add-widget-button` });
    addButton.addEventListener('click', () => {
      const value = addId.value.trim();
      if (!value) { show('workshop.presets.refused.widget_empty', true, { detail: '' }); return; }
      if (!addType.value) { show('workshop.presets.refused.type', true, { detail: '' }); return; }
      if (preset.widgets.some(widget => widget.id.trim() === value)) {
        show('workshop.presets.refused.widget_duplicate', true, { detail: value }); return;
      }
      preset.widgets.push(newWidget(value, addType.value));
      addId.value = '';
      renderPresetForm(); refresh();
      focusFirst(`widget-${preset.widgets.length - 1}-id`, 'add-widget-id');
    });
    addSet.append(labelled('workshop.presets.add_widget_id', addId), addId,
      labelled('workshop.presets.widget_type', addType), addType, addButton);
    set.append(list, addSet);
    return set;
  }

  function renderFindings() {
    const records = findings();
    findingsList.replaceChildren(...records.map(record => {
      const row = node('li', null, { 'data-severity': record.severity, 'data-category': record.category ?? '' });
      const location = `${record.file}${record.line ? `:${record.line}` : ''}`;
      // The severity is a word, never a colour alone.
      row.textContent = `${location} — ${t(`workshop.severity.${record.severity}`)}: ${findingText(record)}`;
      return row;
    }));
    if (!records.length) findingsList.append(node('li', 'workshop.presets.no_findings'));
  }

  /** Read the selected world through the runtime. An answer for a draft that
   * moved while it was being read is discarded rather than shown against newer
   * source. A form holding UNAPPLIED edits over a member that is byte-for-byte
   * what the previous reading held keeps them; an untouched form is rebuilt from
   * the new reading like everything else. */
  async function reload({ announce = true } = {}) {
    const candidate = draft();
    const path = world.value;
    if (!candidate || !path) return;
    const snapshot = definitionsSnapshot(candidate);
    const result = await runtime.presets(snapshot.files, path);
    if (disposed || !snapshotIsCurrent(snapshot, draft())) return;
    if (!result || typeof result !== 'object' || typeof result.path !== 'string' || !Array.isArray(result.presets)
      || !Array.isArray(result.worlds) || !result.choices || typeof result.choices !== 'object') {
      throw new Error('workshop.inspector_refused');
    }
    const previous = catalog();
    const rebuilt = previous && presetsForm(previous);
    const dirty = Boolean(formState) && Boolean(rebuilt) && JSON.stringify(formState) !== JSON.stringify(rebuilt);
    const pending = { form: dirty ? formState : null, path: previous?.path, source: reading?.files?.[previous?.path] };
    reading = { ...snapshot, catalog: result, findings: presetFindings(result) };
    renderAll();
    if (pending.form && result.path === pending.path && snapshot.files[pending.path] === pending.source) {
      formState = pending.form; renderPresets();
    }
    if (announce) show('workshop.presets.refreshed');
  }

  /** The hold goes up for the whole of one runtime call and comes down once its
   * answer has been rendered. Focus moves AFTER that: every control is disabled
   * while the hold is up, so a landing spot chosen inside it could only ever be
   * one the rebuild happened to recreate. */
  async function guarded(action) {
    setBusy(true);
    let focus = null;
    try { focus = await action(); }
    catch (error) { if (!disposed) show(knownId(error), true, { detail: error?.detail ?? '' }); }
    finally {
      if (!disposed) {
        setBusy(false); refresh();
        if (focus?.length) focusFirst(...focus);
      }
    }
  }

  /** The runtime's own refusal, mapped to the sentence for its rule and carrying
   * its words as the detail. */
  function refused(error) {
    const message = refusalMessage(error);
    const value = new Error(refusalStringId(message));
    value.detail = message;
    return value;
  }

  /** What a landed answer does to the draft: ONE edit, so one undo reverts it,
   * then a re-read so the forms show what was written. An answer for a draft that
   * moved in the meantime is refused as stale rather than written over newer
   * source. */
  async function land(read, path, result, focus) {
    if (disposed) return null;
    if (!snapshotIsCurrent(read, draft())) throw new Error('workshop.inspector_stale');
    if (typeof result !== 'string') throw new Error('workshop.inspector_refused');
    const current = draft();
    if (current.edit(path, result)) changed(path);
    show('workshop.changed');
    // A failed re-read leaves the reading honestly stale rather than reporting
    // the landed edit as refused.
    await reload({ announce: false }).catch(() => {});
    return focus;
  }

  /** ONE edit call for ONE member, then ONE draft edit. The draft is never
   * touched before the runtime answers; a refusal is shown by its category with
   * the runtime's own words as the detail. */
  async function commit(read, path, edits, focus = []) {
    let result;
    try { result = await runtime.editPresets(read.files, { document_path: path, expected_source: read.files[path], edits }); }
    catch (error) { throw refused(error); }
    return land(read, path, result, focus);
  }

  refreshButton.addEventListener('click', () => {
    if (busy() || refreshButton.disabled) return;
    void guarded(() => reload());
  });
  applyButton.addEventListener('click', () => {
    if (busy() || applyButton.disabled || !fresh()) return;
    const read = reading, current = catalog();
    if (!current || !formState) return;
    let edits;
    try { edits = planPresetEdits({ catalog: current, form: formState }); }
    catch (error) { show(knownId(error), true, { detail: error?.detail ?? '' }); return; }
    if (!edits.length) { show('workshop.presets.unchanged'); return; }
    void guarded(() => commit(read, current.path, edits, ['apply', 'refresh']));
  });
  /** A new preset is the RUNTIME's own `[[gm_role_preset]]` block, joined to the
   * world it is authored in as one history entry — the same shape a new faction
   * and a new world take. The reserved id and a duplicate are refused in the form
   * before the runtime is asked, because both are refusals an author can see the
   * answer to here. */
  addPresetButton.addEventListener('click', () => {
    if (busy() || addPresetButton.disabled || !fresh()) return;
    const read = reading, current = catalog();
    if (!current || !formState) return;
    const id = addPresetId.value.trim(), label = addPresetLabel.value.trim();
    if (!id) { show('workshop.presets.refused.empty', true, { detail: '' }); return; }
    if (id === RESERVED_PRESET_ID) { show('workshop.presets.refused.reserved', true, { detail: id }); return; }
    if (!label) { show('workshop.presets.refused.label', true, { detail: id }); return; }
    // The block is appended to the SOURCE the form is reading, so a duplicate is
    // judged against the READING — the ids `parse_world` will see — and never
    // against form state, which hides a preset pending Remove and shows a rename
    // nobody has applied. Either would write a second `id = "watch"` into the
    // draft through a check that said it could not happen.
    if ((current.presets || []).some(entry => String(entry.id ?? '').trim() === id)) {
      show('workshop.presets.refused.duplicate', true, { detail: id }); return;
    }
    // Appending changes the very member the forms below are editing, so the
    // re-read that follows rebuilds them from the new source. Refusing here is
    // the honest version of that: an unapplied label or a pending Remove is said
    // to be in the way rather than silently dropped.
    if (JSON.stringify(formState) !== JSON.stringify(presetsForm(current))) {
      show('workshop.presets.add_unapplied', true, { detail: id }); return;
    }
    void guarded(async () => {
      let block;
      try { block = await runtime.newPreset(id, label); }
      catch (error) { throw refused(error); }
      if (disposed) return null;
      if (typeof block !== 'string') throw new Error('workshop.inspector_refused');
      addPresetId.value = ''; addPresetLabel.value = '';
      return land(read, current.path, appendPresetBlock(read.files[current.path], block),
        ['add-preset-id', 'apply', 'refresh']);
    });
  });
  /** A reading belongs to ONE world, so choosing another has nothing to show
   * until the runtime answers for it. The selector's own options survive that,
   * and the unapplied form of the world being left does not: it described a
   * different document. */
  world.addEventListener('change', () => {
    reading = null; formState = null;
    renderAll(); refresh();
    if (busy() || !draft() || typeof runtime?.presets !== 'function') { show('workshop.presets.empty'); return; }
    void guarded(() => reload());
  });
  presetSelect.addEventListener('change', () => { renderPresetForm(); refresh(); });
  // Rendered once before anything is read, so the panel says what it is waiting
  // for rather than showing empty sections with no explanation.
  renderAll();
  show('workshop.presets.empty');
  refresh();
  return { refresh, node: section, dispose() { disposed = true; section.remove(); } };
}
