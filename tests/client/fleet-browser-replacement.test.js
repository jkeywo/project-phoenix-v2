// Synthetic fixtures validate the gate only; no browser acceptance is claimed.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { observeReplacement, replacementHook, replacementOutcome, runReplacementPhase, REPLACEMENT_PHASE_SECONDS } from '../../scripts/fleet-browser-replacement.mjs';

const label = slot => slot <= 4 ? `ship-${slot}` : `gm-${slot - 4}`;
function peer(slot) {
  return { label: label(slot), phase: 'InProgress', fleet: { open: true, frozen: true, owner: slot === 1 },
    mesh: { slot, in_fleet: true, tick: 200, peers: [1,2,3,4,5,6].filter(other => other !== slot),
      agreed: true, disagreement: null, recovery: { losses: [], ships: [1,2,3,4].map(slot => ({ slot, crewed: true })) } },
    frames: [], overflow: false };
}
function evidence(route = 'direct') {
  const before = [1,2,3,4,5,6].map(peer);
  const disconnected = before.filter(row => row.mesh.slot !== 2).map(row => {
    row = structuredClone(row);
    row.mesh.peers = row.mesh.peers.filter(slot => slot !== 2);
    row.mesh.recovery.losses = [{ slot: 2, tick: 250 }];
    row.mesh.recovery.ships[1].crewed = false;
    return row;
  });
  const winner = { ...peer(2), label: 'replacement-1', attempt: { startedMs: 1000, result: { ok: true } } };
  const loser = { label: 'replacement-2', fleet: { open: false, reason: 'slot-taken' }, mesh: { in_fleet: false },
    attempt: { startedMs: 1008, result: { ok: true } } };
  const after = [...before.filter(row => row.mesh.slot !== 2), winner].map(row => {
    row = structuredClone(row);
    row.mesh.tick = 1300;
    row.mesh.recovery.replacement = { slot: 2, leader: 1, claim_seq: 1, boundary_tick: 600,
      result: row.mesh.slot === 2 ? 'recovered' : row.mesh.slot === 1 ? 'led' : 'witnessed' };
    row.frames = [900,1200].map(tick => ({ t: 'digest', d: { from: row.mesh.slot, tick, digest: 'fedcba9876543210' } }));
    return row;
  });
  after[0].frames.push({ t: 'slot-claim', d: { from: 1, slot: 2, claim_seq: 1, tick: 300 } });
  const winnerTransport = route === 'direct'
    ? { relayReady: 0, rtc: [{ connectionState: 'connected', selected: [
      { state: 'succeeded', bytesReceived: 42, localType: 'host', remoteType: 'host' },
    ] }] }
    : { relayReady: 1, relayFrames: 10, rtc: [], signalOffersSent: route === 'automatic-fallback' ? 1 : 0 };
  return { route, before, disconnected, victim: { label: 'ship-2', slot: 2 }, race: [winner, loser], after,
    challengeTick: 640, challenger: { ...structuredClone(loser), challengeAttempt: { startedMs: 1100, result: { ok: true } } },
    winnerTransport };
}

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

