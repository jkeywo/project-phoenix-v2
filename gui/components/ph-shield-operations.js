import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';
import { SCIENCE_SHIELD_RAISED_ACTION_ID, SCIENCE_SHIELD_RECOVERY_ACTION_ID } from '../stations/sensors-actions.js';

const percent = (value) => `${Math.round(Math.max(0, Math.min(1, Number(value) || 0)) * 100)}%`;
const ratio = (value, maximum) => maximum > 0 ? percent(value / maximum) : '\u2014';

/** Operates only the donor arc; destination readings and controls are separate. */
export class PhShieldOperations extends PhElement {
  template() {
    return `<style>
      :host { display:block; font:inherit; min-width:0; }
      :host([hidden]) { display:none; }
      .arc { border-top:1px solid var(--line); padding:0.5rem 0; }
      h4 { margin:0 0 0.3rem; font-size:var(--text-sm); overflow-wrap:anywhere; }
      dl { display:grid; grid-template-columns:minmax(0,1fr) auto; margin:0; gap:0.2rem 0.4rem; font-size:var(--text-xs); }
      dt,dd { margin:0; overflow-wrap:anywhere; }
      .controls[hidden] { display:none; }
      .controls { display:flex; flex-wrap:wrap; gap:0.3rem; margin-top:0.4rem; }
      button,select { min-height:44px; max-width:100%; font:inherit; }
      label { display:flex; flex-direction:column; flex:1 1 100%; gap:0.2rem; font-size:var(--text-xs); }
      select { width:100%; }
      .recovery-state { font-size:var(--text-xs); overflow-wrap:anywhere; margin-top:0.3rem; }
    </style><div id="arcs"></div>`;
  }

  onTemplate() { this.rows = new Map(); }

  activate(id, detail) {
    window.activateSemanticAction?.(id, { source:'control', detail });
  }

  createRow(systemId) {
    const row = document.createElement('section');
    row.className = 'arc';
    row.dataset.systemId = systemId;
    const title = document.createElement('h4');
    const readings = document.createElement('dl');
    const values = {};
    for (const key of ['strength', 'battery', 'heat', 'protection']) {
      const term = document.createElement('dt');
      term.textContent = t(`shield.operation.${key}`);
      const value = document.createElement('dd');
      value.dataset.reading = key;
      values[key] = value;
      readings.append(term, value);
    }
    const controls = document.createElement('div');
    controls.className = 'controls';
    const raised = document.createElement('button');
    raised.type = 'button';
    raised.addEventListener('click', () => {
      const facing = this.state?.facings?.find(f => f.system_id === systemId);
      if (!facing?.controllable || facing.delegated || this.state?.auto) return;
      this.activate(SCIENCE_SHIELD_RAISED_ACTION_ID, { system_id:systemId, raised:!facing.operation.raised });
    });
    const label = document.createElement('label');
    label.append(document.createTextNode(t('shield.operation.recover_to')));
    const target = document.createElement('select');
    target.addEventListener('change', () => {
      this.activate(SCIENCE_SHIELD_RECOVERY_ACTION_ID, { system_id:systemId, target:target.value || null });
    });
    label.append(target);
    const cancel = document.createElement('button');
    cancel.type = 'button';
    cancel.textContent = t('shield.operation.cancel');
    cancel.addEventListener('click', () => this.activate(SCIENCE_SHIELD_RECOVERY_ACTION_ID, { system_id:systemId, target:null }));
    const status = document.createElement('div');
    status.className = 'recovery-state';
    controls.append(raised, label, cancel);
    row.append(title, readings, controls, status);
    return { row, title, values, controls, raised, target, cancel, status };
  }

  render(state) {
    const facings = (state?.facings || []).filter(f => f.system_id && f.operation && !f.delegated);
    const live = new Set(facings.map(f => f.system_id));
    for (const [id, refs] of this.rows) {
      if (!live.has(id)) { refs.row.remove(); this.rows.delete(id); }
    }
    this.hidden = facings.length === 0 || !!state?.auto;
    for (const facing of facings) {
      const id = facing.system_id;
      let refs = this.rows.get(id);
      if (!refs) {
        refs = this.createRow(id);
        this.rows.set(id, refs);
        this.$('arcs').append(refs.row);
      }
      const operation = facing.operation;
      const reserve = operation.reserve;
      refs.title.textContent = facing.label || facing.arc_id;
      refs.values.strength.textContent = ratio(facing.hp, facing.max_hp);
      refs.values.battery.textContent = reserve ? ratio(reserve.energy, reserve.capacity) : '\u2014';
      refs.values.heat.textContent = reserve ? ratio(reserve.heat, reserve.trip_heat) : '\u2014';
      refs.values.protection.textContent = percent(operation.protection_fraction);
      refs.controls.hidden = !facing.controllable;
      refs.raised.textContent = t(operation.raised ? 'shield.operation.lower' : 'shield.operation.raise');
      refs.raised.setAttribute('aria-pressed', String(operation.raised));
      refs.raised.setAttribute('aria-label', `${refs.title.textContent}: ${t('semantic_action.science.shield_raised.label')}`);
      refs.target.setAttribute('aria-label', `${refs.title.textContent}: ${t('shield.operation.recover_to')}`);
      refs.cancel.setAttribute('aria-label', `${refs.title.textContent}: ${t('shield.operation.cancel')}`);
      const eligible = operation.recovery_energy_per_hp > 0 && operation.discharge_hp_per_sec > 0
        ? operation.recovery_targets || [] : [];
      const signature = JSON.stringify(eligible.map(target => [target, state.facings.find(f => f.system_id === target)?.label || target]));
      if (refs.target.dataset.options !== signature) {
        refs.target.replaceChildren();
        const empty = document.createElement('option');
        empty.value = ''; empty.textContent = t('shield.operation.no_recovery');
        refs.target.append(empty);
        for (const target of eligible) {
          const option = document.createElement('option');
          option.value = target;
          option.textContent = state.facings.find(f => f.system_id === target)?.label || target;
          refs.target.append(option);
        }
        refs.target.dataset.options = signature;
      }
      refs.target.value = operation.recovery_target || '';
      refs.target.disabled = !facing.controllable || eligible.length === 0;
      refs.cancel.disabled = !facing.controllable || !operation.recovery_target;
      const recipient = state.facings.find(f => f.system_id === operation.recovery_target);
      const targetReserve = recipient?.operation?.reserve;
      const waiting = reserve?.thermal_stopped ? 'thermal'
        : facing.offline_remaining > 0 ? 'collapse'
        : targetReserve?.thermal_stopped ? 'target_thermal'
        : targetReserve && targetReserve.energy >= targetReserve.capacity ? 'target_full'
        : null;
      refs.status.textContent = operation.recovery_target
        ? `${t('shield.operation.recovering')}: ${recipient?.label || operation.recovery_target}${waiting ? ` — ${t(`shield.operation.wait.${waiting}`)}` : ''}` : '';
    }
  }
}
phDefine('ph-shield-operations', PhShieldOperations);
