import { createWorkshopModelPreview } from '../editor/workshop-model-preview.js';
import { t } from './strings.js';

/** Local renderer controls. Source edits only mark the retained picture stale;
 * Refresh is the deliberate boundary that replaces its immutable input. */
export function mountWorkshopModelPreview({ root, provider, draft, selection, busy, attach = true }) {
  const doc = root.ownerDocument;
  const make = (tag, name, text) => {
    const value = doc.createElement(tag);
    value.id = `workshop-model-preview-${name}`;
    if (text) value.textContent = t(text);
    return value;
  };
  const panel = make('section', 'panel'); panel.className = 'workshop-model-preview';
  const heading = make('h3', 'heading', 'workshop.models.preview.title');
  const controls = make('div', 'controls'); controls.className = 'workshop-toolbar';
  const viewport = make('div', 'viewport'); viewport.className = 'workshop-model-preview-viewport';
  const status = make('p', 'status'); status.setAttribute('role', 'status');
  const stats = make('p', 'stats');
  let session = null, disposed = false, hidden = false, previousDraft, previousRevision, previousSelection;
  const invoke = async fn => { try { await fn(); } catch (error) {
    if (!disposed) { status.textContent = String(error?.message || error); status.setAttribute('role', 'alert'); }
  } };
  const button = (name, text, call) => {
    const value = make('button', name, text); value.type = 'button';
    value.addEventListener('click', () => { if (!value.disabled) void invoke(call); }); controls.append(value); return value;
  };
  const refreshButton = button('refresh', 'workshop.models.preview.refresh', () => session.refresh(draft(), selection()));
  const stop = button('stop', 'workshop.models.preview.stop', () => session.stop());
  const label = (value, text) => {
    const node = doc.createElement('label'); node.htmlFor = value.id; node.textContent = t(text); controls.append(node, value);
  };
  const select = (name, text, options, control) => {
    const value = make('select', name);
    for (const [key, textId] of options) {
      const option = doc.createElement('option'); option.value = key; option.textContent = t(textId); value.append(option);
    }
    label(value, text); value.addEventListener('change', () => {
      if (!value.disabled) void invoke(() => session.control(control(value.value)));
    }); return value;
  };
  const lod = select('lod', 'workshop.models.preview.lod', [
    ['auto', 'workshop.models.preview.lod_auto'], ['base', 'workshop.models.preview.lod_base'],
  ], value => value.startsWith('fixed:') ? { command: 'lod', mode: 'fixed', level: Number(value.slice(6)) }
    : { command: 'lod', mode: value, level: null });
  const lighting = select('lighting', 'workshop.models.preview.lighting', [
    ['directional', 'workshop.models.preview.light_directional'], ['ambient', 'workshop.models.preview.light_ambient'],
    ['off', 'workshop.models.preview.light_off'],
  ], mode => ({ command: 'lighting', mode }));
  const gizmos = make('input', 'gizmos'); gizmos.type = 'checkbox'; gizmos.checked = true;
  label(gizmos, 'workshop.models.preview.gizmos');
  gizmos.addEventListener('change', () => { if (!gizmos.disabled) void invoke(() => session.control({ command: 'gizmos', enabled: gizmos.checked })); });
  const distance = make('input', 'distance'); distance.type = 'number'; distance.min = '0.1'; distance.max = '1000000'; distance.step = 'any';
  label(distance, 'workshop.models.preview.distance');
  distance.addEventListener('change', () => {
    if (!distance.disabled && distance.value.trim() && distance.checkValidity())
      void invoke(() => session.control({ command: 'distance', distance: Number(distance.value) }));
  });
  const camera = (yaw, pitch) => {
    const current = session.snapshot().status?.stats?.camera;
    if (current) return session.control({ command: 'camera', ...current, yaw: current.yaw + yaw, pitch: current.pitch + pitch });
  };
  const cameraButtons = [
    button('left', 'workshop.models.preview.left', () => camera(-Math.PI / 12, 0)),
    button('right', 'workshop.models.preview.right', () => camera(Math.PI / 12, 0)),
    button('up', 'workshop.models.preview.up', () => camera(0, Math.PI / 12)),
    button('down', 'workshop.models.preview.down', () => camera(0, -Math.PI / 12)),
  ];
  panel.append(heading, controls, status, stats, viewport);
  // See mountWorkshopModels: a docked panel is placed by the renderer alone.
  if (attach) root.append(panel);
  function render() {
    if (!session || disposed) return;
    const state = session.snapshot(), measured = state.status?.stats;
    const held = hidden || busy() || state.loading;
    refreshButton.disabled = held || !state.available || !draft() || !selection().model;
    stop.disabled = !state.running && !state.loading;
    for (const value of [lod, lighting, gizmos, distance]) value.disabled = held || !state.running || !measured?.settled;
    for (const value of cameraButtons) value.disabled = held || !state.running || !measured?.camera;
    if (measured) {
      const count = Math.min(1024, Math.max(0, measured.levels || 0));
      if (lod.options.length !== count + 2) {
        const selected = lod.value;
        while (lod.options.length > 2) lod.remove(2);
        for (let index = 0; index < count; index++) {
          const option = doc.createElement('option'); option.value = `fixed:${index}`;
          option.textContent = t('workshop.models.preview.lod_level', { level: String(index + 1) }); lod.append(option);
        }
        lod.value = selected;
      }
      if (['auto', 'base'].includes(measured.mode)) lod.value = measured.mode;
      else if (measured.mode === 'fixed' && measured.level != null) lod.value = `fixed:${measured.level}`;
      if (doc.activeElement !== distance) distance.value = String(measured.distance);
      if (typeof measured.gizmos === 'boolean') gizmos.checked = measured.gizmos;
      if (['off', 'ambient', 'directional'].includes(measured.lighting)) lighting.value = measured.lighting;
      stats.textContent = t('workshop.models.preview.stats', {
        triangles: String(measured.triangles), meshes: String(measured.meshes), textures: String(measured.textures),
        measured: String(measured.measured_textures), pixels: String(measured.texture_pixels), largest: String(measured.largest_texture),
      });
    } else stats.textContent = '';
    const lines = [t(!state.available ? 'workshop.models.preview.unavailable'
      : state.loading || state.status?.starting ? 'workshop.models.preview.loading'
        : state.running ? 'workshop.models.preview.running' : 'workshop.models.preview.idle')];
    if (state.stale) lines.push(t('workshop.models.preview.stale'));
    if (state.error) lines.push(String(state.error));
    status.textContent = lines.join(' '); status.setAttribute('role', state.error ? 'alert' : 'status');
  }
  session = createWorkshopModelPreview({ provider, mount: viewport, title: t('workshop.models.preview.title'), onChange: render,
    timer: doc.defaultView });
  function refresh({ hidden: nextHidden = false } = {}) {
    const current = draft(), revision = current?.sourceRevision, selected = JSON.stringify(selection());
    if (current !== previousDraft || revision !== previousRevision || selected !== previousSelection) {
      const replaced = previousDraft && current !== previousDraft;
      previousDraft = current; previousRevision = revision; previousSelection = selected;
      if (replaced) void invoke(() => session.stop()); else session.invalidate();
    }
    if (nextHidden && !hidden) void invoke(() => session.stop());
    hidden = nextHidden; panel.hidden = hidden; render();
  }
  refresh();
  return { refresh, node: panel, dispose() { disposed = true; void Promise.resolve(session.dispose()).catch(() => {}); panel.remove(); } };
}
