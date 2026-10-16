/** @vitest-environment jsdom */
import { afterEach, expect, it, vi } from 'vitest';
import { capturedPayload } from '../../gui/comms-capture.js';
import { CAPTURE_SELECT_ACTION, registerCaptureActions } from '../../gui/stations/comms-capture-actions.js';
import { mountCommsCaptureControls } from '../../gui/components/comms-capture-controls.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { buildCommsConsoleState } from '../../gui/console-state.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import { makeCommsRender } from '../../gui/stations/comms-console.js';
const row = () => ({ capture: 1, channel: 'fleet', level: 2, captured_tick: 20,
  feasible: true, selected: false, progress_per_mille: 100, decoded_tick: null });
const view = () => capturedPayload({ rows: [row()] }, 'comms', true);

it('projects only the permitted typed feed and grants controls only to its actual Human owner', () => {
  const state = { blackboards: { 'own-radio': {captured:{rows:[row()]}} },
    blackboardKinds:{'own-radio':'Comms'},controlSources:{'own-radio':'Human'} };
  expect(buildCommsConsoleState(state,['own-radio']).comms_capture).toMatchObject({system:'own-radio',controllable:true});
  expect(buildCommsConsoleState(state,['other-system']).comms_capture.controllable).toBe(false);
  expect(buildCommsConsoleState({...state,blackboards:{}}).comms_capture).toBeUndefined();
  const send=vi.fn();dispatchConsoleAction({action:'comms_capture_select',control_system_id:'own-radio',capture:1,correlation:'capture-proof',source:'private'},send);
  expect(send).toHaveBeenCalledWith('ControlSystemCorrelated',{correlation:'capture-proof',target:'own-radio',payload:{type:'SelectCommsCapture',data:{capture:1}}});
  send.mockClear();dispatchConsoleAction({action:'comms_capture_select',control_system_id:'own-radio',capture:9007199254740992},send);expect(send).not.toHaveBeenCalled();
});

it('mounts the independent controls in the existing Comms renderer and hides an absent feed', () => {
  document.body.innerHTML='<section id="comms-capture-host" hidden></section><section id="capture-thread" hidden></section>';
  const render=makeCommsRender({ids:{threadPanel:'capture-thread'}});
  render({messages:[],contacts:[],comms_capture:view()},document);
  const host=document.querySelector('[data-comms-capture]');expect(host.hidden).toBe(false);
  expect(document.getElementById('comms-capture-host').hidden).toBe(false);
  expect(document.getElementById('capture-thread').contains(host)).toBe(false);
  render({messages:[],contacts:[]},document);expect(host.hidden).toBe(true);
  expect(document.getElementById('comms-capture-host').hidden).toBe(true);
});
afterEach(() => { document.body.replaceChildren(); delete window.activateSemanticAction; });
it('keeps undecoded headers opaque and refuses premature or malformed plaintext', () => {
  const decoded = { ...row(), decoded_tick: 30, evidence: { sender_name: 'Literal source', body: 'Literal text', literal_body: false, kind: 'request', emitted_tick: 10,claim:{kind:'request',request:{kind:'report_status'}} } };
  expect(capturedPayload({ rows: [decoded] }, 'comms', false).rows[0].evidence.kind).toBe('request');
  for (const bad of [{ ...decoded, decoded_tick: null }, { ...row(), capture: 0 }, { ...row(), capture: 9007199254740992 },
    { ...row(), captured_tick: NaN }, { ...row(), level: 4 }, { ...row(), progress_per_mille: 1001 },
    { ...decoded, evidence: { ...decoded.evidence, emitted_tick: 21 } }, { ...decoded, evidence: { ...decoded.evidence, kind: 'accepted' } }])
    expect(capturedPayload({ rows: [bad] }, 'comms')).toBeNull();
  expect(capturedPayload({ rows: [row(), row()] }, 'comms')).toBeNull();
  const privateInput = { ...row(), source: 'private', origin_id: 'private', audience: 'private', key: 'private' };
  expect(JSON.stringify(capturedPayload({ rows: [privateInput] }, 'comms'))).not.toContain('private');
});
it('rechecks current own authority, feasible handle and pause before emitting a closed packet', () => {
  let current = { comms_capture: view() }; const send = vi.fn(); let serial = 0;
  const registry = createSemanticActionRegistry({ actionFeedback: new ActionFeedbackLifecycle({ correlation: () => `capture-${++serial}` }) });
  registerCaptureActions(registry, { getState: () => current, sendAction: send });
  const choose = capture => registry.activate(CAPTURE_SELECT_ACTION.id, { context: 'comms-capture', detail: { capture, sender_uuid: 'forged' } });
  choose(1); expect(send).toHaveBeenCalledWith('comms_capture_select', expect.objectContaining({ control_system_id: 'comms', capture: 1 }));
  expect(send.mock.calls[0][1]).not.toHaveProperty('sender_uuid');
  choose(null); expect(send).toHaveBeenCalledTimes(2); send.mockClear();
  choose(2); choose('1'); choose(-1);
  current.comms_capture.rows[0].feasible = false; choose(1);
  current.comms_capture.controllable = false; choose(null);
  current = {}; choose(1); expect(send).not.toHaveBeenCalled();
});
it('shows evidence as text without reply machinery and preserves stable controls', () => {
  const widget = mountCommsCaptureControls(document); document.body.append(widget.node); window.activateSemanticAction = vi.fn();
  widget.update(view()); const button = widget.node.querySelector('button'); button.focus(); widget.update(view());
  expect(widget.node.querySelector('button')).toBe(button); expect(document.activeElement).toBe(button);
  button.click(); expect(window.activateSemanticAction).toHaveBeenCalledWith(CAPTURE_SELECT_ACTION.id, expect.objectContaining({ detail: { capture: 1 } }));
  const decoded = view(); decoded.rows[0].decoded_tick = 30; decoded.rows[0].evidence = { sender_name: '<img src=x>', body: '<script>attack()</script>', literal_body: true, kind: 'text', emitted_tick: 10,claim:{kind:'text'} };
  widget.update(decoded); expect(widget.node.querySelector('script,img')).toBeNull(); expect(widget.node.textContent).toContain('<script>attack()</script>');
  expect(widget.node.querySelectorAll('button')).toHaveLength(1); expect(button.hidden).toBe(true); expect(button.disabled).toBe(true);
  window.activateSemanticAction.mockClear(); widget.update({ ...view(), controllable: false }); button.click();
  widget.update(null); button.click(); widget.dispose(); button.click(); expect(window.activateSemanticAction).not.toHaveBeenCalled();
});
