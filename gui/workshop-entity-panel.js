import { definitionsSnapshot, snapshotIsCurrent, findingsAt, draftFirst } from '../editor/workshop-definitions.js';
import { includesForm, newInclude, moveInclude, includeChoices, planIncludeEdits, componentSkeleton, componentAddable,
  planComponentAdd, planComponentRemove, fieldGroups, fieldsForm, fieldIsEditable, fieldIsMaterialisable, fieldIsScalar,
  planFieldEdits, templateChoices, refusalMessage, refusalStringId } from '../editor/workshop-entity.js';
import { t } from './strings.js';

const PREFIX = 'workshop-entity';

/** Runtime-backed forms over entity template and fragment composition (issue
 * #1476), shaped like the definitions and composition forms: a reading of the
 * draft through the runtime, edits planned in the pure module, and ONE runtime
 * call per press that the runtime either lands as exact source or refuses — a
 * missing, cyclic, self or disallowed include, an unsupported component or a
 * template that would stop parsing never reaches the draft. Inherited fields are
 * shown with their owner and are read-only until they are deliberately
 * materialised, which is the only place a runtime value is written into source.
 * Preview and Test reuse the shared preview and the existing disposable Test
 * rather than a renderer of their own. The selections are local presentation,
 * never simulation inputs. */
