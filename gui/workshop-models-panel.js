import { modelDocuments, entityDocuments, modelFieldGroup, patchModelFields } from '../editor/workshop-models.js';
import { applyModelStructureOperation, inspectModelStructure } from '../editor/workshop-model-structure.js';
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
  const renameVariant = node('button', 'workshop.models.rename_variant', { type: 'button', id: 'workshop-model-rename-variant' });
  const removeVariant = node('button', 'workshop.models.remove_variant', { type: 'button', id: 'workshop-model-remove-variant' });
  // What the preview shows. A model is its GLB and rig; an entity template is
  // its composed visual, which is how a star and a planet become previewable —
  // the shared viewer dispatches [star], [planet] and [mesh] itself, so there is
  // no separate list of either to keep in step with authored content.
  const subject = node('select', null, { id: 'workshop-preview-subject' });
  const status = node('p', null, { role: 'status', tabindex: '-1', id: 'workshop-model-status' });
  const form = node('div', null, { class: 'workshop-model-fields' });
  const structure = node('div', null, { class: 'workshop-model-structure', id: 'workshop-model-structure' });
  const apply = node('button', 'workshop.models.apply', { type: 'button', id: 'workshop-model-apply' });
  section.append(node('p', 'workshop.models.source_scope'), node('label', 'workshop.models.model', { for: model.id }), model,
    node('label', 'workshop.models.variant', { for: variant.id }), variant, inspect,
    node('label', 'workshop.models.new_variant', { for: newName.id }), newName, clone, renameVariant, removeVariant,
    node('label', 'workshop.models.preview_subject', { for: subject.id }), subject,
    status, structure, form, apply);
  // A docked panel is placed by the renderer, which moves the node into its frame.
  // Attaching here as well would strand the node in `root` whenever the stored
  // layout has the panel closed, because a closed panel is never framed.
  if (attach) root.append(section);
  let disposed = false, snapshot = null, rows = [], previousDraft = null, previousPaths = '', preview = null;
  let dependencies = null, dependencyLoad = null, structureSnapshot = null;
  let billboardResult = null;
  let lodGenerationResult = null;
  let previewVisible = true, testHidden = false;
  const option = (value, label) => { const item = node('option', null, { value }); item.textContent = label; return item; };
  const show = (id, error = false) => {
    status.textContent = t(id);
    status.setAttribute('role', error ? 'alert' : 'status');
    if (error) status.focus();
  };
  const vector = input => {
    const result = input.value.split(',').map(value => Number(value.trim()));
    if (result.length !== 3 || result.some(value => !Number.isFinite(value))) throw new Error('model-structure-vector');
    return result;
  };
  const vectorInput = (id, value, labelId) => {
    const input = node('input', null, { type: 'text', id, value: (value || [0, 0, 0]).join(', '), spellcheck: 'false' });
    input.value = (value || [0, 0, 0]).join(', ');
    return [node('label', labelId, { for: id }), input];
  };
  const action = (id, label, invoke, boundary = false) => {
    const button = node('button', label, { type: 'button', id });
    if (boundary) button.dataset.boundary = 'true';
    button.addEventListener('click', () => { try { invoke(); } catch { show('workshop.inspector_refused', true); } }); return button;
  };
  async function loadDependencies() {
    if (dependencies || dependencyLoad || typeof runtime.dependencies !== 'function') return dependencyLoad;
    dependencyLoad = runtime.dependencies();
    try { dependencies = await dependencyLoad; } catch { dependencies = {}; }
    finally { dependencyLoad = null; if (!disposed) { renderStructure(); refresh(); } }
    return dependencies;
  }
  async function commitStructure(operation) {
    if (busy()) return;
    const candidate = draft(), selected = variant.value;
    setBusy(true);
    try {
      const result = await applyModelStructureOperation({ draft: candidate, provider, runtime,
        dependencies: dependencies || {}, operation,
        current: () => !disposed && draft() === candidate && variant.value === selected });
      snapshot = null; form.replaceChildren(); rows = []; changed(result.selected || operation.path);
      refreshVariants();
      if (result.selected) variant.value = result.selected;
      else if (variant.options.length) variant.selectedIndex = 0;
      show('workshop.changed'); renderStructure();
    } catch (error) {
      if (!disposed) {
        const finding = error.report?.findings?.[0];
        status.textContent = finding ? `${finding.file}:${finding.line} ${finding.message}` : t('workshop.inspector_refused');
        status.setAttribute('role', 'alert'); status.focus();
      }
    } finally { if (!disposed) { setBusy(false); refresh(); } }
  }
  function provenance(row) {
    const value = node('small', null, { class: 'workshop-model-owner' });
    value.textContent = t('workshop.models.owner', { file: row.owner, line: String(row.line) }); return value;
  }
  function renderStructure() {
    structure.replaceChildren(); structureSnapshot = null;
    const current = draft(), selected = variant.value;
    if (!current || !model.value || !selected) return;
    let view;
    try { view = inspectModelStructure(current, model.value, selected, dependencies || {}); }
    catch { return; }
    structureSnapshot = { draft: current, path: selected, source: current.read(selected) };
    const group = (legend, id) => { const fieldset = node('fieldset', null, { id }); fieldset.append(node('legend', legend)); structure.append(fieldset); return fieldset; };
    const markers = group('workshop.models.structure.markers', 'workshop-model-markers');
    view.markers.forEach((marker, index) => {
      const row = node('div', null, { class: 'workshop-model-structure-row' });
      const name = node('input', null, { type: 'text', id: `workshop-model-marker-name-${index}`, maxlength: '64' }); name.value = marker.name;
      const [positionLabel, position] = vectorInput(`workshop-model-marker-position-${index}`, marker.position, 'workshop.models.position');
      const [directionLabel, direction] = vectorInput(`workshop-model-marker-direction-${index}`, marker.direction, 'workshop.models.direction');
      row.append(node('label', 'workshop.models.name', { for: name.id }), name, positionLabel, position, directionLabel, direction,
        provenance(marker),
        action(`workshop-model-marker-save-${index}`, 'workshop.models.save', () => commitStructure({ type: 'marker-set', path: selected,
          name: marker.name, newName: name.value, position: vector(position), direction: vector(direction) })),
        action(`workshop-model-marker-up-${index}`, 'workshop.models.up', () => commitStructure({ type: 'marker-move', path: selected, name: marker.name, direction: -1 }), index === 0),
        action(`workshop-model-marker-down-${index}`, 'workshop.models.down', () => commitStructure({ type: 'marker-move', path: selected, name: marker.name, direction: 1 }), index === view.markers.length - 1),
        action(`workshop-model-marker-remove-${index}`, 'workshop.models.remove', () => commitStructure({ type: 'marker-remove', path: selected, name: marker.name })));
      markers.append(row);
    });
    const markerName = node('input', null, { type: 'text', id: 'workshop-model-marker-new-name', maxlength: '64' });
    const [markerPositionLabel, markerPosition] = vectorInput('workshop-model-marker-new-position', [0, 0, 0], 'workshop.models.position');
    const [markerDirectionLabel, markerDirection] = vectorInput('workshop-model-marker-new-direction', [0, 0, 1], 'workshop.models.direction');
    markers.append(node('label', 'workshop.models.new_marker', { for: markerName.id }), markerName, markerPositionLabel, markerPosition,
      markerDirectionLabel, markerDirection, action('workshop-model-marker-add', 'workshop.models.add', () => commitStructure({
        type: 'marker-add', path: selected, name: markerName.value, position: vector(markerPosition), direction: vector(markerDirection) })));

    const targets = group('workshop.models.structure.targets', 'workshop-model-targets');
    view.target_points.forEach((target, index) => {
      const row = node('div', null, { class: 'workshop-model-structure-row' });
      const [label, position] = vectorInput(`workshop-model-target-position-${index}`, target.position, 'workshop.models.position');
      row.append(label, position, provenance(target),
        action(`workshop-model-target-save-${index}`, 'workshop.models.save', () => commitStructure({ type: 'target-set', path: selected, index, position: vector(position) })),
        action(`workshop-model-target-up-${index}`, 'workshop.models.up', () => commitStructure({ type: 'target-move', path: selected, index, direction: -1 }), index === 0),
        action(`workshop-model-target-down-${index}`, 'workshop.models.down', () => commitStructure({ type: 'target-move', path: selected, index, direction: 1 }), index === view.target_points.length - 1),
        action(`workshop-model-target-remove-${index}`, 'workshop.models.remove', () => commitStructure({ type: 'target-remove', path: selected, index })));
      targets.append(row);
    });
    const [targetLabel, targetPosition] = vectorInput('workshop-model-target-new-position', [0, 0, 0], 'workshop.models.position');
    targets.append(targetLabel, targetPosition, action('workshop-model-target-add', 'workshop.models.add', () => commitStructure({
      type: 'target-add', path: selected, position: vector(targetPosition) })));

    const lods = group('workshop.models.structure.lod', 'workshop-model-lods');
    view.lod.forEach((level, index) => {
      const row = node('div', null, { class: 'workshop-model-structure-row' });
      const kind = node('select', null, { id: `workshop-model-lod-kind-${index}` });
      for (const value of ['model', 'billboard', 'shape']) kind.append(option(value, t(`workshop.models.lod.${value}`)));
      kind.value = level.model ? 'model' : level.billboard ? 'billboard' : 'shape';
      const reference = node('input', null, { type: 'text', id: `workshop-model-lod-reference-${index}` }); reference.value = level[kind.value] || '';
      const lodVariant = node('input', null, { type: 'text', id: `workshop-model-lod-variant-${index}` }); lodVariant.value = level.variant || '';
      const distance = node('input', null, { type: 'number', min: '0', step: 'any', id: `workshop-model-lod-distance-${index}` }); distance.value = level.max_distance ?? '';
      const levelValue = () => ({ [kind.value]: reference.value, ...(lodVariant.value ? { variant: lodVariant.value } : {}),
        ...(distance.value ? { max_distance: Number(distance.value) } : {}) });
      row.append(node('label', 'workshop.models.renderer', { for: kind.id }), kind,
        node('label', 'workshop.models.reference', { for: reference.id }), reference,
        node('label', 'workshop.models.lod_variant', { for: lodVariant.id }), lodVariant,
        node('label', 'workshop.models.max_distance', { for: distance.id }), distance, provenance(level),
        action(`workshop-model-lod-save-${index}`, 'workshop.models.save', () => commitStructure({ type: 'lod-set', path: selected, index, level: levelValue() })),
        action(`workshop-model-lod-up-${index}`, 'workshop.models.up', () => commitStructure({ type: 'lod-move', path: selected, index, direction: -1 }), index === 0),
        action(`workshop-model-lod-down-${index}`, 'workshop.models.down', () => commitStructure({ type: 'lod-move', path: selected, index, direction: 1 }), index === view.lod.length - 1),
        action(`workshop-model-lod-remove-${index}`, 'workshop.models.remove', () => commitStructure({ type: 'lod-remove', path: selected, index })));
      lods.append(row);
    });
    const kind = node('select', null, { id: 'workshop-model-lod-new-kind' });
    for (const value of ['model', 'billboard', 'shape']) kind.append(option(value, t(`workshop.models.lod.${value}`)));
    const reference = node('input', null, { type: 'text', id: 'workshop-model-lod-new-reference' }); reference.value = model.value;
    const distance = node('input', null, { type: 'number', min: '0', step: 'any', id: 'workshop-model-lod-new-distance' });
    lods.append(node('label', 'workshop.models.renderer', { for: kind.id }), kind,
      node('label', 'workshop.models.reference', { for: reference.id }), reference,
      node('label', 'workshop.models.max_distance', { for: distance.id }), distance,
      action('workshop-model-lod-add', 'workshop.models.add', () => commitStructure({ type: 'lod-add', path: selected,
        index: view.lod.at(-1)?.max_distance == null ? Math.max(0, view.lod.length - 1) : view.lod.length,
        level: { [kind.value]: reference.value, ...(distance.value ? { max_distance: Number(distance.value) } : {}) } })));
    const captures = view.lod.map((level, index) => ({ level, index })).filter(({ level }) => level.capture && level.billboard);
    if (captures.length) {
      const captureGroup = group('workshop.models.capture.heading', 'workshop-model-captures');
      for (const { level, index } of captures) {
        const row = node('div', null, { class: 'workshop-model-capture' });
        const description = node('p');
        description.textContent = t('workshop.models.capture.authored', { output: level.billboard,
          source: level.capture.source, views: String(level.capture.yaw_views), resolution: String(level.capture.resolution),
          pitch: String(level.capture.pitch) });
        row.append(description);
        if (provider.billboardCapture) {
          const run = async work => {
            if (busy()) return;
            setBusy(true);
            try { await work(); }
            catch (error) { billboardResult = provider.billboardCapture.active;
              if (!disposed) show(error.message === 'workshop.billboard.stale' ? error.message : 'workshop.models.capture.failed', true); }
            finally { if (!disposed) { setBusy(false); renderStructure(); refresh(); } }
          };
          const start = action(`workshop-model-capture-start-${index}`, 'workshop.models.capture.start', () => run(async () => {
            billboardResult = await provider.billboardCapture.start(current, selected, index); show('workshop.models.capture.running');
          }));
          const poll = action(`workshop-model-capture-status-${index}`, 'workshop.models.capture.status', () => run(async () => {
            billboardResult = await provider.billboardCapture.status(current, selected, index);
            show(billboardResult?.state === 'ready' ? 'workshop.models.capture.ready' : 'workshop.models.capture.running');
          }), billboardResult?.sidecar !== selected || billboardResult?.lod !== index);
          const adopt = action(`workshop-model-capture-adopt-${index}`, 'workshop.models.capture.adopt', () => run(async () => {
            const adopted = await provider.billboardCapture.adopt(current, selected, index); billboardResult = null; changed(adopted.path);
            show('workshop.models.capture.adopted');
          }), billboardResult?.state !== 'ready' || billboardResult?.sidecar !== selected || billboardResult?.lod !== index);
          const cancel = action(`workshop-model-capture-cancel-${index}`, 'workshop.models.capture.cancel', () => run(async () => {
            await provider.billboardCapture.cancel(); billboardResult = null; show('workshop.models.capture.cancelled');
          }), billboardResult?.sidecar !== selected || billboardResult?.lod !== index);
          row.append(start, poll, adopt, cancel);
          if (billboardResult?.state === 'ready' && billboardResult.sidecar === selected && billboardResult.lod === index) {
            const image = node('img', null, { src: billboardResult.image_url, alt: t('workshop.models.capture.preview_alt', { output: level.billboard }) });
            const provenance = node('p', null, { class: 'workshop-model-owner' });
            provenance.textContent = t('workshop.models.capture.provenance', { revision: String(billboardResult.source_revision),
              source: billboardResult.source, views: String(billboardResult.yaw_views), resolution: String(billboardResult.resolution),
              pitch: String(billboardResult.pitch) });
            row.append(image, provenance);
          }
        }
        captureGroup.append(row);
      }
    }
    if (provider?.lodGeneration && view.lod.some(level => level.generate)) {
      const generation = group('workshop.models.generation.heading', 'workshop-model-generation');
      const remesh = node('input', null, { type: 'checkbox', id: 'workshop-model-generation-remesh' });
      const progress = node('pre', null, { id: 'workshop-model-generation-progress', 'aria-live': 'polite' });
      progress.textContent = (lodGenerationResult?.progress || []).join('\n');
      const run = async work => {
        if (busy()) return;
        setBusy(true);
        try { await work(); }
        catch (error) { lodGenerationResult = provider.lodGeneration.active;
          if (!disposed) show(error.message === 'workshop.lod_generation.stale' ? error.message : 'workshop.models.generation.failed', true); }
        finally { if (!disposed) { setBusy(false); renderStructure(); refresh(); } }
      };
      const start = action('workshop-model-generation-start', 'workshop.models.generation.start', () => run(async () => {
        lodGenerationResult = await provider.lodGeneration.start(current, selected, { remesh: remesh.checked });
        show('workshop.models.generation.running');
      }));
      const poll = action('workshop-model-generation-status', 'workshop.models.generation.status', () => run(async () => {
        lodGenerationResult = await provider.lodGeneration.status(current, selected);
        show(lodGenerationResult.reviewReady ? 'workshop.models.generation.ready' : 'workshop.models.generation.running');
      }), lodGenerationResult?.sidecar !== selected);
      const review = action('workshop-model-generation-review', 'workshop.models.generation.review', () => run(async () => {
        const candidate = provider.lodGeneration.reviewDraft(current, selected);
        const name = modelDocuments(candidate.paths()).find(entry => entry.model === model.value)?.variants
          .find(entry => entry.path === selected)?.name || null;
        await preview.review(candidate, { model: model.value, variant: name }); show('workshop.models.generation.previewing');
      }), !lodGenerationResult?.reviewReady || lodGenerationResult?.sidecar !== selected);
      const adopt = action('workshop-model-generation-adopt', 'workshop.models.generation.adopt', () => run(async () => {
        const adopted = await provider.lodGeneration.adopt(current, selected); lodGenerationResult = null;
        if (adopted.changed) changed(adopted.paths[0]);
        show(adopted.changed ? 'workshop.models.generation.adopted' : 'workshop.models.generation.unchanged');
      }), !lodGenerationResult?.reviewReady || lodGenerationResult?.sidecar !== selected);
      const cancel = action('workshop-model-generation-cancel', 'workshop.models.generation.cancel', () => run(async () => {
        await provider.lodGeneration.cancel(); lodGenerationResult = null; show('workshop.models.generation.cancelled');
      }), lodGenerationResult?.sidecar !== selected);
      generation.append(node('label', 'workshop.models.generation.remesh', { for: remesh.id }), remesh,
        start, poll, review, adopt, cancel, progress);
    }
    for (const control of structure.querySelectorAll('button,input,select')) control.disabled = busy() || control.dataset.boundary === 'true';
  }
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
      if (previousDraft && current !== previousDraft && provider?.billboardCapture) {
        billboardResult = null; provider.billboardCapture.cancel().catch(() => {});
      }
      if (previousDraft && current !== previousDraft && provider?.lodGeneration) {
        lodGenerationResult = null; provider.lodGeneration.cancel().catch(() => {});
      }
      const old = model.value;
      const entries = modelDocuments(current?.paths() || []);
      model.replaceChildren(...entries.map(entry => option(entry.model, entry.model)));
      if (entries.some(entry => entry.model === old)) model.value = old;
      const chosen = subject.value;
      subject.replaceChildren(option('', t('workshop.models.preview_subject_model')),
        ...entityDocuments(current?.paths() || []).map(path => option(path, path)));
      if ([...subject.options].some(item => item.value === chosen)) subject.value = chosen;
      previousDraft = current; previousPaths = paths; refreshVariants();
    }
    const held = busy();
    model.disabled = variant.disabled = newName.disabled = held || !model.value;
    inspect.disabled = held || !variant.value;
    clone.disabled = held || !model.value;
    renameVariant.disabled = held || !variant.value;
    removeVariant.disabled = held || !variant.value;
    const fresh = snapshot && snapshot.draft === current && snapshot.path === variant.value
      && snapshot.source === current?.read(snapshot.path);
    apply.disabled = held || !fresh || !rows.length;
    for (const row of rows) row.input.disabled = held || !fresh;
    if (snapshot && !fresh) show('workshop.inspector_stale');
    if (!current || !model.value) show('workshop.models.empty');
    if (!dependencyLoad && variant.value && (!structureSnapshot || structureSnapshot.draft !== current
      || structureSnapshot.path !== variant.value || structureSnapshot.source !== current?.read(variant.value))) renderStructure();
    for (const control of structure.querySelectorAll('button,input,select')) control.disabled = held || Boolean(dependencyLoad)
      || control.dataset.boundary === 'true';
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
  clone.addEventListener('click', async () => {
    if (busy() || clone.disabled) return;
    const name = newName.value;
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(name)) { show('workshop.models.invalid_variant', true); return; }
    if (modelDocuments(draft().paths()).find(entry => entry.model === model.value)?.variants.some(item => item.name === name)) {
      show('workshop.models.variant_exists', true); return;
    }
    await commitStructure({ type: 'variant-add', path: variant.value || null, model: model.value, name,
      clone: Boolean(variant.value) });
    newName.value = ''; inspect.focus();
  });
  renameVariant.addEventListener('click', async () => {
    if (busy() || renameVariant.disabled) return;
    const name = newName.value;
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(name)) { show('workshop.models.invalid_variant', true); return; }
    await commitStructure({ type: 'variant-rename', path: variant.value, model: model.value, name });
    newName.value = ''; inspect.focus();
  });
  removeVariant.addEventListener('click', () => commitStructure({ type: 'variant-remove', path: variant.value, model: model.value }));
  const cancelModelToolsSelection = () => {
    if (billboardResult && provider?.billboardCapture) {
      billboardResult = null; provider.billboardCapture.cancel().catch(() => {});
    }
    if (lodGenerationResult && provider?.lodGeneration) {
      lodGenerationResult = null; provider.lodGeneration.cancel().catch(() => {});
    }
  };
  model.addEventListener('change', () => { cancelModelToolsSelection(); refreshVariants(); renderStructure(); refresh(); });
  subject.addEventListener('change', () => refresh());
  variant.addEventListener('change', () => { cancelModelToolsSelection(); renderStructure(); refresh(); });
  if (typeof runtime.dependencies === 'function') loadDependencies();
  refresh();
  if (typeof runtime.dependencies !== 'function') renderStructure();
  const previewSelection = () => {
      // An entity subject is previewed as a composed template; anything else
      // falls back to the model form's own selection, which is what the panel
      // was doing before entity subjects existed.
      if (subject.value.startsWith('assets/entities/')) return { entity: subject.value };
      return { model: model.value,
        variant: modelDocuments(draft()?.paths() || []).find(entry => entry.model === model.value)?.variants
          .find(entry => entry.path === variant.value)?.name || null };
    };
  preview = mountWorkshopModelPreview({ root: section, attach, provider, draft, busy,
    selection: previewSelection });
  return { refresh, node: section, previewNode: preview.node,
    applyLaunch(selection, controls = {}) {
      if (!selection) return true;
      refresh();
      if (selection.entity) {
        if (![...subject.options].some(item => item.value === selection.entity)) return false;
        subject.value = selection.entity;
      } else if (selection.model) {
        if (![...model.options].some(item => item.value === selection.model)) return false;
        model.value = selection.model; refreshVariants();
        if (selection.variant) {
          const entry = modelDocuments(draft()?.paths() || []).find(item => item.model === selection.model);
          const chosen = entry?.variants.find(item => item.name === selection.variant);
          if (!chosen) return false;
          variant.value = chosen.path;
        }
        subject.value = '';
      }
      renderStructure(); refresh();
      preview.applyLaunch?.(controls);
      void preview.review(draft(), previewSelection());
      return true;
    },
    setPreviewVisible(value) {
      if (previewVisible === value) return;
      previewVisible = value; refresh();
    },
    dispose() {
      disposed = true;
      provider?.billboardCapture?.cancel().catch(() => {});
      provider?.lodGeneration?.cancel().catch(() => {});
      preview.dispose();
      section.remove();
    } };
}
