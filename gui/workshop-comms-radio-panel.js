import { t } from './strings.js';
import { inspectCommsRadio, applyCommsRadioOperation } from '../editor/workshop-comms-radio.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

export function mountWorkshopCommsRadio({ root, provider, runtime, draft: getDraft, busy = () => false, setBusy = () => {},
  changed = () => {}, preview = () => false, test = () => false, attach = true }) {
  const doc = root.ownerDocument, prefix = 'workshop-comms-radio-';
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = prefix + id; if (key) node.textContent = t('workshop.comms_radio.' + key); return node; };
  const label = (control, key) => { const node = el('label', null, key); node.htmlFor = control.id; node.append(control); return node; };
  const input = (id, value, type = 'text') => { const node = el('input', id); node.type = type; node.value = value; if (type === 'number') node.step = 'any'; return node; };
  const button = (id, key, action) => { const node = el('button', id, key); node.type = 'button'; node.addEventListener('click', action); return node; };
  const node = el('section', 'panel'), template = el('select', 'template'), materialise = input('materialise', '', 'checkbox');
  const requestReading = input('request-reading', '', 'number'); requestReading.min = '1'; requestReading.max = '4294967295'; requestReading.step = '1';
  const media = el('select', 'media');
  for (const [value, key] of [['', 'media_none'], ['audio', 'media_audio'], ['audio_video', 'media_video']]) {
    const option = el('option', null, key); option.value = value; media.append(option);
  }
  const channels = el('div', 'channels'), keys = el('div', 'keys'), status = el('p', 'status'), owner = el('p', 'owner'); status.setAttribute('role', 'status');
  let dependencies = null, inspection = null, loading = false, disposed = false, generation = 0, observed = null, observedDraft = null, serial = 0;
  let channelRows = [], keyRows = [];
  function controls() {
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = disposed || loading || busy() || !inspection?.capable;
    template.disabled = disposed || loading || busy() || !dependencies;
  }
  function addRow(kind, value) {
    const section = el('fieldset'), index = serial++, rows = kind === 'channel' ? channelRows : keyRows;
    const fields = kind === 'channel' ? ['id', 'range', 'work_units', 'key', 'relay'] : ['identity'];
    const row = { section, fields: {} }; rows.push(row);
    for (const field of fields) {
      const numeric = ['range', 'work_units'].includes(field);
      const control = input(kind + '-' + field + '-' + index, value[field] ?? '', numeric ? 'number' : field === 'relay' ? 'checkbox' : 'text');
      if (field === 'relay') control.checked = value.relay === true;
      if (numeric) control.min = field === 'range' ? '0' : '1'; if (field === 'work_units') { control.step = '1'; control.max = '4294967295'; }
      row.fields[field] = control; section.append(label(control, field));
    }
    section.append(button(null, 'remove', () => { rows.splice(rows.indexOf(row), 1); section.remove(); }));
    (kind === 'channel' ? channels : keys).append(section); controls();
  }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    const selected = paths.includes(template.value) ? template.value : paths[0] || '';
    if ([...template.options].map(option => option.value).join('\n') !== paths.join('\n')) template.replaceChildren(...paths.map(path => { const option = el('option'); option.value = option.textContent = path; return option; }));
    template.value = selected;
    const key = JSON.stringify([draft.sourceRevision, selected]); if (!force && observedDraft === draft && observed === key) { controls(); return; }
    try {
      inspection = inspectCommsRadio(draft, dependencies, selected); observed = key; observedDraft = draft; materialise.checked = false;
      channels.replaceChildren(); keys.replaceChildren(); channelRows = []; keyRows = [];
      requestReading.value = inspection.radio.request_reading_ticks ?? '';
      media.value = inspection.radio.media ?? '';
      for (const value of inspection.radio.channel) addRow('channel', value);
      for (const identity of inspection.radio.keys || []) addRow('key', { identity });
      owner.textContent = t('workshop.comms_radio.owner', { system: inspection.radio.system, range: inspection.endpointRange ?? '—' });
      status.textContent = inspection.capable ? '' : t('workshop.comms_radio.unavailable');
    } catch (error) { inspection = null; status.textContent = t('workshop.comms_radio.refused', { reason: error.message }); }
    controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.comms_radio.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  const save = button('save', 'save', () => {
    if (!inspection?.capable || busy()) return;
    const value = { system: 'comms', ...(media.value ? {media: media.value} : {}), ...(requestReading.value.trim() ? { request_reading_ticks: requestReading.valueAsNumber } : {}), keys: keyRows.map(row => row.fields.identity.value), channel: channelRows.map(row => ({
      id: row.fields.id.value, range: row.fields.range.valueAsNumber, work_units: row.fields.work_units.valueAsNumber,
      ...(row.fields.key.value ? { key: row.fields.key.value } : {}),
      ...(row.fields.relay.checked ? { relay: true } : {}),
    })) };
    const capturedDraft = getDraft(), token = ++generation, operation = { path: template.value, materialise: materialise.checked, value };
    const current = () => !disposed && token === generation && getDraft() === capturedDraft;
    void runWorkshopMutation({ setBusy, current, start: controls,
      invoke: () => applyCommsRadioOperation({ draft: capturedDraft, provider, runtime, dependencies, operation, current }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.comms_radio.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  });
  const show = button('preview', null, () => { if (!busy() && inspection) preview(template.value); }); show.textContent = t('workshop.entity.preview');
  const fly = button('test', null, () => { if (!busy() && inspection) test(template.value); }); fly.textContent = t('workshop.entity.test');
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), owner, label(requestReading, 'request_reading_ticks'), label(media, 'media'), label(materialise, 'materialise'),
    el('h4', null, 'channels'), channels, button('add-channel', 'add_channel', () => addRow('channel', { id: '', range: inspection?.endpointRange ?? 1, work_units: 1 })),
    el('h4', null, 'keys'), keys, button('add-key', 'add_key', () => addRow('key', { identity: '' })), save, show, fly, status);
  template.addEventListener('change', () => inspect(true)); if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; node.remove(); } };
}
