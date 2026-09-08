// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { createGmConfirmationController, createGmConfirmationProfile } from '../../gui/gm-confirmation.js';
import { observeGm } from '../smoke/gm-m2-evidence.js';

let controller;
afterEach(async () => {
  controller?.destroy();
  await Promise.resolve();
  document.body.replaceChildren();
  delete window.__m2Evidence;
});
async function setup() {
  const profile = createGmConfirmationProfile({ storage: { getItem: () => null, setItem() {} } });
  window.__hostGmConfirmationProfile = profile;
  window.__hostLocalGm = () => ({ id: 'gm-a' });
  window.wasm_sim_tick = () => 42;
  const submit = vi.fn(() => true);
  window.wasm_submit_gm_action = submit;
  controller = createGmConfirmationController({ doc: document, profile });
  await observeGm({ evaluate: (fn, arg) => fn(arg) }, 'A');
  const open = () => controller.request({ category: 'effect.damage', description: 'captured target',
    accept: () => window.wasm_submit_gm_action(JSON.stringify({ target: 'target-a' })) });
  return { open, submit, records: window.__m2Evidence };
}
it.each(['button', 'escape', 'backdrop'])('retains actual %s cancellation exactly once', async reason => {
  const { open, submit, records } = await setup();
  open();
  const dialog = document.getElementById('gm-action-confirmation');
  if (reason === 'button') dialog.querySelector('[data-confirmation-cancel]').click();
  if (reason === 'backdrop') dialog.click();
  if (reason === 'escape') document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await Promise.resolve();
  expect(dialog.hidden).toBe(true);
  expect(records.confirmations.map(row => row.event)).toEqual(['shown', 'cancelled']);
  expect(records.confirmations[1]).toMatchObject({ reason, description: 'captured target', operator: { id: 'gm-a' } });
  expect(submit).not.toHaveBeenCalled();
  expect(records.requests).toEqual([]);
});
it('retains same-turn show and acceptance with one ordinary submission', async () => {
  const { open, submit, records } = await setup();
  open();
  document.querySelector('[data-confirmation-accept]').click();
  await Promise.resolve();
  expect(records.confirmations.map(row => row.event)).toEqual(['shown', 'accepted']);
  expect(submit).toHaveBeenCalledTimes(1);
  expect(records.requests).toHaveLength(1);
  expect(records.requests[0]).toMatchObject({ acceptedAtIngress: true, request: { target: 'target-a' } });
});
it('labels programmatic closure without fabricating a human cancellation', async () => {
  const { open, records } = await setup();
  open();
  controller.cancel();
  await Promise.resolve();
  expect(records.confirmations.map(row => row.event)).toEqual(['shown', 'closed']);
  expect(records.confirmations[1].reason).toBe('without-observed-human-decision');
});
it('retains changing previews as updates and a repeated same-turn dialog as a new showing', async () => {
  const { open, records } = await setup();
  let prediction = 'before';
  controller.request({ category: 'effect.lethal', description: 'captured lethal target',
    preview: () => prediction, accept: () => true });
  await Promise.resolve();
  prediction = 'after';
  controller.refresh();
  await Promise.resolve();
  document.querySelector('[data-confirmation-cancel]').click();
  open();
  document.querySelector('[data-confirmation-cancel]').click();
  await Promise.resolve();
  expect(records.confirmations.map(row => row.event)).toEqual(['shown', 'updated', 'cancelled', 'shown', 'cancelled']);
  expect(records.confirmations.slice(0, 2).map(row => row.preview)).toEqual(['before', 'after']);
});
