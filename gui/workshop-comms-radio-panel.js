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
  const listeningEnabled = input('listening-enabled', '', 'checkbox'), coverage = input('coverage', '1', 'number');
  coverage.min = '1'; coverage.max = '65535'; coverage.step = '1';
  const priority = el('textarea','priority');
  const alertEnabled = input('alert-enabled', '', 'checkbox'), alertSource = input('alert-source','captain'), alertChannel = input('alert-channel','public');
  const alertSpacing = input('alert-spacing','90','number'); alertSpacing.min = '1'; alertSpacing.max = '4294967295'; alertSpacing.step = '1';
  const routineText = el('textarea','routine-text'), redText = el('textarea','red-text');
  const captureEnabled = input('capture-enabled', '', 'checkbox'), captureLevel = input('capture-level', '', 'number');
  const captureTicks = input('capture-ticks', '', 'number'), keyTicks = input('capture-key-ticks', '', 'number');
  captureLevel.min = '0'; captureLevel.max = '3'; captureLevel.step = '1';
  for (const control of [captureTicks, keyTicks]) { control.min = '1'; control.max = '36000'; control.step = '1'; }
  for (const [value, key] of [['', 'media_none'], ['audio', 'media_audio'], ['audio_video', 'media_video']]) {
    const option = el('option', null, key); option.value = value; media.append(option);
  }
  const channels = el('div', 'channels'), keys = el('div', 'keys'), status = el('p', 'status'), owner = el('p', 'owner'); status.setAttribute('role', 'status');
  let dependencies = null, inspection = null, loading = false, disposed = false, generation = 0, observed = null, observedDraft = null, serial = 0;
  let channelRows = [], keyRows = [];
  function controls() {
    for (const control of node.querySelectorAll('input,select,textarea,button')) control.disabled = disposed || loading || busy() || !inspection?.capable;
    template.disabled = disposed || loading || busy() || !dependencies;
  }
  function addRow(kind, value) {
    const section = el('fieldset'), index = serial++, rows = kind === 'channel' ? channelRows : keyRows;
    const fields = kind === 'channel' ? ['id', 'range', 'work_units', 'key', 'relay', 'protection_level'] : ['identity'];
    const row = { section, fields: {} }; rows.push(row);
    for (const field of fields) {
      const numeric = ['range', 'work_units', 'protection_level'].includes(field);
      const control = input(kind + '-' + field + '-' + index, value[field] ?? '', numeric ? 'number' : field === 'relay' ? 'checkbox' : 'text');
      if (field === 'relay') control.checked = value.relay === true;
      if (numeric) control.min = field === 'range' ? '0' : '1'; if (field === 'work_units') { control.step = '1'; control.max = '4294967295'; }
      if (field === 'protection_level') { control.step = '1'; control.max = '3'; }
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
      captureEnabled.checked = inspection.radio.capture !== undefined;
      captureLevel.value = inspection.radio.capture?.max_level ?? '1';
      captureTicks.value = inspection.radio.capture?.ticks_per_level ?? '90';
      keyTicks.value = inspection.radio.capture?.held_key_ticks ?? '';
      media.value = inspection.radio.media ?? '';
      listeningEnabled.checked = inspection.radio.traffic?.listening !== undefined;
      coverage.value = inspection.radio.traffic?.listening?.simultaneous_channels ?? '1';
      priority.value = (inspection.radio.traffic?.listening?.priority ?? inspection.radio.channel.map(channel => channel.id)).join('\n');
      alertEnabled.checked = inspection.radio.traffic?.alert !== undefined;
      alertSource.value = inspection.radio.traffic?.alert?.source ?? 'captain';
      alertChannel.value = inspection.radio.traffic?.alert?.channel ?? 'public';
      alertSpacing.value = inspection.radio.traffic?.alert?.minimum_spacing_ticks ?? '90';
      routineText.value = inspection.radio.traffic?.alert?.routine_text ?? '';
      redText.value = inspection.radio.traffic?.alert?.red_alert_text ?? '';
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
    if (disposed || loading || !inspection?.capable || busy()) return;
    const value = { system: 'comms', ...(media.value ? {media: media.value} : {}),
      ...(listeningEnabled.checked || alertEnabled.checked ? {traffic:{
        ...(listeningEnabled.checked ? {listening:{simultaneous_channels:coverage.valueAsNumber,priority:priority.value.split(/\s+/u).filter(Boolean)}} : {}),
        ...(alertEnabled.checked ? {alert:{source:alertSource.value,channel:alertChannel.value,minimum_spacing_ticks:alertSpacing.valueAsNumber,
          routine_text:routineText.value,red_alert_text:redText.value}} : {}),
      }} : {}),
      ...(captureEnabled.checked ? { capture: { max_level: captureLevel.valueAsNumber, ticks_per_level: captureTicks.valueAsNumber,
        ...(keyTicks.value.trim() ? { held_key_ticks: keyTicks.valueAsNumber } : {}) } } : {}),
      ...(requestReading.value.trim() ? { request_reading_ticks: requestReading.valueAsNumber } : {}), keys: keyRows.map(row => row.fields.identity.value), channel: channelRows.map(row => ({
      id: row.fields.id.value, range: row.fields.range.valueAsNumber, work_units: row.fields.work_units.valueAsNumber,
      ...(row.fields.key.value ? { key: row.fields.key.value } : {}),
      ...(row.fields.relay.checked ? { relay: true } : {}),
      ...(row.fields.protection_level.value.trim() ? { protection_level: row.fields.protection_level.valueAsNumber } : {}),
    })) };
    const capturedDraft = getDraft(), token = ++generation, operation = { path: template.value, materialise: materialise.checked, value };
    const current = () => !disposed && token === generation && getDraft() === capturedDraft;
    void runWorkshopMutation({ setBusy, current, start: controls,
      invoke: () => applyCommsRadioOperation({ draft: capturedDraft, provider, runtime, dependencies, operation, current }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.comms_radio.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  });
  const show = button('preview', null, () => { if (!disposed && !loading && !busy() && inspection) preview(template.value); }); show.textContent = t('workshop.entity.preview');
  const fly = button('test', null, () => { if (!disposed && !loading && !busy() && inspection) test(template.value); }); fly.textContent = t('workshop.entity.test');
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), owner, label(requestReading, 'request_reading_ticks'), label(media, 'media'),
    label(captureEnabled, 'capture_enabled'), label(captureLevel, 'capture_level'), label(captureTicks, 'capture_ticks'), label(keyTicks, 'capture_key_ticks'), label(materialise, 'materialise'),
    label(listeningEnabled,'listening_enabled'), label(coverage,'coverage'), label(priority,'priority'),
    label(alertEnabled,'alert_enabled'), label(alertSource,'alert_source'), label(alertChannel,'alert_channel'), label(alertSpacing,'alert_spacing'),
    label(routineText,'routine_text'), label(redText,'red_text'),
    el('h4', null, 'channels'), channels, button('add-channel', 'add_channel', () => addRow('channel', { id: '', range: inspection?.endpointRange ?? 1, work_units: 1 })),
    el('h4', null, 'keys'), keys, button('add-key', 'add_key', () => addRow('key', { identity: '' })), save, show, fly, status);
  template.addEventListener('change', () => inspect(true)); if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { if (disposed) return; node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; controls(); node.remove(); } };
}
