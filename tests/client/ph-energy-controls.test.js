/** @vitest-environment jsdom */
import { beforeEach, afterEach, describe, it, expect, vi } from 'vitest';
import '../../gui/components/ph-power-controls.js';
import { createEngineeringActionRegistry } from '../../gui/stations/engineering-actions.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import { createConsoleViewSession } from '../../gui/console-working-view.js';
import { ENERGY_ACTION_IDS } from '../../gui/stations/energy-actions.js';
const group = (id, priority, parent = null) => ({ id, label: id, parent, priority, desired_flow_per_sec: 4, effective_flow_per_sec: 4,
  max_flow_per_sec: 20, desired_cooling_per_sec: 2, effective_cooling_per_sec: 2, max_cooling_per_sec: 5, cut: false });
function model() { return { mode: 'manual', cooling_usage: 'low', reactor: { generation_enabled: true, energy: 50, capacity: 100, heat: 2, trip_heat: 50, recover_heat: 30,
  thermal_stopped: false, output_target_per_sec: 10, output_actual_per_sec: 9, sustainable_output_per_sec: 10, max_output_per_sec: 20, cooling_per_sec: 2, max_cooling_per_sec: 5 },
groups: [group('helm', 0), group('weapons', 1), group('engine', 0, 'helm')], consumers: [{ system: 'drive', group: 'engine', energy: 30, capacity: 40,
  heat: 5, trip_heat: 50, recover_heat: 30, thermal_stopped: false, intake_per_sec: 1, max_intake_per_sec: 2, flow_override_per_sec: 1 }], coolant: [{ provider: 'tank', available: 10, credit: 0.2 }] }; }
