import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';
import { SENSORS_PASSIVE_ACTION_ID } from '../stations/sensors-actions.js';

/** Bearing evidence only. No contact association or position is inferred here. */
export class PhPassiveReadout extends PhElement {
  template() { return `<style>
    :host{display:block;font:inherit;min-width:0}:host([hidden]){display:none}
    section{border-top:1px solid var(--line-soft);padding:.4rem 0}h3,p{margin:.3rem 0;font-size:var(--text-xs);overflow-wrap:anywhere}
    ul{padding:0;list-style:none;font-size:var(--text-xs)}li{padding:.25rem 0;overflow-wrap:anywhere}
    button{font:inherit;min-height:44px;width:100%;background:transparent;color:var(--ink);border:1px solid var(--line-soft)}
    </style><div id="receivers"></div>`; }
  onTemplate() { this.receivers = new Map(); }
  render(state) {
    const entries = Array.isArray(state?.passive) ? state.passive : [];
    this.hidden = !entries.length;
    for (const [id, row] of this.receivers) if (!entries.some(entry => entry.system_id === id)) { row.node.remove(); this.receivers.delete(id); }
    for (const entry of entries) {
      if (!entry.system_id) continue; let row = this.receivers.get(entry.system_id);
      if (!row) {
        const doc = this.ownerDocument, node = doc.createElement('section'), heading = doc.createElement('h3'), status = doc.createElement('p'), toggle = doc.createElement('button'), signals = doc.createElement('ul');
        heading.textContent = t('passive.title'); toggle.type = 'button'; node.append(heading, status, toggle, signals); this.$('receivers').append(node);
        row = { node, status, toggle, signals, entries: new Map(), current: entry }; this.receivers.set(entry.system_id, row);
        toggle.addEventListener('click', () => { if (row.current.controllable) window.activateSemanticAction?.(SENSORS_PASSIVE_ACTION_ID,
          { source: 'control', detail: { system_id: row.current.system_id, enabled: !row.current.enabled } }); });
      }
      row.current = entry; row.node.dataset.systemId = entry.system_id;
      row.toggle.hidden = !entry.controllable; row.toggle.textContent = t(entry.enabled ? 'passive.disable' : 'passive.enable');
      row.status.textContent = t(entry.enabled ? 'passive.enabled' : 'passive.disabled') + (entry.can_operate ? '' : ' · ' + t('passive.unavailable'));
      const samples = Array.isArray(entry.signals) ? entry.signals : [];
      for (const [id, node] of row.entries) if (!samples.some(sample => sample.sample_id === id)) { node.remove(); row.entries.delete(id); }
      for (const sample of samples) {
        let node = row.entries.get(sample.sample_id); if (!node) { node = this.ownerDocument.createElement('li'); row.entries.set(sample.sample_id, node); row.signals.append(node); }
        const bearing = Number.isFinite(sample.bearing_rad) ? Math.round(((sample.bearing_rad * 180 / Math.PI) % 360 + 360) % 360) : '—';
        const error = Number.isFinite(sample.bearing_error_rad) ? Math.round(sample.bearing_error_rad * 180 / Math.PI) : '—';
        const age = Number.isSafeInteger(entry.observation_tick) && Number.isSafeInteger(sample.observed_tick) ? t('observation.ticks', { ticks: Math.max(0, entry.observation_tick - sample.observed_tick) }) : t('observation.unknown');
        const signature = ['unclassified', 'reactor', 'propulsion', 'weapon', 'active_scan', 'comms'].includes(sample.signature) ? sample.signature : 'unclassified';
        node.textContent = t('passive.sample', { bearing, error, signature: t('passive.signature.' + signature), strength: Number.isFinite(sample.strength) ? sample.strength.toFixed(2) : '—', age });
      }
      row.signals.hidden = !samples.length;
    }
  }
}
phDefine('ph-passive-readout', PhPassiveReadout);
