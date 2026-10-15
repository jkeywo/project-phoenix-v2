/** @vitest-environment jsdom */
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { weaponTargetsPayload } from '../../gui/weapon-targets.js';
import { createTacticalActionRegistry } from '../../gui/stations/tactical-actions.js';
import { WEAPON_TARGET_ACTION_IDS as ids } from '../../gui/stations/weapon-target-actions.js';
import { mountWeaponTargetControls } from '../../gui/components/weapon-target-controls.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
let sequence = 0;
const registryFor = options => createTacticalActionRegistry({ ...options, actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'target-' + (++sequence) }) });
const state = () => ({ weapon_targets: { rows: [
  { system: 'beam-a', controllable: true, assignment: { group: { group: 'pair', target: 'one' }, individual: { mode: 'hold' } } },
  { system: 'tube-b', controllable: false, assignment: { individual: null, group: null } },
  { system: 'beam-c', controllable: true, assignment: { individual: null, group: null } },
], contacts: [{ id: 'one', name: 'Known' }], groups: [{ id: 'pair', label: 'weapon.group.main_beams', members: ['beam-a', 'tube-b', 'beam-c'] }] } });
describe('weapon target assignments', () => {
  afterEach(() => { delete window.activateSemanticAction; document.body.replaceChildren(); });
  it('projects only exact owned Human machinery as controllable without granting a group', () => {
    const view = weaponTargetsPayload({ systemKinds: { a: 'phaser_bank', b: 'torpedo_tube', radar: 'tactical_radar' }, controlSources: { a: 'Human', b: 'Human' } }, ['a'], { b: { individual: { mode: 'hold' } } },
      [{ uuid: 'current', selectable: true }, { uuid: 'old', selectable: true, predicted: true }, { uuid: 'unknown' }]);
    expect(view.rows.map(row => [row.system, row.controllable])).toEqual([['a', true], ['b', false]]);
    expect(view.contacts.map(row => row.id)).toEqual(['current']);
  });
  it('labels only actual AI control as Auto while Offline and absent sources remain read-only', () => {
    const view = weaponTargetsPayload({ systemKinds: { ai: 'phaser_bank', off: 'phaser_bank', absent: 'phaser_bank' }, controlSources: { ai: 'Ai', off: 'Offline' } }, ['ai', 'off', 'absent'], {}, []);
    expect(view.rows.map(row => [row.system, row.auto, row.controllable])).toEqual([['absent', false, false], ['ai', true, false], ['off', false, false]]);
  });
  it('sends explicit hold and both rejoin operations to the actual System with correlation', () => {
    const sendAction = vi.fn(), registry = registryFor({ getState: state, sendAction });
    for (const [name, detail, action] of [['target', { system: 'beam-a', uuid: null }, 'set_weapon_target'], ['rejoin', { system: 'beam-a' }, 'follow_weapon_group'], ['primary', { system: 'beam-a' }, 'follow_primary_target']]) {
      expect(registry.activate(ids[name], { context: 'tactical', detail })).toMatchObject({ handled: true });
      expect(sendAction).toHaveBeenLastCalledWith(action, expect.objectContaining({ control_system_id: 'beam-a', correlation: expect.any(String) }));
    }
    expect(sendAction.mock.calls[0][1].uuid).toBe(null);
  });
  it('refuses unknown/stale contacts, delegated rows and group nonmembers at the semantic seam', () => {
    const sendAction = vi.fn(), registry = registryFor({ getState: state, sendAction });
    for (const [name, detail] of [['target', { system: 'beam-a', uuid: 'stale' }], ['target', { system: 'tube-b', uuid: null }], ['group', { group: 'pair', uuid: 'one' }], ['group', { system: 'tube-b', group: 'pair', uuid: 'one' }], ['group', { system: 'beam-a', group: 'missing', uuid: 'one' }]])
      expect(registry.activate(ids[name], { context: 'tactical', detail })).toMatchObject({ handled: false });
    expect(sendAction).not.toHaveBeenCalled();
  });
  it('group click makes separate correlated activations only for current authorized members', () => {
    const sendAction = vi.fn(); let current = state();
    const registry = registryFor({ getState: () => current, sendAction });
    window.activateSemanticAction = (id, event) => registry.activate(id, event);
    const controls = mountWeaponTargetControls(document); document.body.append(controls.node); controls.update(current.weapon_targets);
    const group = document.querySelector('[data-assignment-key="pair"]');
    group.querySelector('select').value = 'one'; group.querySelector('button').click();
    expect(sendAction.mock.calls.map(([, data]) => data.control_system_id)).toEqual(['beam-a', 'beam-c']);
    expect(new Set(sendAction.mock.calls.map(([, data]) => data.correlation)).size).toBe(2);
    current = { weapon_targets: { ...current.weapon_targets, rows: current.weapon_targets.rows.map(row => ({ ...row, controllable: false })) } };
    group.querySelector('button').click(); expect(sendAction).toHaveBeenCalledTimes(2);
  });
  it('keeps focused picker and draft through progress snapshots and does not rewrite received assignment intent', () => {
    const controls = mountWeaponTargetControls(document); document.body.append(controls.node); const value = state().weapon_targets;
    controls.update(value); const picker = document.querySelector('[data-assignment-key="beam-a"] select'); picker.value = 'one'; picker.focus();
    controls.update(structuredClone(value)); expect(document.activeElement).toBe(picker); expect(picker.value).toBe('one');
    expect(value.rows[0].assignment.individual).toEqual({ mode: 'hold' }); expect(controls.node.open).toBe(false);
  });
  it('maps the four wire commands without a guessed fallback System', () => {
    const send = vi.fn();
    for (const [action, type] of [['set_weapon_target', 'SetWeaponTarget'], ['set_weapon_group_target', 'SetWeaponGroupTarget'], ['follow_weapon_group', 'FollowWeaponGroup'], ['follow_primary_target', 'FollowPrimaryTarget']]) {
      dispatchConsoleAction({ action, control_system_id: 'custom-bank', uuid: null, group: 'pair', correlation: 'request' }, send);
      expect(send).toHaveBeenLastCalledWith('ControlSystemCorrelated', expect.objectContaining({ target: 'custom-bank', payload: expect.objectContaining({ type }) }));
    }
    send.mockClear(); dispatchConsoleAction({ action: 'set_weapon_target', uuid: 'one' }, send); expect(send).not.toHaveBeenCalled();
  });
  it('retains a vanished draft contact without turning the next click into hold fire', () => {
    let current = state(); const sendAction = vi.fn();
    const registry = registryFor({ getState: () => current, sendAction });
    window.activateSemanticAction = (id, event) => registry.activate(id, event);
    const controls = mountWeaponTargetControls(document); document.body.append(controls.node); controls.update(current.weapon_targets);
    const row = document.querySelector('[data-assignment-key="beam-a"]'), picker = row.querySelector('select');
    picker.value = 'one'; picker.focus();
    current.weapon_targets.contacts = []; controls.update(current.weapon_targets);
    expect(document.activeElement).toBe(picker); expect(picker.value).toBe('one');
    expect(picker.selectedOptions[0].disabled).toBe(true);
    row.querySelector('button').click(); expect(sendAction).not.toHaveBeenCalled();
  });
});
