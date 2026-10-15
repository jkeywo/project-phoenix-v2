// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { buildReferenceControls } from '../../gui/sensor-references.js';
import { createSensorScienceActionRegistry, SENSORS_ATTACH_REFERENCE_ACTION_ID } from '../../gui/stations/sensors-actions.js';
import '../../gui/components/ph-sensor-references.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';

const record = { reference: { id: 'unusual-entry', unusual: true, reading_ticks: 2 }, text: 'Held reference', provenance: 'records', gathered_at_tick: 3, source_name: 'Registry' };
const control = () => ({ system_id: 'actual-radar', controllable: true, contacts: ['opaque-contact'], records: [record] });

describe('held sensor references', () => {
  it('uses only explicitly supplied permitted feeds and refuses ambiguous record IDs', () => {
    const args = { radars: [{ systemId: 'actual-radar', data: { tracks: [{ contact_id: 'opaque-contact' }] } }], dossiers: [{ data: { subjects: [{ name: 'Registry', evidence: [record] }] } }], systemIds: ['actual-radar'], controlSources: { 'actual-radar': 'Human' } };
    expect(buildReferenceControls(args)).toEqual([control()]);
    expect(buildReferenceControls({ ...args, dossiers: [] })[0].records).toEqual([]);
    expect(buildReferenceControls({ ...args, systemIds: [] })).toEqual([]);
    expect(buildReferenceControls({ ...args, controlSources: {} })[0].controllable).toBe(false);
    args.dossiers[0].data.subjects[0].evidence.push({ ...record, text: 'Conflicting record' });
    expect(buildReferenceControls(args)[0].records).toEqual([]);
  });
  it('retains native search/picker focus and sends only an explicit selected hypothesis', () => {
    document.body.innerHTML = '<ph-sensor-references></ph-sensor-references>';
    const panel = document.querySelector('ph-sensor-references');
    const state = { reference_controls: [control()], target_uuid: 'opaque-contact' };
    panel.state = state;
    const search = panel.shadowRoot.querySelector('input'), picker = panel.shadowRoot.querySelector('select');
    search.value = 'held'; search.dispatchEvent(new Event('input')); search.focus();
    panel.state = { ...state, target_references: [{ ...record, attached_at_tick: 10 }] };
    expect(panel.shadowRoot.querySelector('input')).toBe(search);
    expect(panel.shadowRoot.activeElement).toBe(search);
    expect(search.value).toBe('held');
    expect(panel.shadowRoot.querySelector('select')).toBe(picker);
    const activate = vi.fn(); window.activateSemanticAction = activate;
    panel.shadowRoot.querySelector('button').click();
    expect(activate).toHaveBeenCalledWith(SENSORS_ATTACH_REFERENCE_ACTION_ID, { source: 'control', detail: { system_id: 'actual-radar', contact: 'opaque-contact', reference: 'unusual-entry' } });
    panel.state = { ...state, reference_controls: [{ ...control(), controllable: false }] };
    expect(panel.shadowRoot.querySelector('button').disabled).toBe(true);
    delete window.activateSemanticAction;
  });
  it('semantic admission refuses missing/stale owner, record and contact before wire emission', () => {
    let view = { reference_controls: [control()] };
    let sequence = 0;
    const send = vi.fn(), registry = createSensorScienceActionRegistry({ getState: () => view, sendAction: send, actionFeedback: new ActionFeedbackLifecycle({ correlation: () => `reference-test-${++sequence}`, now: () => 123 }) });
    const detail = { system_id: 'actual-radar', contact: 'opaque-contact', reference: 'unusual-entry' };
    const invoke = detail => registry.activate(SENSORS_ATTACH_REFERENCE_ACTION_ID, { context: 'science', detail });
    invoke(detail); expect(send).toHaveBeenCalledTimes(1);
    for (const changed of [{ system_id: 'other' }, { contact: 'stale' }, { reference: 'missing' }]) invoke({ ...detail, ...changed });
    view = { reference_controls: [{ ...control(), controllable: false }] }; invoke(detail);
    expect(send).toHaveBeenCalledTimes(1);
  });
});
