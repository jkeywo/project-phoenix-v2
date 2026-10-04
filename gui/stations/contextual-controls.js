import { familyView, familySystemId } from '../console-payload.js';
import { t } from '../strings.js';

const mounted = new WeakMap();

/** Light-DOM ownership; hulls retain roots, placement and theme. */
function mountControl(root, kind, { activate, profile = 'alliance', headingMargin = false } = {}) {
  if (!root) return { dispose() {} };
  const existing = mounted.get(root);
  if (existing) { existing.activate = activate; return existing.handle; }
  const dynasty = profile === 'dynasty';
  const rowClass = dynasty ? 'dynasty-aux-row' : kind === 'dock' ? 'dock-row' : 'tractor-row';
  const separator = () => root.ownerDocument.createTextNode(dynasty ? '' : '\n');
  const controlClass = suffix => dynasty ? '' : `${kind === 'dock' ? 'dock' : 'tractor'}-${suffix}`;
  const element = (tag, { id, className, text, stringId } = {}) => {
    const node = root.ownerDocument.createElement(tag);
    if (id) node.id = id;
    if (className) node.className = className;
    if (stringId) { node.dataset.i18n = stringId; node.textContent = t(stringId); }
    else if (text != null) node.textContent = text;
    return node;
  };
  if (!root.querySelector(`#${kind === 'tow-load' ? 'tow-load-target' : kind + '-btn'}`)) {
    if (kind === 'tow-load') {
      root.append(element('span', { className: dynasty ? '' : 'tow-load-label', stringId: 'console.helm.under_tow' }), separator(),
        element('span', { id: 'tow-load-target', className: dynasty ? '' : 'tow-load-target' }));
    } else {
      if (kind !== 'dock') {
        const heading = element('h2', { stringId: `console.engineering.${kind}` });
        if (headingMargin) heading.style.marginTop = '10px';
        root.append(heading, separator());
      }
      const row = element('div', { className: rowClass });
      const button = element('button', { id: `${kind}-btn`, className: controlClass('btn'),
        stringId: `console.${kind}.${kind === 'dock' ? 'dock' : kind === 'tractor' ? 'engage' : 'start'}` });
      if (dynasty) button.type = 'button';
      row.append(button, separator(), element('span', { id: `${kind}-status`, className: controlClass('status'), text: '—' }));
      const refusal = element('div', { id: `${kind}-refusal`, className: controlClass('refusal') });
      refusal.hidden = true;
      root.append(row, separator(), refusal);
    }
  }
  const button = root.querySelector(`#${kind}-btn`);
  const record = { activate, handle: null };
  const click = () => record.activate?.(kind === 'tractor'
    ? (button.classList.contains('engaged') ? 'release_tractor' : 'engage_tractor')
    : kind === 'umbilical' ? (button.classList.contains('engaged') ? 'stop_transfer' : 'start_transfer') : undefined);
  button?.addEventListener('click', click);
  record.handle = { dispose() {
    if (mounted.get(root) !== record) return;
    button?.removeEventListener('click', click);
    mounted.delete(root);
  } };
  mounted.set(root, record);
  return record.handle;
}

export const mountDockControl = (root, options) => mountControl(root, 'dock', options);
export const mountTowLoadControl = (root, options) => mountControl(root, 'tow-load', options);
export const mountTractorControl = (root, options) => mountControl(root, 'tractor', options);
export const mountUmbilicalControl = (root, options) => mountControl(root, 'umbilical', options);

