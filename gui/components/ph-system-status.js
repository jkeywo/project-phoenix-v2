import {mountEmissionAdvisory} from './emission-profile-controls.js';
import {emissionAdvisoryPayload} from '../emission-profile.js';
import {mountIdentityControls} from './identity-controls.js';
import {mountConcealmentControls} from './concealment-controls.js';
import { mountProtectionControls } from './protection-controls.js';
import { mountBayControls } from './bay-controls.js';
import { mountJammingControls } from './jamming-controls.js';
import { mountIlluminationControls } from './illumination-controls.js';
import { mountGuidanceControls } from './guidance-controls.js';
/** Shared equipment indicators and permitted fitting reports. No simulation state. */
import { t, wireText } from '../strings.js';
import { createFocusTrap } from '../focus-trap.js';
import { mountFittingControls } from './fitting-controls.js';
import { mountFittingPlans } from './fitting-plan-controls.js';
import { mountEquipmentRecoveryControls } from './equipment-recovery-controls.js';

const icon = row => row.reasons.length > 1 ? '✱' : ({ damaged: '◆', disabled: '×', destroyed: '×', offline: '○', unpowered: 'ϟ', no_dock_markers: '◇' }[row.reasons[0]?.code] || '●');
export function statusLabel(row) {
  return `${wireText(row.display_name || row.system_id)}: ${t('system.status.' + row.availability)}`;
}

