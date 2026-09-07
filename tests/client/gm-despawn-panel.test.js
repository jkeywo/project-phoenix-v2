// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from 'vitest';
import { createGmDespawnPanel } from '../../gui/gm-despawn-panel.js';
import { t } from '../../gui/strings.js';

beforeEach(() => {
  document.body.innerHTML = `<p id="gm-despawn-target"></p><button id="gm-despawn-preview"></button>
    <div id="gm-despawn-confirmation" hidden><p id="gm-despawn-consequence"></p>
    <button id="gm-despawn-confirm"></button><button id="gm-despawn-cancel"></button></div>
    <p id="gm-despawn-feedback"></p><ol id="gm-despawn-results"></ol>`;
});
const entity = (id = 'npc') => ({ entity_id: id, name: id, kind: 'npc-ship', removable: true });
function mount() {
  const submit = vi.fn(() => true);
  const schedule = vi.fn();
  const panel = createGmDespawnPanel({ doc: document, submit, schedule,
    getOperator: () => ({ id: 'gm-1' }), correlation: () => 'remove-1' });
  return { panel, submit, schedule };
}
it('requires a target-specific preview and explicit confirmation; cancellation is inert', () => {
  const { panel, submit } = mount();
  panel.select(entity());
  expect(panel.confirm()).toBe(false);
  expect(panel.preview()).toBe(true);
  expect(document.getElementById('gm-despawn-confirmation').hidden).toBe(false);
  document.getElementById('gm-despawn-cancel').click();
  expect(panel.confirm()).toBe(false);
  expect(submit).not.toHaveBeenCalled();
  panel.preview();
  expect(panel.confirm()).toBe(true);
  expect(submit).toHaveBeenCalledWith({ operator_id: 'gm-1', correlation: 'remove-1', target: 'npc' });
  expect(panel.state().selected.entity_id).toBe('npc'); // no optimistic disappearance
  expect(panel.confirm()).toBe(false);
});
it.each(['player-ship', 'gm-peer', 'region', 'asteroid-field', 'authored-asteroid', 'npc-ship', 'structure', 'hazard'])
  ('refuses protected %s previews without submitting', (kind) => {
    const { panel, submit } = mount();
    panel.select({ ...entity(), kind, removable: false });
    expect(panel.preview()).toBe(false);
    expect(panel.confirm()).toBe(false);
    expect(document.getElementById('gm-despawn-preview').disabled).toBe(true);
    expect(submit).not.toHaveBeenCalled();
  });
it.each(['npc-ship', 'structure', 'hazard'])('accepts a projected removable %s', (kind) => {
  const { panel, submit } = mount(); panel.select({ ...entity(), kind });
  panel.preview(); expect(panel.confirm()).toBe(true); expect(submit).toHaveBeenCalledOnce();
});
it('invalidates confirmation on selection, removal and permission changes', () => {
  const { panel, submit } = mount();
  for (const replacement of [entity('other'), null, { ...entity(), removable: false }]) {
    panel.select(entity()); panel.preview(); panel.select(replacement);
    expect(panel.confirm()).toBe(false);
  }
  expect(submit).not.toHaveBeenCalled();
});
it('only matching attributed canonical results settle pending work and preserve terminal history', () => {
  const { panel } = mount(); panel.select(entity()); panel.preview(); panel.confirm();
  const result = { action_kind: 'world-despawn', target: 'npc', operator_id: 'gm-2', correlation: 'remove-1', tick: 4, outcome: 'applied' };
  panel.update({ despawn_results: [result] });
  expect(panel.state().pending).not.toBeNull();
  panel.update({ despawn_results: [{ ...result, operator_id: 'gm-1' }] });
  expect(panel.state().pending).toBeNull();
  expect(document.querySelector('#gm-despawn-feedback').dataset.state).toBe('applied');
  panel.select(null);
  expect(document.querySelectorAll('#gm-despawn-results li')).toHaveLength(1);
});
it('times out bounded pending feedback and ignores malformed result replacements', () => {
  const { panel, schedule } = mount(); panel.select(entity()); panel.preview(); panel.confirm();
  expect(panel.update({ despawn_results: [{ target: 'npc' }] })).toBe(false);
  schedule.mock.calls[0][0]();
  expect(panel.state().pending).toBeNull();
  expect(document.querySelector('#gm-despawn-feedback').dataset.state).toBe('timed_out');
  panel.reset(); expect(panel.state().selected).toBeNull();
});

it.each(['unknown-entity', 'protected-entity'])('renders the real localized %s refusal', (reason) => {
  const panel = createGmDespawnPanel({ doc: document, t });
  panel.update({ despawn_results: [{ action_kind: 'world-despawn', operator_id: 'gm-1',
    correlation: 'old', target: 'target', tick: 5, outcome: 'refused', reason }] });
  const text = document.querySelector('#gm-despawn-results').textContent;
  expect(text).toContain(t(`server.gm.session.reason.${reason.replaceAll('-', '_')}`));
  expect(text).not.toMatch(/server\.gm\.|⟨/);
});
