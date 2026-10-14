import '../strings-boot.js';
import { t, wireText } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';
import { ENERGY_ACTION_IDS } from '../stations/energy-actions.js';
import { activateEngineeringAction } from '../stations/engineering-action-control.js';

const number = value => Number.isFinite(value) ? value.toFixed(2).replace(/\.00$/, '') : '—';
const sorted = rows => rows.slice().sort((a, b) => a.priority - b.priority || a.id.localeCompare(b.id));
export class PhEnergyControls extends PhElement {
  template() {
    return `<style>:host{display:block;color:var(--ink)}[hidden]{display:none!important}fieldset{border:1px solid var(--line-faint);padding:.5rem;margin:.5rem 0;min-width:0}label{display:flex;flex-wrap:wrap;align-items:center;gap:.4rem}input[type=range]{flex:1;min-width:6rem;min-height:44px}select,button{min-height:44px;font:inherit;color:inherit;background:var(--bg-card);border:1px solid var(--line-faint)}.row{display:flex;flex-wrap:wrap;align-items:center;gap:.4rem}.stats{font-size:var(--text-xs);line-height:1.6}.cut{color:var(--reloading)}.hot{color:var(--fire)}.children{margin-inline-start:1rem}output{min-width:3rem}summary{cursor:pointer;min-height:44px;padding:.4rem}progress{width:8rem;max-width:100%}</style>
    <div class="row"><label for="depth">${t('energy.controls.depth')}<select id="depth"><option value="min">${t('energy.controls.min')}</option><option value="mid">${t('energy.controls.mid')}</option><option value="detail">${t('energy.controls.detail')}</option></select></label>
    <label for="mode">${t('energy.controls.mode')}<select id="mode"><option value="manual">${t('energy.controls.manual')}</option><option value="minimal">${t('energy.controls.minimal')}</option></select></label>
    <label for="usage">${t('energy.controls.coolant_usage')}<select id="usage"><option value="none">${t('energy.controls.none')}</option><option value="low">${t('energy.controls.low')}</option><option value="high">${t('energy.controls.high')}</option></select></label></div>
    <fieldset id="reactor"><legend>${t('energy.controls.reactor')}</legend><p class="stats" id="reactor-status"></p><label for="generation"><input id="generation" type="checkbox">${t('energy.controls.generation')}</label><div id="reactor-controls"></div></fieldset>
    <p class="stats" id="coolant"></p><div id="groups"></div><details id="consumer-section"><summary>${t('energy.controls.local_reserves')}</summary><div id="consumers"></div></details>`;
  }
  onTemplate() {
    this.$('generation').onchange = () => { if (!this.$('generation').disabled) this.#activate('generation', { enabled: this.$('generation').checked }); };
    this._depth ||= 'mid'; this._groups = new Map(); this._consumers = new Map();
    this._groupRoot = this.$('groups'); this._consumerRoot = this.$('consumers');
    this.$('depth').onchange = () => { this._depth = this.$('depth').value; this.render(this.state);
      this.dispatchEvent(new CustomEvent('console-view-change', { detail: { energy_depth: this._depth }, bubbles: true, composed: true })); };
    const mode = () => this.#activate('mode', { mode: this.$('mode').value, cooling_usage: this.$('usage').value });
    this.$('mode').onchange = mode; this.$('usage').onchange = mode;
    this._output = this._slider(this.$('reactor-controls'), 'output', 'energy.controls.output', value => this.#activate('reactor', {
      output_per_sec: value, cooling_per_sec: this._cooling.input.valueAsNumber }));
    this._cooling = this._slider(this.$('reactor-controls'), 'reactor-cooling', 'energy.controls.cooling', value => this.#activate('reactor', {
      output_per_sec: this._output.input.valueAsNumber, cooling_per_sec: value }));
  }
  connectedCallback() {
    super.connectedCallback();
    this._feedbackHandler ||= event => {
      const value = event.detail;
      if (value?.isCurrent === false || !['Refused', 'TimedOut'].includes(value?.state) || !Object.values(ENERGY_ACTION_IDS).includes(value?.actionId)) return;
      const controls = [this._output, this._cooling, ...[...this._groups.values()].flatMap(row => [row.flow, row.cooling]), ...[...this._consumers.values()].map(row => row.flow)];
      for (const control of controls) { control.pending = null; if (control.desired != null) control.input.value = String(control.desired); }
      this.render(this.state);
    };
    this.ownerDocument.defaultView?.addEventListener('phoenix-action-feedback', this._feedbackHandler);
  }
  disconnectedCallback() { super.disconnectedCallback?.(); this.ownerDocument.defaultView?.removeEventListener('phoenix-action-feedback', this._feedbackHandler); }
  #activate(name, detail) {
    if (this.state?.auto) return;
    activateEngineeringAction(this, ENERGY_ACTION_IDS[name], detail, 'set_energy_' + name);
  }
  _slider(parent, id, key, changed) {
    const doc = this.ownerDocument, label = doc.createElement('label'), text = doc.createElement('span'), input = doc.createElement('input'), output = doc.createElement('output');
    text.textContent = t(key); input.id = id; input.type = 'range'; input.min = '0'; input.step = 'any'; label.htmlFor = id;
    input.oninput = () => { output.value = number(input.valueAsNumber); };
    const control = { input, output, label, pending: null, desired: null };
    input.onchange = () => { if (!input.disabled && Number.isFinite(input.valueAsNumber)) {
      control.pending = { value: input.valueAsNumber, before: control.desired }; changed(input.valueAsNumber);
    } };
    label.append(text, input, output); parent.append(label); return control;
  }
  #value(control, desired, maximum, disabled) {
    control.input.max = String(maximum); control.input.disabled = disabled;
    if (disabled || (control.pending && (Math.fround(desired) === Math.fround(control.pending.value) || desired !== control.pending.before))) control.pending = null;
    control.desired = desired; const displayed = control.pending?.value ?? desired;
    // Keep a live gesture/focus through incoming snapshots. The readouts still
    // report current authoritative requested/effective values independently.
    if (disabled || this.shadowRoot.activeElement !== control.input) control.input.value = String(displayed);
    control.output.value = number(!disabled && this.shadowRoot.activeElement === control.input ? control.input.valueAsNumber : displayed);
  }
  applyConsoleView(depth) { this._depth = ['min', 'mid', 'detail'].includes(depth) ? depth : 'mid'; this.render(this.state); }
  #makeGroup(group) {
    const doc = this.ownerDocument, node = doc.createElement('fieldset'), heading = doc.createElement('legend'), stats = doc.createElement('p'), controls = doc.createElement('div');
    node.dataset.energyGroup = group.id; stats.className = 'stats'; node.append(heading, stats, controls);
    const flow = this._slider(controls, 'flow-' + group.id, 'energy.controls.flow', value => {
      this.#activate('group', { group: group.id, flow_per_sec: value, cooling_per_sec: cooling.input.valueAsNumber }); });
    const cooling = this._slider(controls, 'cooling-' + group.id, 'energy.controls.cooling', value => {
      this.#activate('group', { group: group.id, flow_per_sec: flow.input.valueAsNumber, cooling_per_sec: value }); });
    const priority = doc.createElement('div'); priority.className = 'row'; const priorityLabel = doc.createElement('span'); priorityLabel.textContent = t('energy.controls.priority'); priority.append(priorityLabel);
    const move = (direction, label) => { const button = doc.createElement('button'); button.type = 'button'; button.textContent = t(label);
      button.onclick = () => { const live = this.state.energy.groups.find(row => row.id === group.id), siblings = sorted(this.state.energy.groups.filter(row => row.parent === live.parent));
        this.#activate('priority', { group: group.id, priority: siblings.findIndex(row => row.id === group.id) + direction }); };
      priority.append(button); return button; };
    const up = move(-1, 'energy.controls.raise_priority'), down = move(1, 'energy.controls.lower_priority'); node.append(priority);
    const children = doc.createElement('div'); children.className = 'children'; node.append(children);
    return { node, heading, stats, controls, flow, cooling, priority, up, down, children };
  }
  render(state) {
    const model = state?.energy; if (!model) return;
    const auto = !!state.auto, reactor = model.reactor, depth = this._depth;
    this.$('depth').value = depth; this.$('mode').value = model.mode; this.$('usage').value = model.cooling_usage;
    this.$('mode').disabled = this.$('usage').disabled = auto;
    this.$('usage').parentElement.hidden = model.mode !== 'minimal';
    const reactorStatus = this.$('reactor-status'); reactorStatus.classList.toggle('hot', reactor.thermal_stopped);
    reactorStatus.textContent = t('energy.controls.reactor_status', { energy: number(reactor.energy), capacity: number(reactor.capacity), heat: number(reactor.heat), trip: number(reactor.trip_heat), output: number(reactor.output_actual_per_sec), desired: number(reactor.output_target_per_sec) })
      + (reactor.thermal_stopped ? ' ' + t('energy.controls.thermal_stop') : '');
    this.$('generation').disabled = auto; this.$('generation').checked = reactor.generation_enabled;
    this.$('reactor-controls').hidden = depth === 'min';
    this.#value(this._output, reactor.output_target_per_sec, reactor.max_output_per_sec, auto);
    this.#value(this._cooling, reactor.cooling_per_sec, reactor.max_cooling_per_sec, auto);
    this.$('coolant').textContent = (model.coolant || []).map(stock => t('energy.controls.coolant_stock', { provider: wireText(stock.provider), available: number(stock.available), credit: number(stock.credit) })).join(' · ');
    const rows = model.groups || [], ids = new Set(rows.map(row => row.id));
    for (const [id, row] of this._groups) if (!ids.has(id)) { row.node.remove(); this._groups.delete(id); }
    for (const group of rows) if (!this._groups.has(group.id)) this._groups.set(group.id, this.#makeGroup(group));
    const place = (parent, children) => sorted(children).forEach((group, index) => {
      const row = this._groups.get(group.id); if (parent.children[index] !== row.node) parent.insertBefore(row.node, parent.children[index] || null);
      row.node.hidden = !!group.parent && depth !== 'detail'; row.heading.textContent = wireText(group.label || group.id); row.stats.classList.toggle('cut', group.cut);
      row.stats.textContent = t('energy.controls.group_status', { desired: number(group.desired_flow_per_sec), actual: number(group.effective_flow_per_sec), cooling: number(group.effective_cooling_per_sec), requested_cooling: number(group.desired_cooling_per_sec) })
        + (group.cut ? ' ' + t('energy.controls.flow_cut') : '');
      row.controls.hidden = depth === 'min'; row.priority.hidden = depth !== 'min';
      this.#value(row.flow, group.desired_flow_per_sec, group.max_flow_per_sec, auto || model.mode === 'minimal');
      this.#value(row.cooling, group.desired_cooling_per_sec, group.max_cooling_per_sec, auto || model.mode === 'minimal');
      row.up.disabled = auto || index === 0; row.down.disabled = auto || index === children.length - 1;
      place(row.children, rows.filter(child => child.parent === group.id));
    });
    place(this._groupRoot, rows.filter(row => !row.parent));
    this.#consumers(model, auto, depth);
  }
  #consumers(model, auto, depth) {
    const rows = model.consumers || [], ids = new Set(rows.map(row => row.system));
    for (const [id, row] of this._consumers) if (!ids.has(id)) { row.node.remove(); this._consumers.delete(id); }
    for (const [index, consumer] of rows.entries()) {
      let row = this._consumers.get(consumer.system);
      if (!row) {
        const doc = this.ownerDocument, node = doc.createElement('fieldset'), heading = doc.createElement('legend'), stats = doc.createElement('p'), controls = doc.createElement('div'); stats.className = 'stats'; node.dataset.energyConsumer = consumer.system; node.append(heading, stats, controls);
        const flow = this._slider(controls, 'system-flow-' + consumer.system, 'energy.controls.override_flow', value => this.#activate('system', { system: consumer.system, flow_per_sec: value }));
        const reset = doc.createElement('button'); reset.type = 'button'; reset.textContent = t('energy.controls.reset_flow'); reset.onclick = () => { flow.pending = null; this.#activate('system', { system: consumer.system, flow_per_sec: null }); }; controls.append(reset);
        row = { node, heading, stats, controls, flow, reset }; this._consumers.set(consumer.system, row);
      }
      if (this._consumerRoot.children[index] !== row.node) this._consumerRoot.insertBefore(row.node, this._consumerRoot.children[index] || null);
      row.heading.textContent = wireText(model.groups.find(group => group.id === consumer.group)?.label || consumer.system); row.stats.classList.toggle('hot', consumer.thermal_stopped);
      row.stats.textContent = t('energy.controls.consumer_status', { energy: number(consumer.energy), capacity: number(consumer.capacity), heat: number(consumer.heat), trip: number(consumer.trip_heat), intake: number(consumer.intake_per_sec) })
        + (consumer.thermal_stopped ? ' ' + t('energy.controls.thermal_stop') : '');
      row.controls.hidden = depth !== 'detail';
      this.#value(row.flow, consumer.flow_override_per_sec ?? consumer.intake_per_sec, consumer.max_intake_per_sec, auto);
      row.reset.disabled = auto || consumer.flow_override_per_sec == null;
    }
  }
}
phDefine('ph-energy-controls', PhEnergyControls);