export function mountSystemStatus(document) {
  const root = document.createElement('details'); root.className = 'system-status-strip';
  root.setAttribute('aria-label', t('system.status.equipment'));
  const heading = document.createElement('summary'); heading.textContent = t('system.status.equipment'); root.append(heading);
  const style = document.createElement('style');
  style.append(document.createTextNode(`.system-status-strip {padding:.35rem;background:var(--surface-panel);position:fixed;bottom:.3rem;right:.3rem;max-width:95%;max-height:40vh;overflow:auto;z-index:20}
.system-status-strip summary{cursor:pointer;min-height:var(--control-hit-min);padding:.35rem}.system-status-strip[hidden],.system-status-dialog[hidden]{display:none}
.system-status-strip button,.system-status-dialog button {font:inherit;color:var(--ink);background:transparent;border:1px solid var(--edge-faint);min-height:var(--control-hit-min);padding:.25rem .5rem}
.system-status-dialog label,.fitting-plan-controls label{display:flex;gap:.4em;flex-wrap:wrap;align-items:center}.system-status-dialog select,.fitting-plan-controls input,.fitting-plan-controls select,.fitting-plan-controls textarea{font:inherit;color:var(--ink);background:var(--surface-panel);min-height:var(--control-hit-min);max-width:100%;box-sizing:border-box}.fitting-plan-controls textarea{min-height:6rem;width:100%}.system-status-light{margin-right:.4em}.system-status-light[data-availability=normal]{color:var(--loaded)}.system-status-light[data-availability=degraded]{color:var(--reloading)}.system-status-light[data-availability=unavailable]{color:var(--fire)}.system-status-light[data-availability=unknown]{color:var(--ink-dim)}
.system-status-dialog{position:fixed;inset:10%;overflow:auto;padding:1rem;z-index:1000;background:var(--surface-panel);color:var(--ink);border:1px solid var(--edge-faint)}`));
  root.append(style);
  const list = document.createElement('div'); root.append(list);
  const plans = mountFittingPlans(document); root.append(plans.node);
  const summary = document.createElement('button'); summary.type = 'button'; summary.dataset.delegatedSummary = ''; root.append(summary);
  const dialog = document.createElement('section'); dialog.className = 'system-status-dialog'; dialog.hidden = true;
  dialog.setAttribute('role', 'dialog'); dialog.setAttribute('aria-modal', 'true'); dialog.setAttribute('aria-label', t('system.status.details'));
  const closeButton = document.createElement('button'); closeButton.type = 'button'; closeButton.textContent = t('system.status.close');
  const detail = document.createElement('div'); detail.setAttribute('aria-live', 'polite'); dialog.append(closeButton, detail);
  document.body.append(root, dialog);
  const delegatedSelection = Symbol('delegated equipment');
  let rows = [], selected = null;
  const detailRows = new Map();
  const buttons = new Map();
  const close = () => { dialog.hidden = true; selected = null; trap.release(); };
  const trap = createFocusTrap(dialog, { doc: document, onEscape: close, initialFocus: 'button' });
  function paintDetails() {
    const chosen = selected === delegatedSelection ? rows.filter(row => row.delegated) : rows.filter(row => row.system_id === selected);
    if (!chosen.length) { close(); return; }
    const seen = new Set();
    for (const row of chosen) {
      seen.add(row.system_id);
      let entry = detailRows.get(row.system_id);
      if (!entry) {
        const node = document.createElement('section'), heading = document.createElement('h3'), facts = document.createElement('div');
        const emission = mountEmissionAdvisory(document), fitting = mountFittingControls(document), recovery = mountEquipmentRecoveryControls(document), guidance = mountGuidanceControls(document), illumination = mountIlluminationControls(document), jamming = mountJammingControls(document), protection = mountProtectionControls(document), identity = mountIdentityControls(document), concealment = mountConcealmentControls(document), bay = mountBayControls(document); node.append(heading, facts, emission.node, fitting.node, recovery.node, guidance.node, illumination.node, jamming.node, protection.node, identity.node, concealment.node, bay.node); detail.append(node);
        entry = { node, heading, facts, emission, fitting, recovery, guidance, illumination, jamming, protection, identity, concealment, bay, signature: null }; detailRows.set(row.system_id, entry);
      }
      const label = statusLabel(row); if (entry.heading.textContent !== label) entry.heading.textContent = label;
      const signature = JSON.stringify([row.reasons, row.physical]);
      if (entry.signature !== signature) {
        entry.signature = signature; entry.facts.replaceChildren();
        for (const reason of row.reasons) {
          const line = document.createElement('p'); line.textContent = t('system.status.reason.' + reason.code)
            + (reason.component_id ? ` (${reason.component_id})` : ''); entry.facts.append(line);
        }
        if (row.physical) {
          const physical = row.physical, room = document.createElement('p');
          room.textContent = t('damage.status.structure', { room: physical.compartment, current: Math.round(physical.structural_hp), max: Math.round(physical.structural_max_hp) });
          entry.facts.append(room);
          for (const component of physical.components || []) {
            const line = document.createElement('p'); line.textContent = `${component.id} · ${t('damage.status.' + component.condition.kind)}`; entry.facts.append(line);
          }
        }
      }
      entry.emission.update(emissionAdvisoryPayload(row.emission));
      entry.fitting.update(row.system_id, row.fitting || [], row.delegated || selected === delegatedSelection);
      entry.recovery.update(row.system_id, row.equipment_recovery, row.delegated || selected === delegatedSelection);
      entry.guidance.update(row.system_id, row.guidance, row.delegated || selected === delegatedSelection);
      entry.illumination.update(row.system_id, row.illumination, row.delegated || selected === delegatedSelection);
      entry.jamming.update(row.system_id, row.jamming, row.delegated || selected === delegatedSelection);
      entry.protection.update(row.system_id, row.signal_protection, row.delegated || selected === delegatedSelection);
      entry.identity.update(row.system_id,row.identity,row.delegated||selected===delegatedSelection);
      entry.concealment.update(row.system_id,row.concealment,row.delegated||selected===delegatedSelection);
      entry.bay.update(row.system_id, row.bay, row.delegated || selected === delegatedSelection);
    }
    for (const [id, entry] of detailRows) if (!seen.has(id)) { entry.emission.dispose(); entry.fitting.dispose(); entry.recovery.dispose(); entry.guidance.dispose(); entry.illumination.dispose(); entry.jamming.dispose(); entry.protection.dispose(); entry.identity.dispose(); entry.concealment.dispose(); entry.bay.dispose(); entry.node.remove(); detailRows.delete(id); }
  }

  function open(id) { selected = id; paintDetails(); if (selected === null) return; dialog.hidden = false; trap.activate(); }
  summary.addEventListener('click', () => open(delegatedSelection));
  closeButton.addEventListener('click', close);
  return {
    root,
    update(next, { station = null } = {}) {
      rows = (Array.isArray(next) ? next : []).map(row => ({ ...row, reasons: row.reasons || [] }));
      plans.update(rows, station);
      if (!root.isConnected) document.body.append(root);
      if (!dialog.isConnected) document.body.append(dialog);
      const visible = rows.filter(row => !row.delegated);
      const ids = new Set(visible.map(row => row.system_id));
      for (const [id, button] of buttons) if (!ids.has(id)) { button.remove(); buttons.delete(id); }
      for (const [index, row] of visible.entries()) {
        let button = buttons.get(row.system_id);
        if (!button) {
          button = document.createElement('button'); button.type = 'button'; button.dataset.systemId = row.system_id;
          button.setAttribute('aria-haspopup', 'dialog');
          const light = document.createElement('span'); light.className = 'system-status-light'; light.setAttribute('aria-hidden', 'true');
          const label = document.createElement('span'); button.append(light, label);
          button.addEventListener('click', () => open(row.system_id)); buttons.set(row.system_id, button);
        }
        const measured = emissionAdvisoryPayload(row.emission);
        const advice = measured?.high ? t('emission.high', {rate: measured.rate, cause: t('emission.cause.' + measured.cause)}) : '';
        button.dataset.emissionHigh = String(!!advice);
        button.setAttribute('aria-label', statusLabel(row) + (advice ? '; ' + advice : '')); button.title = statusLabel(row) + (advice ? '; ' + advice : '');
        button.firstChild.dataset.availability = row.availability;
        if (button.firstChild.textContent !== icon(row)) button.firstChild.textContent = icon(row);
        const label = (advice ? '⚑ ' : '') + wireText(row.display_name || row.system_id);
        if (button.lastChild.textContent !== label) button.lastChild.textContent = label;
        // Do not detach an unchanged focused control on every snapshot.
        if (list.children[index] !== button) list.insertBefore(button, list.children[index] || null);
      }
      const delegated = rows.filter(row => row.delegated);
      summary.hidden = !delegated.length;
      const summaryText = t('system.status.delegated', { count: delegated.length, affected: delegated.filter(row => ['degraded', 'unavailable'].includes(row.availability)).length });
      if (summary.textContent !== summaryText) summary.textContent = summaryText;
      root.hidden = !rows.length;
      if (selected !== null) paintDetails();
    },
    dispose() { close(); plans.dispose(); for (const entry of detailRows.values()) { entry.emission.dispose(); entry.fitting.dispose(); entry.recovery.dispose(); entry.guidance.dispose(); entry.illumination.dispose(); entry.jamming.dispose(); entry.protection.dispose(); entry.identity.dispose(); entry.concealment.dispose(); entry.bay.dispose(); } detailRows.clear(); root.remove(); dialog.remove(); },
  };
}