function setup() {
  let view = { system_id: 'main-reactor', energy: model(), power_auto: false };
  const wire = vi.fn(), sendAction = (action, detail) => dispatchConsoleAction({ action, ...detail }, wire);
  const registry = createEngineeringActionRegistry({ getState: () => view, sendAction,
    actionFeedback: new ActionFeedbackLifecycle({ correlation: (() => { let n = 0; return () => 'energy-' + ++n; })() }) });
  window.activateSemanticAction = (id, options) => registry.activate(id, { context: 'engineering', ...options });
  document.body.innerHTML = '<ph-power-controls></ph-power-controls>';
  const host = document.querySelector('ph-power-controls'); host.state = { energy: view.energy, auto: false, system_id: view.system_id };
  const widget = host.shadowRoot.querySelector('ph-energy-controls');
  return { host, widget, wire, get view() { return view; }, update(patch) { view = { ...view, ...patch }; host.state = { energy: view.energy, auto: view.power_auto, system_id: view.system_id }; } };
}
const change = (widget, id, value) => { const input = widget.shadowRoot.getElementById(id); input.value = value; input.dispatchEvent(new Event('change')); return input; };
beforeEach(() => { document.body.innerHTML = ''; });
afterEach(() => { document.body.innerHTML = ''; delete window.activateSemanticAction; delete window.sendAction; });
describe('local energy human controls', () => {
  it('sends generation intent through the current operator without changing export or reserve state', () => {
    const fixture = setup(), { widget, wire } = fixture, toggle = widget.shadowRoot.getElementById('generation');
    expect(toggle.checked).toBe(true); toggle.click();
    expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated', expect.objectContaining({ target: 'main-reactor', payload: { type: 'SetEnergyGeneration', data: { enabled: false } } }));
    expect(fixture.view.energy.reactor.generation_enabled).toBe(true);
    expect(fixture.view.energy.reactor.output_target_per_sec).toBe(10);
    fixture.update({ power_auto: true }); expect(toggle.disabled).toBe(true);
    wire.mockClear(); toggle.click(); expect(wire).not.toHaveBeenCalled();
  });
  it('rejects stale or out-of-bounds flow commands without emitting machinery requests', () => {
    const { wire, view } = setup();
    const request = (id, detail) => window.activateSemanticAction(id, { detail });
    request(ENERGY_ACTION_IDS.group, { group: 'removed', flow_per_sec: 1, cooling_per_sec: 1 });
    request(ENERGY_ACTION_IDS.group, { group: 'helm', flow_per_sec: 21, cooling_per_sec: 1 });
    request(ENERGY_ACTION_IDS.system, { system: 'drive', flow_per_sec: 3 });
    request(ENERGY_ACTION_IDS.reactor, { output_per_sec: Infinity, cooling_per_sec: 1 });
    request(ENERGY_ACTION_IDS.priority, { group: 'helm', priority: 2 });
    expect(wire).not.toHaveBeenCalled(); expect(view.energy.groups[0].desired_flow_per_sec).toBe(4);
  });
  it('mounts one new branch and sends actual correlated System commands from native controls', () => {
    const { host, widget, wire, view } = setup();
    expect(host.shadowRoot.getElementById('groups-container').hidden).toBe(true); expect(widget.hidden).toBe(false);
    change(widget, 'flow-helm', '6'); expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated', expect.objectContaining({ target: 'main-reactor',
      payload: { type: 'SetEnergyGroup', data: { group: 'helm', flow_per_sec: 6, cooling_per_sec: 2 } } }));
    expect(view.energy.groups[0].desired_flow_per_sec).toBe(4);
  });
  it('changes only Console working depth and exposes a real minimal priority list', () => {
    const { widget, wire } = setup(), send = vi.fn(), session = createConsoleViewSession({ send, apply: () => {} });
    const listen = event => session.change(event.detail); document.addEventListener('console-view-change', listen);
    change(widget, 'depth', 'min'); document.removeEventListener('console-view-change', listen);
    expect(send.mock.calls[0][0].energy_depth).toBe('min'); expect(wire).not.toHaveBeenCalled();
    expect(widget.shadowRoot.querySelector('[data-energy-group="helm"] .row').hidden).toBe(false);
    widget.shadowRoot.querySelector('[data-energy-group="helm"] .row button:last-child').click();
    expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated', expect.objectContaining({ payload: { type: 'SetEnergyPriority', data: { group: 'helm', priority: 1 } } }));
    change(widget, 'mode', 'minimal'); expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated', expect.objectContaining({ payload: { type: 'SetEnergyMode', data: { mode: 'minimal', cooling_usage: 'low' } } }));
  });
  it('shows nested fine groups and explicit per-System override/reset controls', () => {
    const { widget, wire } = setup(); widget.applyConsoleView('detail');
    expect(widget.shadowRoot.querySelector('[data-energy-group="engine"]').hidden).toBe(false);
    change(widget, 'system-flow-drive', '1.5'); expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated', expect.objectContaining({ payload: { type: 'SetEnergySystemFlow', data: { system: 'drive', flow_per_sec: 1.5 } } }));
    widget.shadowRoot.querySelector('[data-energy-consumer="drive"] button').click();
    expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated', expect.objectContaining({ payload: { type: 'SetEnergySystemFlow', data: { system: 'drive', flow_per_sec: null } } }));
  });
  it('keeps paired input intent during stale snapshots without changing authoritative readouts', () => {
    const fixture = setup(), { widget, wire } = fixture;
    change(widget, 'output', '12'); fixture.update({ energy: structuredClone(fixture.view.energy) }); change(widget, 'reactor-cooling', '3');
    expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated', expect.objectContaining({ payload: { type: 'SetEnergyReactor', data: { output_per_sec: 12, cooling_per_sec: 3 } } }));
    expect(fixture.view.energy.reactor.output_target_per_sec).toBe(10);
    window.dispatchEvent(new CustomEvent('phoenix-action-feedback', { detail: { actionId: ENERGY_ACTION_IDS.reactor, state: 'Refused', isCurrent: true } }));
    expect(widget.shadowRoot.getElementById('output').value).toBe('10');
  });
  it('preserves focused controls across updates and disables ordinary delegated controls', () => {
    const fixture = setup(), input = fixture.widget.shadowRoot.getElementById('flow-helm'); input.focus();
    const next = structuredClone(fixture.view.energy); next.groups[0].cut = true; next.groups[0].effective_flow_per_sec = 0;
    fixture.update({ energy: next }); expect(fixture.widget.shadowRoot.getElementById('flow-helm')).toBe(input);
    expect(fixture.widget.shadowRoot.activeElement).toBe(input); expect(input.disabled).toBe(false);
    expect(fixture.widget.shadowRoot.querySelector('[data-energy-group="helm"] .stats').textContent).toContain('Intake cut');
    fixture.update({ power_auto: true }); expect(input.disabled).toBe(true);
    expect(fixture.widget.shadowRoot.getElementById('depth').disabled).toBe(false);
  });
});
