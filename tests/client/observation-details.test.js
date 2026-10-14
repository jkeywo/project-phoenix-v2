// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { consoleReadState } from '../../gui/console-topology.js';
import { buildRepairConsoleState } from '../../gui/console-state.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { t } from '../../gui/strings.js';
import { observationRows, observationDetails, observationTickFor } from '../../gui/observation-details.js';
import '../../gui/components/ph-scan-readout.js';
import '../../gui/components/ph-dossier-panel.js';
import '../../gui/components/ph-repair-teams.js';
const metadata = () => ({ subject: { kind: 'entity', uuid: 'target' }, provenance: { kind: 'scan', system: 'scanner' }, observed_tick: 10, received_tick: 90, uncertainty: { kind: 'quantized', field: 'condition_fraction', step: .25 } });
const values = (m, tick, fields) => Object.fromEntries(observationRows(m, tick, fields).map(([key, , value]) => [key, value]));
const mount = tag => { document.body.innerHTML = `<${tag}></${tag}>`; return document.querySelector(tag); };

describe('common observation details', () => {
  it('keeps absent sample/precision unknown and treats tick zero as a real observation', () => {
    expect(values(null, 100)).toMatchObject({ sample: t('observation.unknown'), age: t('observation.unknown'), precision: t('observation.unknown') });
    expect(values({ observed_tick: 0 }, 25).age).toBe(t('observation.ticks', { ticks: 25 }));
    expect(values({ received_tick: 25 }, 30).age).toBe(t('observation.unknown'));
  });
  it('held and forwarded samples age only from the supplied logical tick', () => {
    const m = metadata(), before = structuredClone(m);
    expect(values(m, 100).age).toBe(t('observation.ticks', { ticks: 90 }));
    expect(values({ ...m, received_tick: 999 }, 100).age).toBe(values(m, 100).age);
    expect(values(m, 120).age).toBe(t('observation.ticks', { ticks: 110 }));
    expect(values(m, undefined).age).toBe(t('observation.unknown')); expect(m).toEqual(before);
  });
  it('does not leak unknown/hidden measurement precision or invent probability', () => {
    const m = metadata(); expect(values(m, 100).precision).toBe(t('observation.unknown'));
    expect(values(m, 100, { condition_fraction: 'component.scan.condition' }).precision).toContain('0.25');
    m.uncertainty = { kind: 'quantized', field: 'secret_strength', step: 12345 };
    const el = observationDetails(document, m, 100, { condition_fraction: 'component.scan.condition' });
    expect(el.textContent).not.toContain('12345'); expect(el.textContent).not.toContain('secret_strength');
    m.uncertainty = { kind: 'qualitative' }; expect(values(m, 100).precision).toBe(t('observation.qualitative'));
  });
  it('keeps report metadata inside its typed Console feed and removes it on revocation', () => {
    const record = { group: { type: 'Core' }, observations: [metadata()] };
    const state = { observationTick: 100, blackboards: { repair: { queue_depth: [record] } }, blackboardKinds: { repair: 'Repair' } };
    const definition = { id: 'reader', systems: [], feeds: [{ source: 'repair', kind: 'Repair' }] };
    const read = () => buildRepairConsoleState(consoleReadState(state, definition));
    expect(read().reports).toEqual([record]); expect(read().observation_tick).toBe(100);
    definition.feeds[0].kind = 'Sensors'; expect(read().reports || []).toEqual([]);
    definition.feeds = []; expect(read().reports || []).toEqual([]);
    expect(state.blackboards.repair.queue_depth).toEqual([record]);
  });
  it('renders untrusted source as text, never markup', () => {
    const m = metadata(); m.provenance.system = '<img src=x onerror=alert(1)>';
    const el = observationDetails(document, m, 100); expect(el.querySelector('img')).toBeNull(); expect(el.textContent).toContain(m.provenance.system);
  });
  it('Scan details preserve held reading and disclosure across tick updates, then clear with reading', () => {
    const panel = mount('ph-scan-readout');
    const reading = { subject_uuid: 'target', condition_fraction: .5, condition_step: .25, mass: 3, observation: metadata() };
    panel.state = { scan: { capable: true, reading, observation_tick: 100 } }; panel.render(panel.state);
    panel.shadowRoot.querySelector('details').open = true;
    panel.state = { scan: { capable: true, reading, observation_tick: 120 } }; panel.render(panel.state);
    expect(panel.shadowRoot.querySelector('details').open).toBe(true);
    expect(panel.shadowRoot.querySelector('[data-observation="age"]').textContent).toBe(t('observation.ticks', { ticks: 110 }));
    panel.state = { scan: { capable: true, reading: null } }; panel.render(panel.state);
    expect(panel.shadowRoot.querySelector('details')).toBeNull();
  });
  it('Dossier evidence uses its own metadata and never fills missing sample from receipt', () => {
    const panel = mount('ph-dossier-panel');
    panel.state = { observation_tick: 100, dossiers: [{ uuid: 'target', evidence: [{ text: 'observation.details', observation: { received_tick: 90, uncertainty: { kind: 'qualitative' } } }] }] };
    panel.select('target');
    expect(panel.shadowRoot.querySelector('[data-observation="age"]').textContent).toBe(t('observation.unknown'));
  });
  it('Repair shows permitted contributor metadata without numeric deficit or new controls', () => {
    const panel = mount('ph-repair-teams');
    panel.state = { teams: [], observation_tick: 100, reports: [{ group: { type: 'Console', data: 'helm' }, station_label: 'observation.source.system_report', deficit: 98765, observations: [metadata(), { provenance: { kind: 'repair_team' }, observed_tick: 20 }] }] };
    expect(panel.shadowRoot.querySelectorAll('details')).toHaveLength(2);
    expect(panel.shadowRoot.textContent).not.toContain('98765');
    panel.state = { teams: [], reports: [] }; expect(panel.shadowRoot.querySelectorAll('details')).toHaveLength(0);
  });
});