describe('bounded replacement evidence gate', () => {
  it.each(['direct', 'ws-relay', 'automatic-fallback'])('accepts complete %s fixture evidence', route => {
    expect(replacementOutcome(evidence(route)).passed).toBe(true);
  });
  it('fails closed with missing evidence', () => expect(replacementOutcome({}).passed).toBe(false));
  const reject = (name, change, field) => it(name, () => {
    const row = evidence(); change(row);
    expect(replacementOutcome(row)[field]).toBe(false);
    expect(replacementOutcome(row).passed).toBe(false);
  });
  reject('requires the full frozen original fleet', row => row.before[0].fleet.frozen = false, 'baseline');
  reject('requires observed same-tick Backfill on every survivor', row => row.disconnected[1].mesh.recovery.losses[0].tick++, 'disconnected');
  reject('does not accept a transport departure with stale ship crew', row => row.disconnected[4].mesh.recovery.ships[1].crewed = true, 'disconnected');
  reject('rejects duplicate survivor labels in loss evidence', row => row.disconnected[1].label = row.disconnected[0].label, 'disconnected');
  reject('rejects serial claims masquerading as a race', row => row.race[1].attempt.startedMs += 1000, 'raced');
  reject('requires submitted production join attempts', row => row.race[1].attempt.result.ok = false, 'raced');
  reject('rejects two admitted replacements', row => { row.race[1] = { ...structuredClone(row.race[0]), label: 'replacement-2' }; }, 'oneWinner');
  reject('requires a specific losing refusal', row => row.race[1].fleet.reason = 'transport-closed', 'oneWinner');
  reject('requires restore, not merely transport admission', row => row.after[5].mesh.recovery.replacement.result = 'no-valid-record', 'recovered');
  reject('requires one common recovery boundary', row => row.after[1].mesh.recovery.replacement.boundary_tick++, 'recoveryAgreed');
  reject('requires one common claim sequence', row => row.after[1].mesh.recovery.replacement.claim_seq++, 'recoveryAgreed');
  reject('rejects added slots', row => row.after[1].mesh.slot = 20, 'restoredRoster');
  reject('rejects changed survivor identities', row => row.after[1].label = 'unknown', 'restoredRoster');
  reject('requires one owner-minted claim, including after the challenge', row => row.after[0].frames.push(structuredClone(row.after[0].frames.at(-1))), 'oneClaim');
  reject('rejects a claim emitted by a non-owner', row => { row.after[1].frames.push(row.after[0].frames.pop()); }, 'oneClaim');
  reject('requires a fresh late attempt rather than the old refusal', row => delete row.challenger.challengeAttempt, 'protectedHolder');
  reject('requires explicit late refusal', row => row.challenger.fleet.reason = 'version-mismatch', 'protectedHolder');
  reject('requires the admitted winner to remain connected', row => row.after[5].fleet.open = false, 'protectedHolder');
  reject('requires resumed ticks after the challenge', row => row.after[5].mesh.tick = row.challengeTick, 'protectedHolder');
  reject('requires all six local digests at both later ticks', row => row.after[4].frames.pop(), 'agreement');
  reject('rejects a differing final digest', row => row.after[4].frames[0].d.digest = 'fedcba9876543211', 'agreement');
  reject('rejects digest numbers even if JS has rounded them equally', row => row.after.forEach(peer => peer.frames.filter(frame => frame.t === 'digest').forEach(frame => frame.d.digest = 18446744073709551615)), 'agreement');
  reject('cannot hide a contradictory duplicate digest', row => row.after[0].frames.push({ t: 'digest', d: { from: 1, tick: 900, digest: '0000000000000001' } }), 'agreement');
  reject('cannot use stale pre-challenge checkpoints', row => row.challengeTick = 1250, 'agreement');
  reject('rejects observer overflow even with matching retained data', row => row.after[0].overflow = true, 'passed');
  reject('does not infer the winning route from the requested route', row => row.winnerTransport.rtc[0].selected[0].bytesReceived = 0, 'routeVerified');
  it('requires a real RTC attempt for automatic fallback', () => {
    const row = evidence('automatic-fallback'); row.winnerTransport.signalOffersSent = 0;
    expect(replacementOutcome(row).passed).toBe(false);
  });
  it('bounds the supplied fault duration', () => {
    for (const faultSeconds of [0,181,Infinity,1.5]) expect(() => replacementHook({ faultSeconds })).toThrow('faultSeconds');
  });
});

describe('replacement observer', () => {
  it('returns untouched production bytes and retains exact high-bit hex digests', () => {
    const raw = '[{"t":"digest","d":{"from":2,"tick":900,"digest":"ffffffffffffffff"}},{"t":"tick","d":{"commands":[{"private":"not-collected"}]}}]';
    const take = vi.fn(() => raw);
    vi.stubGlobal('window', { wasm_take_mesh_frames: take });
    observeReplacement();
    expect(window.wasm_take_mesh_frames('argument')).toBe(raw);
    expect(take).toHaveBeenCalledWith('argument');
    expect(window.__replacementEvidence.frames).toEqual([{ t: 'digest', d: { from: 2, tick: 900, digest: 'ffffffffffffffff' } }]);
    expect(JSON.stringify(window.__replacementEvidence)).not.toContain('private');
  });
  it('marks overflow without changing traffic or retaining unbounded evidence', () => {
    const raw = JSON.stringify(Array.from({ length: 2001 }, (_, tick) => ({ t: 'digest', d: { from: 1, tick, digest: 'ffffffffffffffff' } })));
    vi.stubGlobal('window', { wasm_take_mesh_frames: () => raw });
    observeReplacement();
    expect(window.wasm_take_mesh_frames()).toBe(raw);
    expect(window.__replacementEvidence.frames).toHaveLength(2000);
    expect(window.__replacementEvidence.overflow).toBe(true);
  });
});



