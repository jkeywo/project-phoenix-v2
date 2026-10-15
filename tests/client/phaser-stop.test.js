/** @vitest-environment jsdom */
import { afterEach, expect, it, vi } from 'vitest';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { createTacticalActionRegistry, TACTICAL_PHASER_STOP_ACTION_ID } from '../../gui/stations/tactical-actions.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import '../../gui/components/ph-phasers-controls.js';
afterEach(() => { document.body.replaceChildren(); delete window.activateSemanticAction; });
it('stops only the current owned manual active bank through the correlated admitted action', () => {
  let view = { phaser_mode: 'Manual', banks: [{ id: 'fore', firing: true, on_cooldown: true }],
    weapon_targets: { rows: [{ system: 'phaser-fore', controllable: true }] } };
  let sequence = 0; const sendAction = vi.fn();
  const registry = createTacticalActionRegistry({ getState: () => view, sendAction,
    actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'beam-stop-' + (++sequence) }) });
  const activate = () => registry.activate(TACTICAL_PHASER_STOP_ACTION_ID, { context: 'tactical', detail: { bank: 'fore' } });
  expect(activate()).toMatchObject({ handled: true });
  expect(sendAction).toHaveBeenLastCalledWith('stop_phaser', expect.objectContaining({ bank: 'fore', correlation: 'beam-stop-1' }));
  for (const next of [
    { ...view, banks: [{ id: 'fore', firing: false, on_cooldown: true }] },
    { ...view, phaser_mode: 'Auto' },
    { ...view, weapon_targets: { rows: [{ system: 'phaser-fore', controllable: false }] } },
    { ...view, weapon_targets: { rows: [] } },
  ]) { view = next; expect(activate()).toMatchObject({ handled: false }); }
  expect(sendAction).toHaveBeenCalledTimes(1);
  const wire = vi.fn(); dispatchConsoleAction({ action: 'stop_phaser', bank: 'fore', correlation: 'beam-stop' }, wire);
  expect(wire).toHaveBeenCalledWith('ControlSystemCorrelated', { correlation: 'beam-stop', target: 'phaser-fore', payload: { type: 'StopPhaser' } });
});
it('uses the live firing flag rather than cooldown and keeps delegated active beams read only', () => {
  const activation = vi.fn(); window.activateSemanticAction = activation;
  const el = document.createElement('ph-phasers-controls'); document.body.append(el);
  const bank = { id: 'fore', firing: true, manual_control: true, fire_ready: false, on_cooldown: true };
  el.state = { mode: 'Manual', target_valid: false, banks: [bank] };
  const button = el.shadowRoot.querySelector('.bank-row button');
  expect(button.textContent).toContain('Stop beam'); expect(button.disabled).toBe(false);
  button.click(); expect(activation).toHaveBeenLastCalledWith(TACTICAL_PHASER_STOP_ACTION_ID, expect.objectContaining({ detail: { bank: 'fore' } }));
  el.state = { mode: 'Manual', banks: [{ ...bank, firing: false }] };
  expect(button.textContent).toContain('FIRE'); expect(button.disabled).toBe(true);
  el.state = { mode: 'Manual', banks: [{ ...bank, manual_control: false }] };
  expect(button.textContent).toContain('Stop beam'); expect(button.disabled).toBe(true);
  button.click(); expect(activation).toHaveBeenCalledTimes(1);
});
