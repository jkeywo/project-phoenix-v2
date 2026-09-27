import { CONTINUATION_LIMITS, createContinuationJournal, reconcileContinuation } from './fleet-continuation.js';

// This private envelope travels only on the authenticated server namespace.
export const CONTINUATION_WIRE = 'phoenix-fleet-continuation-v1';
export const continuationEnvelope = (kind, body) => JSON.stringify({ continuation: CONTINUATION_WIRE, kind, body });
export const isContinuationEnvelope = value => value?.continuation === CONTINUATION_WIRE;

/** One owner-loss transaction. Network ownership remains in fleet-session. */
export function createOwnerContinuation({ local, owner, participants, request, deliver, replayFrame = null,
  send, onHeld = () => {}, onCommit = () => {}, onError = () => {} }) {
  const journal = createContinuationJournal({ local, participants });
  let phase = 'live', epoch = 0, successor = null, plan = null;
  let survivors = participants.filter(slot => slot !== owner);
  const tails = new Map(), acknowledgements = new Map(), buffered = [];
  let beginPromise = null;
  let eligible = true;
  let replayedStatus = null;
  const pendingEgress = [];
  let pendingEgressBytes = 0, bufferedBytes = 0;
  const refuse = error => {
    phase = 'refused';
    const reason = error instanceof Error ? error.message : String(error);
    onError('owner-continuation-refused', reason);
    return false;
  };
  const checked = async (value, expected) => {
    const result = await request(value);
    if (!result || result.status !== expected) throw new Error(result?.reason || `continuation-${expected}-refused`);
    return result;
  };
  const outgoing = (slot, kind, body) => send(slot, continuationEnvelope(kind, body));
  async function replay(targetPlan) {
    if (phase !== 'held') throw new Error('continuation-not-held');
    phase = 'replaying';
    journal.replay(targetPlan, (raw, source) => {
      if (!replayFrame || replayFrame(raw, source, epoch) === false) throw new Error('continuation-replay-ingress-refused');
    });
    const result = await checked({ op: 'replayed', epoch }, 'replayed');
    if (!Number.isSafeInteger(result.loss_tick) || result.loss_tick < 1) throw new Error('missing-continuation-watermark');
    phase = 'replayed';
    replayedStatus = { epoch, frontier: targetPlan.frontier, loss_tick: result.loss_tick };
    return replayedStatus;
  }
  async function commit(body) {
    if (phase !== 'replayed' || body.epoch !== epoch || body.previous_owner !== owner
      || body.next_owner !== successor || JSON.stringify(body.acked) !== JSON.stringify(survivors)
      || body.loss_tick !== replayedStatus?.loss_tick
      || JSON.stringify(body.frontier) !== JSON.stringify(replayedStatus?.frontier)) {
      throw new Error('invalid-continuation-commit');
    }
    await checked({ op: 'commit', epoch, previous_owner: owner, next_owner: successor,
      loss_tick: body.loss_tick, acked: survivors }, 'committed');
    journal.commit({ departed: owner, frontier: body.frontier });
    owner = successor;
    phase = 'live';
    onCommit({ owner, epoch, departed: body.previous_owner, loss_tick: body.loss_tick });
    bufferedBytes = 0;
    for (const item of buffered.splice(0)) {
      const raw = api.receive(item.envelope, item.origin);
      if (raw) {
        if (item.onDeferred) item.onDeferred(raw);
        else deliver(raw, owner);
      }
    }
    pendingEgressBytes = 0;
    for (const raw of pendingEgress.splice(0)) {
      const wire = api.broadcast(raw);
      if (!wire) continue;
      if (local === owner) {
        for (const slot of survivors) if (slot !== local) send(slot, wire);
      } else send(owner, wire);
    }
  }
  async function acceptAck(slot, body) {
    if (local !== successor || !survivors.includes(slot) || body.epoch !== epoch
      || JSON.stringify(body.frontier) !== JSON.stringify(plan?.frontier)) throw new Error('invalid-continuation-ack');
    acknowledgements.set(slot, body.loss_tick);
    if (acknowledgements.size !== survivors.length) return;
    const values = [...acknowledgements.values()];
    if (!values.every(value => value === values[0])) throw new Error('continuation-watermark-disagreement');
    const bodyCommit = { epoch, previous_owner: owner, next_owner: successor,
      loss_tick: values[0], acked: survivors, frontier: plan.frontier };
    // Ordered channels put commit before any resumed frame to each survivor.
    for (const slot of survivors) if (slot !== local) outgoing(slot, 'commit', bodyCommit);
    await commit(bodyCommit);
  }
  async function acceptTail(slot, tail) {
    if (local !== successor || phase !== 'held' || tail.local !== slot || !survivors.includes(slot)) throw new Error('invalid-continuation-tail-source');
    if (tails.has(slot)) throw new Error('duplicate-continuation-tail');
    tails.set(slot, tail);
    if (tails.size !== survivors.length) return;
    plan = reconcileContinuation({ participants, departed: owner, coordinator: local, tails: [...tails.values()] });
    for (const target of plan.targets) if (target.local !== local) {
      outgoing(target.local, 'replay', { epoch, departed: owner, frontier: plan.frontier, targets: [target] });
    }
    await acceptAck(local, await replay(plan));
  }
  const api = {
    get phase() { return phase; },
    get epoch() { return epoch; },
    get owner() { return owner; },
    get successor() { return successor; },
    get participants() { return [...participants]; },
    frontier() { return journal.frontier(); },
    updateRoster(active, baseline=null) {
      if (phase !== 'live') return;
      if (active.some(slot => !participants.includes(slot))) eligible = false;
      participants = [...new Set([...participants,...active])].sort((a,b)=>a-b);
      survivors = active.filter(slot => slot !== owner).sort((a,b)=>a-b);
      journal.sync(active, baseline);
    },
    broadcast(raw) {
      if (phase === 'refused') return null;
      if (['held', 'replaying', 'replayed'].includes(phase)) {
        const bytes = raw.length * 2;
        if (pendingEgress.length >= CONTINUATION_LIMITS.frames
            || pendingEgressBytes + bytes > CONTINUATION_LIMITS.bytes) {
          refuse('continuation-egress-overflow'); return null;
        }
        pendingEgress.push(raw); pendingEgressBytes += bytes;
        return null;
      }
      // During pending begin, Rust can still drain already minted frames. They
      // enter the tail even though the failed transport cannot forward them.
      let row;
      try { row = journal.record(raw); } catch (error) { refuse(error); return null; }
      return phase === 'live' ? continuationEnvelope('stream', row) : null;
    },
    receive(envelope, origin, onDeferred = null) {
      if (phase === 'refused') return null;
      if (phase === 'replaying' || phase === 'replayed') {
        const bytes = (envelope?.raw?.length || 0) * 2;
        if (buffered.length >= 64 || bufferedBytes + bytes > CONTINUATION_LIMITS.bytes) return refuse('continuation-pending-overflow');
        bufferedBytes += bytes;
        buffered.push({ envelope, origin, onDeferred });
        return null;
      }
      try { return journal.receive(envelope, origin); } catch (error) { refuse(error); return null; }
    },
    begin(nextEpoch) {
      if (beginPromise) return beginPromise;
      if (!eligible) return Promise.resolve(refuse('continuation-after-membership-change-unavailable'));
      if (phase !== 'live' || nextEpoch !== epoch + 1 || !survivors.includes(local)) return Promise.resolve(false);
      phase = 'pending'; epoch = nextEpoch; successor = Math.min(...survivors);
      beginPromise = (async () => {
        await checked({ op: 'begin', epoch, previous_owner: owner, next_owner: successor, participants: survivors }, 'held');
        journal.hold(); phase = 'held'; onHeld(); return true;
      })().catch(refuse);
      return beginPromise;
    },
    async connected() {
      if (!await beginPromise || phase !== 'held') return false;
      try {
        if (local === successor) await acceptTail(local, journal.tail());
        else outgoing(successor, 'tail', journal.tail());
        return true;
      } catch (error) { return refuse(error); }
    },
    async control(kind, body, source) {
      try {
        if (kind === 'tail') await acceptTail(source, body);
        else if (kind === 'replay') {
          if (source !== successor || body.epoch !== epoch || body.departed !== owner) throw new Error('invalid-continuation-replay');
          outgoing(successor, 'replayed', await replay(body));
        } else if (kind === 'replayed') await acceptAck(source, body);
        else if (kind === 'commit') {
          if (source !== successor) throw new Error('invalid-continuation-coordinator');
          await commit(body);
        } else throw new Error('unknown-continuation-control');
        return true;
      } catch (error) { return refuse(error); }
    },
  };
  return api;
}
