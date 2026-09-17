import { modelDocuments, modelFieldGroup, createModelVariant, patchModelFields } from '../editor/workshop-models.js';
import { t } from './strings.js';
import { validInspectorDescriptor, renderInspectorMetadata } from './inspector-field.js';
import { mountWorkshopModelPreview } from './workshop-model-preview-panel.js';

/** Specialised source form over the ordinary Workshop inspector and history.
 * The model/variant selections are local presentation, never simulation inputs. */
export function mountWorkshopModels({ root, provider, runtime, draft, busy, setBusy, changed, attach = true }) {
  const doc = root.ownerDocument;
  const node = (tag, id, attrs = {}) => {
    const value = doc.createElement(tag);
    if (id) value.textContent = t(id);
    for (const [key, item] of Object.entries(attrs)) value.setAttribute(key, item);
    return value;
  };
  // A dock panel carries its own header, so this is a plain grouping element.
  const section = node('section', null, { class: 'workshop-models', id: 'workshop-models' });
  const model = node('select', null, { id: 'workshop-model' });
  const variant = node('select', null, { id: 'workshop-model-variant' });
  const inspect = node('button', 'workshop.inspect', { type: 'button', id: 'workshop-model-inspect' });
  const newName = node('input', null, { type: 'text', maxlength: '64', id: 'workshop-model-new-variant' });
  const clone = node('button', 'workshop.models.clone', { type: 'button', id: 'workshop-model-clone' });
  const status = node('p', null, { role: 'status', tabindex: '-1', id: 'workshop-model-status' });
  const form = node('div', null, { class: 'workshop-model-fields' });
  const apply = node('button', 'workshop.models.apply', { type: 'button', id: 'workshop-model-apply' });
  section.append(node('p', 'workshop.models.source_scope'), node('label', 'workshop.models.model', { for: model.id }), model,
    node('label', 'workshop.models.variant', { for: variant.id }), variant, inspect,
    node('label', 'workshop.models.new_variant', { for: newName.id }), newName, clone, status, form, apply);
  // A docked panel is placed by the renderer, which moves the node into its frame.
  // Attaching here as well would strand the node in `root` whenever the stored
  // layout has the panel closed, because a closed panel is never framed.
  if (attach) root.append(section);
  let disposed = false, snapshot = null, rows = [], previousDraft = null, previousPaths = '', preview = null;
  let previewVisible = true, testHidden = false;
  const option = (value, label) => { const item = node('option', null, { value }); item.textContent = label; return item; };
  const show = (id, error = false) => {
    status.textContent = t(id);
    status.setAttribute('role', error ? 'alert' : 'status');
    if (error) status.focus();
  };
  function refreshVariants() {
    const old = variant.value;
    const entry = modelDocuments(draft()?.paths() || []).find(entry => entry.model === model.value);
    variant.replaceChildren(...(entry?.variants || []).map(entry => option(entry.path, entry.name)));
    if (entry?.variants.some(entry => entry.path === old)) variant.value = old;
  }
  function refresh({ hidden = testHidden } = {}) {
    testHidden = hidden;
    section.hidden = hidden;
    const current = draft(), paths = current?.paths().join('\n') || '';
    if (current !== previousDraft || paths !== previousPaths) {
      const old = model.value;
      const entries = modelDocuments(current?.paths() || []);
      model.replaceChildren(...entries.map(entry => option(entry.model, entry.model)));
      if (entries.some(entry => entry.model === old)) model.value = old;
      previousDraft = current; previousPaths = paths; refreshVariants();
    }
    const held = busy();
    model.disabled = variant.disabled = newName.disabled = held || !model.value;
    inspect.disabled = held || !variant.value;
    clone.disabled = held || !model.value;
    const fresh = snapshot && snapshot.draft === current && snapshot.path === variant.value
      && snapshot.source === current?.read(snapshot.path);
    apply.disabled = held || !fresh || !rows.length;
    for (const row of rows) row.input.disabled = held || !fresh;
    if (snapshot && !fresh) show('workshop.inspector_stale');
    if (!current || !model.value) show('workshop.models.empty');
    // The captured picture stops whenever its own panel stops being shown, not
    // only in Test mode: a closed, unselected or narrow-projected-away panel is
    // as invisible as a hidden one, and holding a render session open then keeps
    // an asset capture the operator can no longer see.
    preview?.refresh({ hidden: hidden || !previewVisible });
  }
  function render(fields) {
    form.replaceChildren(); rows = [];
    const groups = new Map();
    fields.forEach((field, index) => {
      const group = modelFieldGroup(field);
      if (!groups.has(group)) {
        const fieldset = node('fieldset'); fieldset.append(node('legend', `workshop.models.group.${group}`));
        groups.set(group, fieldset); form.append(fieldset);
      }
      const row = node('div', null, { class: 'workshop-model-field' });
      const input = node('input', null, { type: 'text', id: `workshop-model-field-${index}`, spellcheck: 'false' });
      input.value = field.source;
      const label = node('label', null, { for: input.id });
      label.textContent = field.path.map(key => typeof key === 'number' ? `[${key}]` : key).join('.');
      const metadata = node('small');
      if (validInspectorDescriptor(field)) renderInspectorMetadata(metadata, field, { t });
      else {
        metadata.textContent = t(field.runtime_owned ? 'workshop.field_runtime' : 'workshop.field_fallback',
          { type: field.kind, line: String(field.line) });
        if (field.default_source != null) metadata.textContent += ` ${t('workshop.field_default', { value: field.default_source })}`;
      }
      row.append(label, input, metadata); groups.get(group).append(row); rows.push({ field, input });
    });
  }
  inspect.addEventListener('click', async () => {
    if (busy() || !variant.value) return;
    const candidate = draft(), path = variant.value, source = candidate.read(path);
    if (typeof source !== 'string') return;
    setBusy(true);
    try {
      const fields = await runtime.inspect(source, path);
      if (disposed || draft() !== candidate || candidate.read(path) !== source || variant.value !== path) return;
      snapshot = { draft: candidate, path, source };
      render(fields); show(fields.length ? 'workshop.models.inspected' : 'workshop.inspector_empty');
    } catch {
      if (!disposed) show('workshop.inspector_refused', true);
    } finally { if (!disposed) { setBusy(false); refresh(); } }
  });
  apply.addEventListener('click', async () => {
    if (busy() || !snapshot || apply.disabled) return;
    const read = snapshot;
    const fields = rows.map(row => row.field), values = rows.map(row => row.input.value);
    setBusy(true);
    try {
      const edited = await patchModelFields({ draft: read.draft, runtime, documentPath: read.path,
        source: read.source, fields, values,
        current: () => !disposed && draft() === read.draft && variant.value === read.path });
      if (edited) { snapshot = null; form.replaceChildren(); rows = []; changed(read.path); }
      show(edited ? 'workshop.changed' : 'workshop.models.unchanged');
    } catch (error) {
      if (!disposed) show(error.message === 'workshop.inspector_stale' ? error.message : 'workshop.inspector_refused', true);
    } finally { if (!disposed) { setBusy(false); refresh(); } }
  });
  clone.addEventListener('click', () => {
    if (busy() || clone.disabled) return;
    try {
      const path = createModelVariant(draft(), model.value, newName.value, variant.value || null);
      refresh(); variant.value = path; newName.value = ''; snapshot = null; form.replaceChildren(); rows = [];
      changed(path); show('workshop.changed'); refresh(); inspect.focus();
    } catch (error) {
      show(['workshop.models.invalid_variant', 'workshop.models.variant_exists'].includes(error.message)
        ? error.message : 'workshop.inspector_refused', true);
    }
  });
  model.addEventListener('change', () => { refreshVariants(); refresh(); });
  variant.addEventListener('change', () => refresh());
  refresh();
  preview = mountWorkshopModelPreview({ root: section, attach, provider, draft, busy,
    selection: () => ({ model: model.value,
      variant: modelDocuments(draft()?.paths() || []).find(entry => entry.model === model.value)?.variants
        .find(entry => entry.path === variant.value)?.name || null }) });
  return { refresh, node: section, previewNode: preview.node,
    setPreviewVisible(value) {
      if (previewVisible === value) return;
      previewVisible = value; refresh();
    },
    dispose() { disposed = true; preview.dispose(); section.remove(); } };
}
