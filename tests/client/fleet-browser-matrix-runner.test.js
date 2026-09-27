import { describe, expect, it } from 'vitest';
import { optionsFrom, verifyEvidence, verifyImpairment } from '../../scripts/fleet-browser-matrix.mjs';

function evidence(route = 'direct') {
  const state = (i = 0) => ({ fleet: { role: i < 4 ? 'ship' : 'gm' }, phase: 'InProgress', mesh: { slot: i + 1, in_fleet: true, peers: [1, 2, 3, 4, 5], peers_heard: [1, 2, 3, 4, 5], samples: 1, agreed: true },
    outcomes: [{ correlation: 'matrix-1-captain-0', outcome: 'Applied' }],
    relayFrames: 20, signalOffersSent: route === 'ws-relay' ? 0 : 1, relayReady: route === 'direct' ? 0 : 1,
    rtc: route === 'direct' ? [{ connectionState: 'connected', selected: [{ state: 'succeeded', localType: 'host', remoteType: 'host', bytesReceived: 10 }] }] : [] });
  return { route, commandWaves: 1, peers: Array.from({ length: 6 }, (_, i) => ({ label: `peer-${i}`, state: state(i) })),
    clients: Array.from({ length: 12 }, (_, i) => ({ label: `client-${i}`, state: state() })), gmActions: ['pause', 'resume'] };
}
describe('real browser matrix evidence gates', () => {
  it.each(['direct', 'ws-relay', 'automatic-fallback'])('accepts complete observed %s evidence', route => expect(verifyEvidence(evidence(route))).toBe(true));
  it('refuses an intended direct route without any received traffic', () => {
    const row = evidence(); row.clients[0].state.rtc[0].selected[0].bytesReceived = 0;
    expect(() => verifyEvidence(row)).toThrow('no observed direct');
  });
  it('refuses a TURN candidate under the direct pin', () => {
    const row = evidence(); row.peers[1].state.rtc[0].selected[0].remoteType = 'relay';
    expect(() => verifyEvidence(row)).toThrow('no observed direct');
  });
  it('requires every Station document to receive an applied command outcome', () => {
    const row = evidence(); row.clients[11].state.outcomes = [];
    expect(() => verifyEvidence(row)).toThrow('no applied command');
  });
  it('refuses a workload with a rejected action even when another applied', () => {
    const row = evidence(); row.clients[2].state.outcomes.push({ correlation: 'matrix-3-helm-1', outcome: 'Refused' });
    expect(() => verifyEvidence(row)).toThrow('workload command refused');
  });
  it('refuses missing peers and a disagreement', () => {
    const row = evidence(); row.peers[0].state.mesh.agreed = false;
    expect(() => verifyEvidence(row)).toThrow('simulation disagreement');
    row.peers.pop(); expect(() => verifyEvidence(row)).toThrow('six real peers');
  });
  it('requires an attempted RTC connection before claiming automatic fallback', () => {
    const row = evidence('automatic-fallback'); row.peers[1].state.signalOffersSent = 0;
    expect(() => verifyEvidence(row)).toThrow('never attempted RTC');
  });
  it('requires real roles, distinct slots and an exchanged digest', () => {
    const row = evidence(); row.peers[5].state.fleet.role = 'ship';
    expect(() => verifyEvidence(row)).toThrow('roles');
    row.peers[5].state.fleet.role = 'gm'; row.peers[5].state.mesh.slot = 1;
    expect(() => verifyEvidence(row)).toThrow('slots');
    row.peers[5].state.mesh.slot = 6; row.peers[5].state.mesh.samples = 0;
    expect(() => verifyEvidence(row)).toThrow('digest exchange');
  });
  it('requires all command waves and refuses browser exceptions', () => {
    const row = evidence(); row.commandWaves = 2;
    expect(() => verifyEvidence(row)).toThrow('receipts incomplete');
    row.commandWaves = 1; row.peers[0].errors = ['uncaught'];
    expect(() => verifyEvidence(row)).toThrow('browser exception');
  });
  it('checks the owner has every direct link', () => {
    const row = evidence(); row.peers[0].label = 'ship-1';
    expect(() => verifyEvidence(row)).toThrow('no observed direct');
    row.peers[0].state.rtc = Array.from({length: 8}, () => structuredClone(row.peers[1].state.rtc[0]));
    expect(verifyEvidence(row)).toBe(true);
  });
  it('bounds runtime and rejects ambiguous route/port selection', () => {
    expect(() => optionsFrom(['--out', 'unused', '--seconds', '301'])).toThrow('seconds');
    expect(() => optionsFrom(['--out', 'unused', '--routes', 'direct,direct'])).toThrow('distinct');
    expect(() => optionsFrom(['--out', 'unused', '--port', '18431'])).toThrow('ports must differ');
  });
});

describe('impairment evidence gates', () => {
  const options = {delayMs: 20, lossPercent: 10, seed: 1530};
  const relay = () => ({route: 'ws-relay', impairment: {profile: {delay_ms: 20, loss_percent: 10, seed: 1530}, counters: {relay_reliable_written: 10, relay_snapshot_written: 8, relay_snapshot_dropped: 2, relay_frames_delayed: 18, observed_write_delay_ms: {reliable: {count: 10, min: 20}, snapshot: {count: 8, min: 20}}}}});
  it('requires observed writes, delay and loss with the exact profile', () => {
    expect(verifyImpairment(relay(), options)).toBe(true);
    const row = relay(); row.impairment.counters.relay_snapshot_dropped = 0;
    expect(() => verifyImpairment(row, options)).toThrow('no observed drop');
    row.impairment.profile.seed = 42; expect(() => verifyImpairment(row, options)).toThrow('differs');
  });
  it('rejects queued delay whose observed writes happened too early', () => {
    const row = relay(); row.impairment.counters.observed_write_delay_ms.reliable.min = 2;
    expect(() => verifyImpairment(row, options)).toThrow('observed write delays');
  });
  it('requires actual suppressed offers for fallback even without impairment', () => {
    expect(() => verifyImpairment({route: 'automatic-fallback', impairment: {profile: {block_rtc_offers: true}, counters: {signal_offers_dropped: 0}}}, {delayMs: 0, lossPercent: 0})).toThrow('suppressed');
  });
});