describe('logical observation clock', () => {
  it('withholds ages for old held boards until a matching-generation publication arrives', () => {
    const state = new ClientSimState();
    state.blackboardPresentation.scan = 1;
    state.apply({ type: 'SimState', data: { snapshot: { observation_tick: 5, observation_generation: 2 } } });
    expect(observationTickFor(state, 'scan')).toBeNull();
    state.apply({ type: 'BlackboardUpdate', data: { updates: [['scan', { kind: 'Scan', data: { capable: true } }]], console_scope: ['scan'], console_scope_tick: 5, presentation_generation: 2 } });
    expect(observationTickFor(state, 'scan')).toBe(5);
    expect(values({ observed_tick: 2 }, observationTickFor(state, 'scan')).age).toBe(t('observation.ticks', { ticks: 3 }));
  });
  it('ignores stale snapshot clocks and rebases only with a new existing presentation generation', () => {
    const state = new ClientSimState();
    const snapshot = (tick, generation) => state.apply({ type: 'SimState', data: { snapshot: { observation_tick: tick, observation_generation: generation } } });
    snapshot(100, 2); snapshot(80, 2); expect(state.observationTick).toBe(100);
    state.apply({ type: 'BlackboardUpdate', data: { updates: [], console_scope: [], console_scope_tick: 5, presentation_generation: 3 } });
    expect(state.observationTick).toBeNull(); snapshot(200, 2); expect(state.observationTick).toBeNull();
    snapshot(5, 3); expect(state.observationTick).toBe(5);
    snapshot(2, 4);
    state.apply({ type: 'BlackboardUpdate', data: { updates: [], console_scope: ['old-record'], console_scope_tick: 900, presentation_generation: 3 } });
    expect(state.consoleScopeGeneration).toBe(3); expect(state.observationTick).toBe(2);
    state.apply({ type: 'GameStarted', data: {} }); expect(state.observationTick).toBeNull();
    state.apply({ type: 'SimState', data: { snapshot: {} } }); expect(state.observationTick).toBeNull();
  });
});
