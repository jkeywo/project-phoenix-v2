import { createActionCorrelation, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS } from './action-feedback.js';
import { wireText } from './strings.js';

/** Shared native/browser GM controls; all mutation goes through the typed lane. */
export function createGmPresentationPanel({ doc = globalThis.document, t = id => id,
  getOperator = () => null, submit = () => false, correlation = createActionCorrelation,
  schedule = globalThis.setTimeout, cancelSchedule = globalThis.clearTimeout } = {}) {
  const root = doc.createElement('fieldset'); root.id = 'gm-presentation-panel';
  const legend = doc.createElement('legend'); legend.textContent = t('server.gm.presentation.title'); root.append(legend);
  const input = (name, tag = 'input') => {
    const label = doc.createElement('label'), field = doc.createElement(tag);
    label.textContent = t(`server.gm.presentation.${name}`); field.id = `gm-presentation-${name}`;
    label.append(field); root.append(label); return field;
  };
  const ship = input('ship', 'select'), mode = input('view', 'select');
  for (const value of ['camera', 'radar', 'sensors_radar', 'navigation_chart', 'cinematic']) {
    const opt = doc.createElement('option'); opt.value = value; opt.textContent = t(`server.gm.presentation.view_${value}`); mode.append(opt);
  }
  const camera = input('camera', 'select');
  const duration = input('duration'); duration.type = 'number'; duration.min = '1'; duration.max = '4294967295'; duration.step = '1';
  const title = input('heading'); title.maxLength = 4096;
  const subtitle = input('body', 'textarea'); subtitle.maxLength = 4096;
  const message = input('message', 'select');
  const sound = input('sound', 'select'), soundSource = input('sound_source', 'select');
  const current = doc.createElement('p'); current.className = 'gm-presentation-current'; root.append(current);
  const status = doc.createElement('p'); status.setAttribute('role', 'status');
  let ships = [], pending = null, timer = null, state = {}, listKey = '', messages = [], messageKey = '', cameras = {}, cameraKey = '';
  let sounds = [], soundKey = '', sources = [], sourceKey = '';
  const buttons = [];
  function feedback(value) { status.textContent = t(`server.gm.presentation.${value}`); status.dataset.state = value; }
  function replaceChoices(field, choices) {
    const old = field.value, oldLabel = field.selectedOptions[0]?.textContent || old;
    field.replaceChildren();
    for (const [value, label] of choices) {
      const option = doc.createElement('option'); option.value = value; option.textContent = label; field.append(option);
    }
    if (!old) return;
    if (!choices.some(([value]) => value === old)) {
      const missing = doc.createElement('option'); missing.value = old; missing.textContent = oldLabel;
      missing.disabled = true; field.prepend(missing);
    }
    field.value = old;
  }
  function refreshAdmission() {
    for (const button of buttons) button.disabled = !getOperator() || !!pending || !ships.some(row => row.entity_id === ship.value);
    const cameraChoices = cameras[ship.value] || [];
    const cameraListKey = JSON.stringify(cameraChoices);
    if (cameraListKey !== cameraKey) {
      replaceChoices(camera, cameraChoices.map(name => [name, name]));
      cameraKey = cameraListKey;
    }
    const choices = messages.filter(row => row.ship === null || row.ship === ship.value);
    const nextSounds = JSON.stringify(sounds), nextSources = JSON.stringify(sources);
    if (nextSounds !== soundKey) { replaceChoices(sound, sounds.map(id => [id, id])); soundKey = nextSounds; }
    if (nextSources !== sourceKey) { replaceChoices(soundSource, [['', t('server.gm.presentation.sound_static')], ...sources]); sourceKey = nextSources; }
    const key = JSON.stringify(choices);
    if (key !== messageKey) {
      replaceChoices(message, choices.map(row => [row.message, `${wireText(row.sender)} (${row.message})`]));
      messageKey = key;
    }
    const shown = state[ship.value];
    const view = shown?.forced_view;
    const viewName = view && (typeof view.view === 'string' ? t(`server.gm.presentation.view_${view.view}`) : view.view.camera);
    current.textContent = [view ? t('server.gm.presentation.current_view', { view: viewName, tick: view.until_tick }) : t('server.gm.presentation.no_forced_view'),
      shown?.card ? t('server.gm.presentation.current_card', { tick: shown.card.until_tick }) : t('server.gm.presentation.no_card')].join(' · ');
  }
  function send(cue) {
    const operator = getOperator();
    if (!operator || pending || !ships.some(row => row.entity_id === ship.value)) return false;
    const request = { operator_id: operator.id, correlation: correlation(), ship: ship.value, cue };
    let accepted = false;
    try { accepted = submit(request) !== false; } catch { /* surfaced below */ }
    if (!accepted) { feedback('refused'); return true; }
    pending = request; feedback('pending'); refreshAdmission();
    timer = schedule(() => { pending = null; timer = null; feedback('timed_out'); refreshAdmission(); }, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS);
    return true;
  }
  const ticks = () => { const n = Number(duration.value); return Number.isInteger(n) && n > 0 && n <= 4294967295 ? n : null; };
  const button = (name, action) => {
    const el = doc.createElement('button'); el.type = 'button'; el.textContent = t(`server.gm.presentation.${name}`);
    el.addEventListener('click', () => { if (!action()) feedback('invalid'); }); root.append(el); buttons.push(el);
  };
  button('force', () => ticks() && (mode.value !== 'camera' || (cameras[ship.value] || []).includes(camera.value)) && send({ force_view: { view: mode.value === 'camera' ? { camera: camera.value } : mode.value, duration_ticks: ticks() } }));
  button('release', () => send('release_view'));
  button('show_title', () => ticks() && title.value.trim() && send({ title_card: { title: title.value, subtitle: subtitle.value, duration_ticks: ticks() } }));
  button('incoming', () => ticks() && messages.some(row => row.message === message.value && (row.ship === null || row.ship === ship.value)) && send({ incoming_comms: { message: message.value, duration_ticks: ticks() } }));
  button('clear', () => send('clear_card'));
  button('play_sound', () => sounds.includes(sound.value) && (!soundSource.value || sources.some(([id]) => id === soundSource.value))
    && send({ sound: { id: sound.value, source: soundSource.value || null } }));
  root.append(status);
  // The docked host when the GM desk composed one (issue #1505), and the mission
  // panel it has always lived in otherwise.
  (doc.getElementById('gm-presentation-dock') || doc.getElementById('gm-mission-panel'))?.append(root);
  ship.addEventListener('change', () => {
    camera.value = ''; message.value = ''; cameraKey = ''; messageKey = ''; refreshAdmission();
  });
  function update(payload) {
    let p = payload;
    if (typeof p === 'string') { try { p = JSON.parse(p); } catch { return false; } }
    if (!p || !Array.isArray(p.entities)) return false;
    ships = p.entities.filter(row => row.kind === 'player_ship'); state = p.presentation || {}; messages = p.presentation_messages || []; cameras = p.presentation_cameras || {};
    sounds = p.presentation_sounds || []; sources = p.entities.map(row => [row.entity_id, wireText(row.name)]);
    const key = JSON.stringify(ships.map(row => [row.entity_id, row.name]));
    if (key !== listKey) {
      replaceChoices(ship, ships.map(row => [row.entity_id, wireText(row.name)]));
      listKey = key;
    }
    const result = pending && (p.presentation_results || []).find(row => row.operator_id === pending.operator_id && row.correlation === pending.correlation);
    if (result && ['applied', 'no-op', 'refused'].includes(result.outcome)) {
      cancelSchedule(timer); timer = null; pending = null; feedback(result.outcome);
    }
    refreshAdmission(); return true;
  }
  function reset() { if (timer !== null) cancelSchedule(timer); timer = null; pending = null; ships = []; state = {}; messages = []; cameras = {}; sounds = []; sources = []; soundKey = ''; sourceKey = ''; listKey = ''; cameraKey = ''; messageKey = ''; ship.replaceChildren(); camera.replaceChildren(); message.replaceChildren(); sound.replaceChildren(); soundSource.replaceChildren(); feedback('ready'); refreshAdmission(); }
  function focusControls({ ship: shipId = null, field = '', value = '' } = {}) {
    if (shipId && [...ship.options].some(option => option.value === shipId)) {
      ship.value = shipId; ship.dispatchEvent(new doc.defaultView.Event('change'));
    }
    let control = duration;
    if (field.includes('views.camera') || (field.includes('force_view') && value.startsWith('camera_'))) {
      mode.value = 'camera'; camera.value = value; control = camera;
    } else if (field.includes('views.mode') || field.includes('force_view.view')) {
      if ([...mode.options].some(option => option.value === value)) mode.value = value;
      control = mode;
    } else if (field.includes('title_card.title')) control = title;
    else if (field.includes('title_card.subtitle')) control = subtitle;
    else if (field.includes('incoming_comms.message') || field.includes('comms.available')) {
      message.value = value; control = message;
    } else if (field.includes('cue.sound.id') || field === 'sound.id') {
      sound.value = value; control = sound;
    } else if (field.includes('cue.sound.source')) control = soundSource;
    control.scrollIntoView?.({ block: 'nearest' }); control.focus?.({ preventScroll: true }); return true;
  }
  refreshAdmission();
  return { update, reset, refreshAdmission, focusControls, state: () => ({ pending, presentation: state }), destroy() { reset(); root.remove(); } };
}
