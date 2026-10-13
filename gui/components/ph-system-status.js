/** Shared read-only equipment indicators. No commands or simulation state. */
import { t, wireText } from '../strings.js';
import { createFocusTrap } from '../focus-trap.js';

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
.system-status-light{margin-right:.4em}.system-status-light[data-availability=normal]{color:var(--loaded)}.system-status-light[data-availability=degraded]{color:var(--reloading)}.system-status-light[data-availability=unavailable]{color:var(--fire)}.system-status-light[data-availability=unknown]{color:var(--ink-dim)}
.system-status-dialog{position:fixed;inset:10%;overflow:auto;padding:1rem;z-index:1000;background:var(--surface-panel);color:var(--ink);border:1px solid var(--edge-faint)}`));
  root.append(style);
  const list = document.createElement('div'); root.append(list);
  const summary = document.createElement('button'); summary.type = 'button'; summary.dataset.delegatedSummary = ''; root.append(summary);
  const dialog = document.createElement('section'); dialog.className = 'system-status-dialog'; dialog.hidden = true;
  dialog.setAttribute('role', 'dialog'); dialog.setAttribute('aria-modal', 'true'); dialog.setAttribute('aria-label', t('system.status.details'));
  const closeButton = document.createElement('button'); closeButton.type = 'button'; closeButton.textContent = t('system.status.close');
  const detail = document.createElement('div'); detail.setAttribute('aria-live', 'polite'); dialog.append(closeButton, detail);
  document.body.append(root, dialog);
  const delegatedSelection = Symbol('delegated equipment');
  let rows = [], selected = null, detailSignature = null;
  const buttons = new Map();
  const close = () => { dialog.hidden = true; selected = null; trap.release(); };
  const trap = createFocusTrap(dialog, { doc: document, onEscape: close, initialFocus: 'button' });
  function paintDetails() {
    const chosen = selected === delegatedSelection ? rows.filter(row => row.delegated) : rows.filter(row => row.system_id === selected);
    if (!chosen.length) { close(); return; }
    const content = chosen.map(row => ({
      id: row.system_id, heading: statusLabel(row),
      reasons: row.reasons.map(reason => t('system.status.reason.' + reason.code)
        + (reason.component_id ? ` (${reason.component_id})` : '')),
    }));
    const signature = JSON.stringify(content);
    if (signature === detailSignature) return;
    detailSignature = signature;
    detail.replaceChildren();
    for (const row of content) {
      const heading = document.createElement('h3'); heading.textContent = row.heading; detail.append(heading);
      for (const reason of row.reasons) {
        const line = document.createElement('p'); line.textContent = reason; detail.append(line);
      }
    }
  }
  function open(id) { selected = id; paintDetails(); if (selected === null) return; dialog.hidden = false; trap.activate(); }
  summary.addEventListener('click', () => open(delegatedSelection));
  closeButton.addEventListener('click', close);
  return {
    root,
    update(next) {
      rows = (Array.isArray(next) ? next : []).map(row => ({ ...row, reasons: row.reasons || [] }));
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
        button.setAttribute('aria-label', statusLabel(row)); button.title = statusLabel(row);
        button.firstChild.dataset.availability = row.availability;
        if (button.firstChild.textContent !== icon(row)) button.firstChild.textContent = icon(row);
        const label = wireText(row.display_name || row.system_id);
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
    dispose() { close(); root.remove(); dialog.remove(); },
  };
}