describe('replacement phase deadlines and provenance', () => {
  const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
  it('gives admission, restore and challenge independent budgets and retains observed milestones', async () => {
    vi.useFakeTimers(); vi.setSystemTime(1000000);
    const row = {};
    const durations = { loss: 1000, admission: 86000, restore: 20000, challenge: 86000, digests: 20000 };
    const task = (async () => {
      for (const [name, seconds] of Object.entries(REPLACEMENT_PHASE_SECONDS)) {
        await runReplacementPhase(row, name, seconds, async ({ bounded }) => {
          await bounded(() => wait(durations[name]), 'actual observation');
          return { observed: name };
        });
      }
    })();
    await vi.advanceTimersByTimeAsync(213000); await task;
    expect(row.phases.map(p => p.status)).toEqual(Array(5).fill('passed'));
    expect(row.phases.map(p => p.elapsedMs)).toEqual(Object.values(durations));
    expect(row.phases.at(-1).finishedMs - row.phases[0].startedMs).toBe(213000);
    row.phases.forEach((p, index) => {
      expect(p.deadlineMs - p.startedMs).toBe(REPLACEMENT_PHASE_SECONDS[p.name] * 1000);
      expect(p.startedUtc).toBe(new Date(p.startedMs).toISOString());
      expect(p.finishedUtc).toBe(new Date(p.finishedMs).toISOString());
      expect(p.milestone).toEqual({ observed: p.name });
      if (index) expect(p.startedMs).toBe(row.phases[index - 1].finishedMs);
    });
  });
  it.each(Object.keys(REPLACEMENT_PHASE_SECONDS))('records a hung %s operation as timed out and does not enter the next phase', async name => {
    vi.useFakeTimers(); vi.setSystemTime(10000);
    const row = {}, later = vi.fn();
    const task = (async () => {
      await runReplacementPhase(row, name, 1, async ({ bounded }) => {
        await bounded(() => new Promise(() => {}), 'blocked browser read');
      });
      await runReplacementPhase(row, 'later', 1, later);
    })();
    const rejected = expect(task).rejects.toThrow('deadline');
    await vi.advanceTimersByTimeAsync(1000); await rejected;
    expect(later).not.toHaveBeenCalled();
    expect(row.phases).toHaveLength(1);
    expect(row.phases[0]).toMatchObject({ name, status: 'timed-out', startedMs: 10000,
      finishedMs: 11000, elapsedMs: 1000, budgetMs: 1000, lastOperation: 'blocked browser read' });
    expect(row.phases[0].milestone).toBeUndefined();
  });
  it('does not renew the deadline when unsuccessful observations keep arriving', async () => {
    vi.useFakeTimers(); vi.setSystemTime(0);
    const row = {}, observe = vi.fn(() => false);
    const task = runReplacementPhase(row, 'restore', 1, ({ poll }) => poll('canonical agreement', observe));
    const rejected = expect(task).rejects.toThrow('deadline');
    await vi.advanceTimersByTimeAsync(1000); await rejected;
    expect(row.phases[0].status).toBe('timed-out');
    expect(observe).toHaveBeenCalledTimes(4);
    await vi.advanceTimersByTimeAsync(2000);
    expect(observe).toHaveBeenCalledTimes(4);
    expect(row.phases[0].finishedMs).toBe(1000);
  });
  it('retains a concrete refusal instead of relabelling it a timeout', async () => {
    const row = {};
    await expect(runReplacementPhase(row, 'admission', 1, () => { throw new Error('Both replacements entered'); }))
      .rejects.toThrow('Both replacements entered');
    expect(row.phases[0]).toMatchObject({ status: 'failed', error: 'Error: Both replacements entered' });
  });
  it('bounds admission configuration independently from the recovery budget', () => {
    for (const admissionSeconds of [0,181,Infinity,1.5]) {
      expect(() => replacementHook({ admissionSeconds })).toThrow('admissionSeconds');
    }
  });
});