export function mountWorkshopEntity({ root, runtime, draft, busy, setBusy, changed, win, attach = true,
  preview = null, test = null }) {
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
  const originText = origin => (origin ? String(origin) : t('workshop.entity.origin_missing'));
  const templateOption = entry => option(entry.path, entry.origin === 'draft' ? entry.path : `${entry.path} (${originText(entry.origin)})`);

  // A dock panel carries its own header, so this is a plain grouping element.
  const section = node('section', null, { class: 'workshop-entity', id: PREFIX });
  const refreshButton = node('button', 'workshop.entity.refresh', { type: 'button', id: `${PREFIX}-refresh` });
  const status = node('p', null, { role: 'status', tabindex: '-1', id: `${PREFIX}-status` });
  const templateSet = fieldset('workshop.entity.templates');
  const template = node('select', null, { id: `${PREFIX}-template` });
  const originNode = node('div', null, { id: `${PREFIX}-origin`, class: 'workshop-entity-origin' });
  templateSet.append(labelled('workshop.entity.template', template), template, originNode);
  const includesSet = fieldset('workshop.entity.includes');
  const includesList = node('ul', null, { id: `${PREFIX}-includes` });
  const addInclude = node('select', null, { id: `${PREFIX}-add-include` });
  const addIncludeButton = node('button', 'workshop.entity.add', { type: 'button', id: `${PREFIX}-add-include-button` });
  const addIncludeRow = node('div', null, { class: 'workshop-entity-row' });
  addIncludeRow.append(labelled('workshop.entity.add_include', addInclude), addInclude, addIncludeButton);
  const applyIncludes = node('button', 'workshop.entity.apply', { type: 'button', id: `${PREFIX}-apply-includes` });
  includesSet.append(includesList, addIncludeRow, applyIncludes);
  const componentsSet = fieldset('workshop.entity.components');
  const componentsList = node('ul', null, { id: `${PREFIX}-components` });
  componentsSet.append(componentsList);
  const fieldsSet = fieldset('workshop.entity.fields');
  const fieldsNode = node('div', null, { class: 'workshop-entity-form', id: `${PREFIX}-fields` });
  const applyFields = node('button', 'workshop.entity.apply', { type: 'button', id: `${PREFIX}-apply-fields` });
  fieldsSet.append(fieldsNode, applyFields);
  const exerciseSet = fieldset('workshop.entity.exercise');
  const previewButton = node('button', 'workshop.entity.preview', { type: 'button', id: `${PREFIX}-preview` });
  const testButton = node('button', 'workshop.entity.test', { type: 'button', id: `${PREFIX}-test` });
  const exerciseRow = node('div', null, { class: 'workshop-entity-row' });
  exerciseRow.append(previewButton, testButton);
  exerciseSet.append(node('p', 'workshop.entity.exercise_hint'), exerciseRow);
  const findingsSet = fieldset('workshop.entity.findings');
  const findingsList = node('ul', null, { id: `${PREFIX}-findings`, class: 'workshop-entity-findings' });
  findingsSet.append(findingsList);
  section.append(node('p', 'workshop.entity.hint'), refreshButton, status, templateSet, includesSet, componentsSet,
    fieldsSet, exerciseSet, findingsSet);
  // A docked panel is placed by the renderer, which moves the node into its frame.
  // Attaching here as well would strand the node in `root` whenever the stored
  // layout has the panel closed, because a closed panel is never framed.
  if (attach) root.append(section);

  let disposed = false, reading = null, testHidden = false, includesState = null, fieldsState = null;
  let previousDraft = null, previousPaths = '', preferredTemplate = null, templateOptions = [];
  const show = (id, error = false, params = undefined) => {
    status.textContent = t(id, params);
    status.setAttribute('role', error ? 'alert' : 'status');
    if (error) status.focus();
  };
  /** A removal or a move rebuilds its form, which takes the pressed control out
   * of the document; focus would fall to the body and a keyboard user would Tab
   * from the top again. The first present, enabled candidate takes it instead —
   * and the status line, which is always focusable and carries what just
   * happened, when NONE of them is: a component with no runtime skeleton offers
   * no Add, so after its Remove there is no control at that place at all. */
  const focusFirst = (...ids) => {
    for (const id of ids) {
      const target = id && section.querySelector(`#${PREFIX}-${id}`);
      if (target && !target.disabled) { target.focus(); return; }
    }
    status.focus();
  };
  const fresh = () => Boolean(reading) && snapshotIsCurrent(reading, draft());
  const composition = () => reading?.composition;
  const editable = () => fresh() && composition()?.origin === 'draft';

  function refresh({ hidden = testHidden } = {}) {
    testHidden = hidden;
    section.hidden = hidden;
    const current = draft(), paths = current?.paths().join('\n') || '';
    // The selector is offered before any reading exists: its whole job is to say
    // which template the first read is for.
    if (current !== previousDraft || paths !== previousPaths) {
      previousDraft = current; previousPaths = paths; renderTemplates(); renderOrigin();
    }
    const held = busy();
    const live = fresh();
    const open = editable();
    refreshButton.disabled = held || !current || !template.options.length || typeof runtime?.entity !== 'function';
    if (reading && !live) show('workshop.inspector_stale');
    if (!current) show('workshop.entity.empty');
    template.disabled = held || !current || !template.options.length;
    for (const control of includesList.querySelectorAll('input, select, button')) {
      control.disabled = held || !open || control.dataset.readonly === 'true';
    }
    // The select stays reachable when it has nothing left to offer, so the focus
    // a press hands back to it does not fall to the body; the button is what goes.
    addInclude.disabled = held || !open;
    addIncludeButton.disabled = held || !open || !addInclude.options.length;
    applyIncludes.disabled = held || !open || !includesState;
    for (const control of componentsList.querySelectorAll('button')) {
      control.disabled = held || !open || control.dataset.readonly === 'true';
    }
    for (const control of fieldsNode.querySelectorAll('input, button')) {
      control.disabled = held || !open || control.dataset.readonly === 'true';
    }
    applyFields.disabled = held || !open || !fieldsState;
    // Previewing and exercising READ the draft, so a dependency template is
    // offered too; only editing is the draft's own.
    previewButton.disabled = held || !live || !composition() || typeof preview !== 'function';
    testButton.disabled = held || !live || !composition() || typeof test !== 'function';
  }

  function renderAll() {
    renderTemplates();
    renderOrigin();
    resetIncludes();
    renderComponents();
    resetFields();
    renderFindings();
  }

  /** Every template the panel can read: the draft's own entity members plus the
   * ones the runtime named. The runtime's list survives a cleared reading, so
   * choosing a dependency template does not remove it from the selector it was
   * chosen in. */
  function renderTemplates() {
    const wanted = preferredTemplate || template.value;
    preferredTemplate = null;
    const paths = draft()?.paths() || [];
    const known = new Map(templateOptions.map(entry => [entry.path, entry]));
    for (const entry of templateChoices(paths, composition())) known.set(entry.path, entry);
    // A member the draft no longer has stops being offered; a dependency
    // template is not in the draft to begin with, so it stays.
    templateOptions = [...known.values()].filter(entry => entry.origin !== 'draft' || paths.includes(entry.path));
    const entries = draftFirst(templateOptions);
    template.replaceChildren(...entries.map(templateOption));
    if (entries.some(entry => entry.path === wanted)) template.value = wanted;
  }

  function renderOrigin() {
    originNode.replaceChildren();
    const current = composition();
    // A draft with no entity template at all cannot be told to choose one.
    if (!current) {
      originNode.append(node('p', template.options.length ? 'workshop.entity.no_reading' : 'workshop.entity.no_templates'));
      return;
    }
    const origin = node('p');
    origin.textContent = current.origin === 'draft'
      ? t('workshop.entity.template_draft', { path: current.path })
      : t('workshop.entity.read_only_origin', { origin: originText(current.origin) });
    originNode.append(origin);
    if (current.resolvable === false) {
      const unresolvable = node('p', null, { class: 'workshop-entity-finding' });
      unresolvable.textContent = t('workshop.entity.unresolvable', { detail: current.error ?? '' });
      originNode.append(unresolvable);
    }
    const sources = node('p');
    sources.textContent = (current.sources || []).length
      ? t('workshop.entity.sources', { paths: current.sources.join(', ') })
      : t('workshop.entity.no_sources');
    originNode.append(sources);
  }

  function resetIncludes() {
    const current = composition();
    includesState = current ? includesForm(current) : null;
    renderIncludes();
  }

  function renderIncludes() {
    includesList.replaceChildren();
    const current = composition();
    // Rebuilt on every include change; the fragment chosen for the next one survives it.
    const wantedAdd = addInclude.value;
    const choices = current ? draftFirst(includeChoices(current, includesState?.includes.map(entry => entry.canonical) || [])) : [];
    addInclude.replaceChildren(...choices.map(templateOption));
    if (choices.some(entry => entry.path === wantedAdd)) addInclude.value = wantedAdd;
    if (!current || !includesState) return;
    const form = includesState;
    form.includes.forEach((entry, index) => {
      const row = node('li', null, { 'data-origin': entry.origin ?? '' });
      const label = node('span');
      const messages = findingsAt(current, current.path, entry.line).map(finding => finding.message);
      label.textContent = [t('workshop.entity.include', { authored: entry.authored,
        path: entry.canonical ?? entry.authored, origin: originText(entry.origin) }), ...messages].join(' ');
      const up = node('button', 'workshop.entity.move_up', { type: 'button', id: `${PREFIX}-include-${index}-up`,
        'aria-label': `${t('workshop.entity.move_up')}: ${entry.authored}` });
      const down = node('button', 'workshop.entity.move_down', { type: 'button', id: `${PREFIX}-include-${index}-down`,
        'aria-label': `${t('workshop.entity.move_down')}: ${entry.authored}` });
      const remove = node('button', 'workshop.entity.remove', { type: 'button', id: `${PREFIX}-include-${index}-remove`,
        'aria-label': `${t('workshop.entity.remove')}: ${entry.authored}` });
      // An end entry has nowhere to go in that direction: the control stays in
      // place, disabled, so the row keeps the same shape for a keyboard user.
      if (index === 0) up.dataset.readonly = 'true';
      if (index === form.includes.length - 1) down.dataset.readonly = 'true';
      const move = direction => () => {
        const target = moveInclude(form, index, direction);
        if (target == null) return;
        renderIncludes(); refresh();
        const kind = direction < 0 ? 'up' : 'down', other = direction < 0 ? 'down' : 'up';
        focusFirst(`include-${target}-${kind}`, `include-${target}-${other}`, `include-${target}-remove`);
      };
      up.addEventListener('click', move(-1));
      down.addEventListener('click', move(1));
      remove.addEventListener('click', () => {
        form.includes.splice(index, 1); renderIncludes(); refresh();
        // The entry that took this place, else the last one left, else the add control.
        focusFirst(`include-${index}-remove`, `include-${index - 1}-remove`, 'add-include');
      });
      row.append(label, up, down, remove);
      includesList.append(row);
    });
    if (!form.includes.length) includesList.append(node('li', 'workshop.entity.no_includes'));
  }

  /** One row per component the runtime supports, saying whether the draft owns
   * it, a fragment does, BOTH do, or nothing does — in words, never by colour
   * alone. Add writes the runtime's own default; Remove takes away local text.
   *
   * A component the draft authors and a fragment ALSO authors — the central case
   * of composition — is `override`: it has local text to remove, so Remove is a
   * real control, and the row names the fragment whose copy composes once that
   * text is gone. A component with NO local text is `inherited` and carries the
   * sentence that says what to do instead, because a whole inherited table has no
   * tombstone the merge understands. */
  function renderComponents() {
    componentsList.replaceChildren();
    const current = composition();
    if (!current) { componentsList.append(node('li', 'workshop.entity.no_components')); return; }
    const entries = new Map((current.components || []).map(entry => [entry.key, entry]));
    const keys = (current.supported_components || []).length ? current.supported_components : [...entries.keys()];
    for (const key of keys) {
      const component = entries.get(key) || { key, local: false, local_line: null, inherited_from: null, skeleton: false };
      const local = Boolean(component.local);
      const state = local ? (component.inherited_from ? 'override' : 'local')
        : component.inherited_from ? 'inherited' : 'absent';
      const row = node('li', null, { 'data-state': state, 'data-component': key });
      const label = node('span');
      const words = [];
      if (local) {
        words.push(t('workshop.entity.component_local', { key, line: String(component.local_line ?? '') }));
        if (component.inherited_from) words.push(t('workshop.entity.component_override', { source: component.inherited_from }));
      } else if (component.inherited_from) {
        words.push(t('workshop.entity.component_inherited', { key, source: component.inherited_from }));
        words.push(t('workshop.entity.component_materialise_first'));
      } else {
        words.push(t('workshop.entity.component_absent', { key }));
        if (!componentSkeleton(component)) words.push(t('workshop.entity.no_skeleton'));
      }
      label.textContent = words.join(' ');
      row.append(label);
      if (componentAddable(component)) {
        const add = node('button', 'workshop.entity.add_component', { type: 'button', id: `${PREFIX}-component-${key}-add`,
          'aria-label': `${t('workshop.entity.add_component')}: ${key}` });
        add.addEventListener('click', () => {
          if (busy() || add.disabled || !fresh()) return;
          const read = reading, target = composition();
          let edits;
          try { edits = planComponentAdd({ composition: target, key }); }
          catch (error) { show(knownId(error), true, { detail: error?.detail ?? '' }); return; }
          void guarded(() => commit(read, target.path, edits, [`component-${key}-remove`, `component-${key}-add`, 'refresh']));
        });
        row.append(add);
      }
      if (local) {
        const remove = node('button', 'workshop.entity.remove', { type: 'button', id: `${PREFIX}-component-${key}-remove`,
          // The accessible name says what this particular Remove does: on an
          // override row it drops local text and leaves the fragment's copy.
          'aria-label': component.inherited_from
            ? `${t('workshop.entity.remove')}: ${key} — ${t('workshop.entity.component_override', { source: component.inherited_from })}`
            : `${t('workshop.entity.remove')}: ${key}` });
        remove.addEventListener('click', () => {
          if (busy() || remove.disabled || !fresh()) return;
          const read = reading, target = composition();
          let edits;
          try { edits = planComponentRemove({ composition: target, key }); }
          catch (error) { show(knownId(error), true, { detail: error?.detail ?? '' }); return; }
          // A removal takes the pressed control out of the rebuilt list: the Add
          // that replaces it takes focus, else the reading control.
          void guarded(() => commit(read, target.path, edits, [`component-${key}-add`, `component-${key}-remove`, 'refresh']));
        });
        row.append(remove);
      }
      componentsList.append(row);
    }
  }

  function resetFields() {
    const current = composition();
    fieldsState = current ? fieldsForm(current) : null;
    renderFields();
  }

  function renderFields() {
    fieldsNode.replaceChildren();
    const current = composition();
    if (!current || !fieldsState) return;
    const form = fieldsState;
    const groups = fieldGroups(current);
    if (!groups.length) { fieldsNode.append(node('p', 'workshop.entity.no_fields')); return; }
    let position = 0;
    for (const group of groups) {
      const set = node('fieldset', null, { class: 'workshop-entity-group' });
      const legend = node('legend');
      legend.textContent = t('workshop.entity.field_group', { key: group.key });
      set.append(legend);
      const list = node('ul');
      for (const field of group.fields) {
        const index = position; position += 1;
        const row = node('li', null, { 'data-owner': field.local ? 'local' : 'inherited' });
        const value = node('input', null, { type: 'text', id: `${PREFIX}-field-${index}-value`, spellcheck: 'false' });
        value.value = form.values[field.address] ?? '';
        const words = [field.address];
        if (field.kind) words.push(t('workshop.entity.field_kind', { kind: field.kind }));
        words.push(field.local
          ? t('workshop.entity.field_local', { line: String(field.line ?? '') })
          : t('workshop.entity.field_inherited', { source: field.source ?? '' }));
        if ((field.chain || []).length) words.push(t('workshop.entity.field_chain', { chain: field.chain.join(' → ') }));
        // Inherited is read-only until it is materialised (criterion 2); a keyed
        // address is the station and system forms' to author (#1481); and a list
        // or sub-table is a source edit, which the `set` route refuses. Each
        // says WHICH of the three it is rather than being silently disabled.
        if (!fieldIsEditable(field)) {
          value.dataset.readonly = 'true';
          if (field.local) {
            words.push(fieldIsScalar(field)
              ? t('workshop.entity.field_keyed')
              : t('workshop.entity.field_structured', { kind: field.kind ?? '' }));
          }
        }
        // An inherited value the local document cannot name yet — inside an
        // array entry this template does not author — is refused by the runtime,
        // so the row says so rather than offering a dead Materialise.
        if (!field.local && !fieldIsMaterialisable(field)) words.push(t('workshop.entity.field_no_entry'));
        value.addEventListener('input', () => { form.values[field.address] = value.value; });
        row.append(labelled(null, value, words.join(' — ')), value);
        if (fieldIsMaterialisable(field)) {
          const materialise = node('button', 'workshop.entity.materialise', { type: 'button',
            id: `${PREFIX}-field-${index}-materialise`, 'aria-label': `${t('workshop.entity.materialise')}: ${field.address}` });
          materialise.addEventListener('click', () => {
            if (busy() || materialise.disabled || !fresh()) return;
            const read = reading, target = composition();
            if (typeof runtime?.materialiseEntity !== 'function') { show('workshop.inspector_refused', true); return; }
            void guarded(() => materialiseField(read, target.path, field.address,
              [`field-${index}-value`, `field-${index}-materialise`, 'apply-fields']));
          });
          row.append(materialise);
        }
        list.append(row);
      }
      set.append(list);
      fieldsNode.append(set);
    }
  }

  function renderFindings() {
    const records = composition()?.findings || [];
    findingsList.replaceChildren(...records.map(record => {
      const row = node('li', null, { 'data-severity': record.severity });
      const location = `${record.file}${record.line ? `:${record.line}` : ''}`;
      // The severity is a word, never a colour alone.
      row.textContent = `${location} — ${t(`workshop.severity.${record.severity}`)}: ${record.message}`;
      return row;
    }));
    if (!records.length) findingsList.append(node('li', 'workshop.entity.no_findings'));
  }

  /** Read the selected template through the runtime. An answer for a draft that
   * moved while it was being read is discarded rather than shown against newer
   * source. A form holding UNAPPLIED edits over a member that is byte-for-byte
   * what the previous reading held keeps them: applying the includes must not
   * throw away a field typed beside them, nor the reverse. An untouched form is
   * rebuilt from the new reading like everything else. */
  async function reload({ announce = true } = {}) {
    const candidate = draft();
    const path = template.value;
    if (!candidate || !path) return;
    const snapshot = definitionsSnapshot(candidate);
    const result = await runtime.entity(snapshot.files, path);
    if (disposed || !snapshotIsCurrent(snapshot, draft())) return;
    if (!result || typeof result !== 'object' || typeof result.path !== 'string' || !Array.isArray(result.includes)
      || !Array.isArray(result.components) || !Array.isArray(result.fields)
      || !Array.isArray(result.supported_components)) throw new Error('workshop.inspector_refused');
    const dirty = (state, current) => Boolean(state) && Boolean(current) && JSON.stringify(state) !== JSON.stringify(current);
    const previous = composition();
    const pending = {
      includes: dirty(includesState, previous && includesForm(previous)) ? includesState : null,
      fields: dirty(fieldsState, previous && fieldsForm(previous)) ? fieldsState : null,
      path: previous?.path, source: reading?.files?.[previous?.path],
    };
    reading = { ...snapshot, composition: result };
    renderAll();
    if (pending.path && result.path === pending.path && snapshot.files[pending.path] === pending.source) {
      if (pending.includes) { includesState = pending.includes; renderIncludes(); }
      if (pending.fields) { fieldsState = pending.fields; renderFields(); }
    }
    if (announce) show('workshop.entity.refreshed');
  }

  /** The hold goes up for the whole of one runtime call and comes down once its
   * answer has been rendered. Focus moves AFTER that: every control is disabled
   * while the hold is up, so a landing spot chosen inside it could only ever be
   * one the rebuild happened to recreate — the stable fallbacks (`refresh`,
   * `apply-fields`, `add-include`) were all ineligible, and after the Remove of a
   * component with no skeleton focus fell to the body. */
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

  /** The runtime's own refusal, mapped to the sentence for its rule and
   * carrying its words as the detail. */
  function refused(error) {
    const message = refusalMessage(error);
    const value = new Error(refusalStringId(message));
    value.detail = message;
    return value;
  }

  /** What a landed answer does to the draft: ONE edit, so one undo reverts it,
   * then a re-read so the forms show what was written. An answer for a draft
   * that moved in the meantime is refused as stale rather than written over
   * newer source. The landing spot is RETURNED for `guarded` to apply once the
   * hold is down and the controls have been refreshed. */
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
    try { result = await runtime.editEntity(read.files, { document_path: path, expected_source: read.files[path], edits }); }
    catch (error) { throw refused(error); }
    return land(read, path, result, focus);
  }

  /** Materialising is the only place a runtime VALUE becomes source, and it
   * writes new local text only: the runtime reads the resolved value at that
   * address and puts it into the local document, leaving every other byte alone. */
  async function materialiseField(read, path, address, focus = []) {
    let result;
    try { result = await runtime.materialiseEntity(read.files, path, address); }
    catch (error) { throw refused(error); }
    return land(read, path, result, focus);
  }

  refreshButton.addEventListener('click', () => {
    if (busy() || refreshButton.disabled) return;
    void guarded(() => reload());
  });
  applyIncludes.addEventListener('click', () => {
    if (busy() || applyIncludes.disabled || !fresh()) return;
    const read = reading, current = composition();
    if (!current || !includesState) return;
    let edits;
    try { edits = planIncludeEdits({ composition: current, form: includesState }); }
    catch (error) { show(knownId(error), true, { detail: error?.detail ?? '' }); return; }
    if (!edits.length) { show('workshop.entity.unchanged'); return; }
    void guarded(() => commit(read, current.path, edits, ['add-include', 'apply-includes']));
  });
  addIncludeButton.addEventListener('click', () => {
    if (busy() || addIncludeButton.disabled || !includesState) return;
    const current = composition();
    if (!addInclude.value || !current) return;
    const choice = (current.fragment_choices || []).find(entry => entry.path === addInclude.value)
      || { path: addInclude.value, origin: null };
    includesState.includes.push(newInclude(current.path, choice));
    renderIncludes(); refresh();
    section.querySelector(`#${PREFIX}-add-include`)?.focus();
  });
  applyFields.addEventListener('click', () => {
    if (busy() || applyFields.disabled || !fresh()) return;
    const read = reading, current = composition();
    if (!current || !fieldsState) return;
    let edits;
    try { edits = planFieldEdits({ composition: current, form: fieldsState }); }
    catch (error) { show(knownId(error), true, { detail: error?.detail ?? '' }); return; }
    if (!edits.length) { show('workshop.entity.unchanged'); return; }
    void guarded(() => commit(read, current.path, edits, ['apply-fields', 'refresh']));
  });
  /** Criterion 5 reuses the surfaces that already exist: the shared model
   * preview takes this template as its subject and the disposable Test takes it
   * as the ship, both from the same unsaved source. A surface that cannot take
   * it says so instead of appearing to work. */
  previewButton.addEventListener('click', () => {
    if (busy() || previewButton.disabled) return;
    const current = composition();
    if (!current) return;
    if (preview(current.path)) show('workshop.entity.previewing', false, { path: current.path });
    else show('workshop.entity.preview_unavailable', true, { detail: current.path });
  });
  testButton.addEventListener('click', () => {
    if (busy() || testButton.disabled) return;
    const current = composition();
    if (!current) return;
    if (test(current.path)) show('workshop.entity.testing', false, { path: current.path });
    else show('workshop.entity.test_unavailable', true, { detail: current.path });
  });
  /** A reading belongs to ONE template, so choosing another has nothing to show
   * until the runtime answers for it. The selector's own options survive that,
   * and the unapplied forms of the template being left do not: they described a
   * different document. */
  template.addEventListener('change', () => {
    reading = null; includesState = null; fieldsState = null;
    renderAll(); refresh();
    if (busy() || !draft() || typeof runtime?.entity !== 'function') { show('workshop.entity.empty'); return; }
    void guarded(() => reload());
  });
  // Rendered once before anything is read, so the panel says what it is waiting
  // for rather than showing empty sections with no explanation.
  renderAll();
  show('workshop.entity.empty');
  refresh();
  return { refresh, node: section, dispose() { disposed = true; section.remove(); } };
}