export function renderDockPanel(s, doc, tr) {
  const dockPanel = doc.getElementById('dock-panel');
  const d = s.dock || null;
  const dockBtn = doc.getElementById('dock-btn');
  if (dockBtn) dockBtn.dataset.systemId = d?.system_id || '';
  if (!dockPanel) return;
  if (!d || (!d.available && !d.engaged && !d.docked)) {
    dockPanel.hidden = true;
    return;
  }
  dockPanel.hidden = false;
  const docked = !!d.docked;
  if (dockBtn) {
    dockBtn.classList.toggle('docked', docked);
    dockBtn.textContent = tr(docked ? 'console.dock.undock' : 'console.dock.dock');
  }
  const dockStatus = doc.getElementById('dock-status');
  if (dockStatus) {
    dockStatus.textContent = docked
      ? tr('console.dock.docked') + (d.docked_to_name ? ' · ' + tr(d.docked_to_name) : '')
      : tr('console.dock.available') + (d.available_target_name ? ' · ' + tr(d.available_target_name) : '');
  }
  const dockRefusal = doc.getElementById('dock-refusal');
  if (dockRefusal) {
    if (d.refusal) { dockRefusal.hidden = false; dockRefusal.textContent = tr(d.refusal); }
    else { dockRefusal.hidden = true; dockRefusal.textContent = ''; }
  }
}

export function renderTowLoadPanel(s, doc, tr) {
  const towPanel = doc.getElementById('tow-load-panel');
  if (!towPanel) return;
  const tl = s.tow_load || null;
  if (!tl || !tl.active) {
    towPanel.hidden = true;
    return;
  }
  towPanel.hidden = false;
  const towTarget = doc.getElementById('tow-load-target');
  if (towTarget) towTarget.textContent = tl.target_name ? '· ' + tr(tl.target_name) : '';
}

export function renderTractorPanel(s, doc, tr) {
  const panel = doc.getElementById('tractor-panel');
  if (!panel) return;
  const tv = familyView(s, 'tractor');
  const tractorSystemId = tv.system_id || familySystemId(s, 'tractor');
  if (!tractorSystemId) {
    panel.hidden = true;
    return;
  }
  panel.hidden = false;
  const engaged = !!tv.engaged;
  const btn = doc.getElementById('tractor-btn');
  if (btn) {
    btn.classList.toggle('engaged', engaged);
    btn.textContent = tr(engaged ? 'console.tractor.release' : 'console.tractor.engage');
  }
  const status = doc.getElementById('tractor-status');
  if (status) {
    status.textContent = engaged
      ? tr('console.tractor.holding') + (tv.coupled_target_name ? ' · ' + tr(tv.coupled_target_name) : '')
      : tr('console.tractor.idle') + ' · ' + tr('console.tractor.range') + ' ' + Math.round(tv.range || 0);
  }
  const refusal = doc.getElementById('tractor-refusal');
  if (refusal) {
    if (tv.refusal) { refusal.hidden = false; refusal.textContent = tr(tv.refusal); }
    else { refusal.hidden = true; refusal.textContent = ''; }
  }
}

export function renderUmbilicalPanel(s, doc, tr) {
  const panel = doc.getElementById('umbilical-panel');
  if (!panel) return;
  const um = familyView(s, 'umbilical');
  const umbilicalSystemId = um.system_id || familySystemId(s, 'umbilical');
  if (!umbilicalSystemId) {
    panel.hidden = true;
    return;
  }
  panel.hidden = false;
  const running = !!um.running;
  const btn = doc.getElementById('umbilical-btn');
  if (btn) {
    btn.classList.toggle('engaged', running);
    btn.textContent = tr(running ? 'console.umbilical.stop' : 'console.umbilical.start');
  }
  const lvl = (v) => (v == null) ? '—' : Math.round(v);
  const status = doc.getElementById('umbilical-status');
  if (status) {
    status.textContent = tr(running ? 'console.umbilical.flowing' : 'console.umbilical.idle')
      + ' · ' + tr('console.umbilical.rate') + ' ' + Math.round(um.rate || 0)
      + ' · ' + tr('console.umbilical.levels') + ' ' + lvl(um.operator_level) + ' → ' + lvl(um.partner_level);
  }
  const refusal = doc.getElementById('umbilical-refusal');
  if (refusal) {
    if (um.refusal) { refusal.hidden = false; refusal.textContent = tr(um.refusal); }
    else { refusal.hidden = true; refusal.textContent = ''; }
  }
}
