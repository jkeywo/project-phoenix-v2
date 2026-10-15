// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { registerGuidanceActions, GUIDANCE_ACTION_IDS } from '../../gui/stations/guidance-actions.js';
import { mountGuidanceControls } from '../../gui/components/guidance-controls.js';
import { mountSystemStatus } from '../../gui/components/ph-system-status.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
const view = () => ({ rounds: [{ round_uuid: 'own-round', tube: 'custom-tube', revision: 1, target: 'public-old', unresolved: true, can_retarget: true, can_stop: true }],
  targets: [{ contact: 'public-current', name: 'Observed target' }] });
const status = guidance => ({ system_id: 'custom-tube', display_name: 'Tube', availability: 'normal', reasons: [], delegated: false, guidance });
afterEach(() => { delete window.activateSemanticAction; document.body.replaceChildren(); });
it('routes only the exact own launch and observed target through correlated tube admission', () => {
  const state = { system_status: [status(view())] }, send = vi.fn(); let sequence = 0;
  const registry = createSemanticActionRegistry({ actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'guidance-' + (++sequence) }) });
  registerGuidanceActions(registry, { getState: () => state, sendAction: send });
  const activate = (kind, detail) => registry.activate(GUIDANCE_ACTION_IDS[kind], { context: 'guidance', detail });
  activate('target', { owner: 'custom-tube', round_uuid: 'own-round', target_uuid: 'public-current' });
  activate('stop', { owner: 'custom-tube', round_uuid: 'own-round' });
  const wire = vi.fn(); for (const [action, fields] of send.mock.calls) dispatchConsoleAction({ action, ...fields }, wire, vi.fn());
  expect(wire.mock.calls.map(row => row[1])).toEqual([
    expect.objectContaining({ target: 'custom-tube', correlation: 'guidance-1', payload: { type: 'RetargetGuidedRound', data: { round_uuid: 'own-round', target_uuid: 'public-current' } } }),
    expect.objectContaining({ target: 'custom-tube', correlation: 'guidance-2', payload: { type: 'StopGuidedRoundGuidance', data: { round_uuid: 'own-round' } } }),
  ]);
  for (const detail of [{ owner: 'foreign-tube', round_uuid: 'own-round', target_uuid: 'public-current' },
    { owner: 'custom-tube', round_uuid: 'guessed-remote', target_uuid: 'public-current' },
    { owner: 'custom-tube', round_uuid: 'own-round', target_uuid: 'private-world-id' }]) activate('target', detail);
  state.system_status[0].delegated = true;
  activate('stop', { owner: 'custom-tube', round_uuid: 'own-round' });
  expect(send).toHaveBeenCalledTimes(2);
});
it('preserves focused native choices and labels a heard receipt without claiming survival or acceptance', () => {
  const widget = mountGuidanceControls(document); document.body.append(widget.node); window.activateSemanticAction = vi.fn();
  const first = view(); widget.update('custom-tube', first);
  const [round, target] = widget.node.querySelectorAll('select'), option = target.options[0]; target.focus();
  widget.update('custom-tube', { ...first, rounds: [{ ...first.rounds[0], unresolved: false }] });
  expect(widget.node.querySelectorAll('select')[1]).toBe(target); expect(target.options[0]).toBe(option); expect(document.activeElement).toBe(target);
  expect(widget.node.textContent).toContain('Order heard; application and survival unknown');
  const [apply, stop] = widget.node.querySelectorAll('button'); apply.click(); stop.click();
  expect(window.activateSemanticAction).toHaveBeenCalledWith(GUIDANCE_ACTION_IDS.target, expect.objectContaining({ detail: { owner: 'custom-tube', round_uuid: 'own-round', target_uuid: 'public-current' } }));
  expect(window.activateSemanticAction).toHaveBeenCalledWith(GUIDANCE_ACTION_IDS.stop, expect.objectContaining({ detail: { owner: 'custom-tube', round_uuid: 'own-round' } }));
  widget.update('custom-tube', { ...first, targets: [] }); expect(apply.disabled).toBe(true); expect(stop.disabled).toBe(false);
  widget.update('custom-tube', first, true); expect(apply.hidden).toBe(true); expect(stop.disabled).toBe(true);
  expect(round.value).toBe('own-round'); widget.dispose();
});
it('mounts source controls through shared System details and keeps delegated summaries read-only', () => {
  const widget = mountSystemStatus(document); widget.update([status(view())]);
  document.querySelector('[data-system-id="custom-tube"]').click();
  expect(document.querySelector('[data-guidance]')).not.toBeNull();
  widget.update([{ ...status(view()), delegated: true }]);
  document.querySelector('[data-delegated-summary]').click();
  expect([...document.querySelector('[data-guidance]').querySelectorAll('button')].every(row => row.hidden && row.disabled)).toBe(true);
  widget.dispose();
});
