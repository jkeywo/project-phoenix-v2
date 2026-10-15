import '../components/ph-port-service-controls.js';
import { familyView, familySystemId } from '../console-payload.js';
import { t, wireText } from '../strings.js';
import { UMBILICAL_RESOURCE_ACTION_ID, UMBILICAL_DIRECTION_ACTION_ID } from './engineering-actions.js';
import { activateEngineeringAction } from './engineering-action-control.js';

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
  if (!d || (!d.available && !d.engaged && !d.docked && !d.separating)) {
    dockPanel.hidden = true;
    return;
  }
  dockPanel.hidden = false;
  const docked = !!d.docked;
  if (dockBtn) {
    dockBtn.classList.toggle('docked', docked);
    dockBtn.disabled = !!d.separating;
    dockBtn.textContent = tr(docked ? 'console.dock.undock' : 'console.dock.dock');
  }
  const dockStatus = doc.getElementById('dock-status');
  if (dockStatus) {
    dockStatus.textContent = d.separating ? tr('console.dock.separating', { distance: Number.isFinite(d.separation_remaining) ? Math.round(d.separation_remaining) : '?' }) : docked
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
  let selector = panel.querySelector('[data-transfer-resource]');
  if (!selector && Array.isArray(um.allowed_resources) && um.allowed_resources.length > 0) {
    const label = doc.createElement('label'), caption = doc.createElement('span');
    caption.textContent = tr('console.umbilical.resource'); selector = doc.createElement('select');
    selector.dataset.transferResource = ''; selector.style.minHeight = '44px';
    selector.onchange = () => activateEngineeringAction({ sendAction: (...args) => doc.defaultView?.sendAction?.(...args) },
      UMBILICAL_RESOURCE_ACTION_ID, { resource: selector.value, control_system_id: selector.dataset.systemId }, 'set_transfer_resource');
    label.append(caption, selector); panel.append(label);
  }
  if (selector) {
    const choices = um.allowed_resources || [], signature = JSON.stringify(choices);
    if (selector.dataset.choices !== signature) {
      selector.replaceChildren(...choices.map(resource => { const option = doc.createElement('option'); option.value = resource;
        option.textContent = resource === 'coolant' ? tr('energy.coolant')
          : resource === 'reserve_fuel' ? tr('console.umbilical.reserve_fuel')
          : resource === 'structural_parts' ? tr('damage.resource.structural_parts')
          : resource === 'machinery_parts' ? tr('damage.resource.machinery_parts') : wireText(resource); return option; }));
      selector.dataset.choices = signature;
    }
    selector.dataset.systemId = umbilicalSystemId;
    selector.parentElement.hidden = choices.length === 0;
    selector.disabled = !!um.umbilical_auto;
    selector.value = um.selected_resource || um.capacity || '';
  }
  let port = panel.querySelector('ph-port-service-controls');
  if (!port) { port = doc.createElement('ph-port-service-controls'); panel.append(port); }
  port.state = { ...um, system_id: umbilicalSystemId };
  let direction = panel.querySelector('[data-transfer-direction]');
  if (!direction) {
    const label = doc.createElement('label'), caption = doc.createElement('span');
    caption.textContent = tr('console.umbilical.direction'); direction = doc.createElement('select');
    direction.dataset.transferDirection = ''; direction.style.minHeight = '44px';
    for (const value of ['deliver', 'collect']) {
      const option = doc.createElement('option'); option.value = value;
      option.textContent = tr(value === 'deliver' ? 'console.umbilical.deliver' : 'console.umbilical.collect'); direction.append(option);
    }
    direction.onchange = () => activateEngineeringAction({ sendAction: (...args) => doc.defaultView?.sendAction?.(...args) },
      UMBILICAL_DIRECTION_ACTION_ID, { direction: direction.value, control_system_id: direction.dataset.systemId }, 'set_transfer_direction');
    label.append(caption, direction); panel.append(label);
  }
  direction.dataset.systemId = umbilicalSystemId;
  direction.disabled = !!um.umbilical_auto || !!um.running;
  direction.value = um.direction || 'deliver';
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
      + ' · ' + tr('console.umbilical.levels') + ' ' + lvl(um.operator_level) + (um.direction === 'collect' ? ' ← ' : ' → ') + lvl(um.partner_level);
  }
  const refusal = doc.getElementById('umbilical-refusal');
  if (refusal) {
    if (um.refusal) { refusal.hidden = false; refusal.textContent = tr(um.refusal); }
    else { refusal.hidden = true; refusal.textContent = ''; }
  }
}
