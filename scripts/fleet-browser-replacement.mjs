// #1534 real-browser fixed-slot replacement race, plugged into afterHealthy.
// This helper observes production Rust egress and drives the public host action.
// Fixture tests of its evidence gate are not runtime acceptance evidence.
export function observeReplacement() {
  const evidence = window.__replacementEvidence = { frames: [], overflow: false };
  const take = window.wasm_take_mesh_frames;
  if (typeof take !== 'function') throw new Error('Missing production mesh egress');
  window.wasm_take_mesh_frames = function(...args) {
    const raw = take(...args);
    // The production codec already encodes digests as exact 16-digit hex strings.
    for (const frame of JSON.parse(raw)) {
      if (!['digest', 'slot-claim'].includes(frame.t)) continue;
      if (evidence.frames.length === 2000) evidence.overflow = true;
      else evidence.frames.push(frame);
    }
    return raw;
  };
}

const integer = value => Number.isSafeInteger(value) && value >= 0;
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const slotsOf = rows => rows.map(row => row.mesh?.slot).sort((a, b) => a - b);
const admitted = (row, slot) => row?.fleet?.open === true && row.mesh?.in_fleet === true && row.mesh.slot === slot;
const refused = row => row?.fleet?.open === false && row.fleet.reason === 'slot-taken' && row.mesh?.in_fleet === false;

function checkpoints(peers, afterTick) {
  const ticks = new Map();
  let contradictory = false;
  for (const peer of peers) for (const frame of peer.frames || []) {
    if (frame.t !== 'digest' || frame.d?.from !== peer.mesh?.slot || !integer(frame.d.tick) || frame.d.tick <= afterTick) continue;
    if (typeof frame.d.digest !== 'string' || !/^[0-9a-f]{16}$/.test(frame.d.digest)) { contradictory = true; continue; }
    if (!ticks.has(frame.d.tick)) ticks.set(frame.d.tick, new Map());
    const row = ticks.get(frame.d.tick);
    if (row.has(peer.label) && row.get(peer.label) !== frame.d.digest) contradictory = true;
    row.set(peer.label, frame.d.digest);
  }
  const common = [...ticks].filter(([, rows]) => rows.size === 6)
    .map(([tick, rows]) => ({ tick, byPeer: Object.fromEntries(rows), agreed: new Set(rows.values()).size === 1 }));
  return { common, contradictory };
}

function routeObserved(evidence) {
  const transport = evidence.winnerTransport;
  if (!transport) return false;
  const connected = (transport.rtc || []).filter(peer => peer.connectionState === 'connected');
  const received = peer => peer.selected?.some(pair => pair.state === 'succeeded' && pair.bytesReceived > 0
    && ['host', 'srflx', 'prflx'].includes(pair.localType) && ['host', 'srflx', 'prflx'].includes(pair.remoteType));
  if (evidence.route === 'direct') return !transport.relayReady && connected.length === 1 && received(connected[0]);
  if (!['ws-relay', 'automatic-fallback'].includes(evidence.route)) return false;
  return transport.relayReady > 0 && transport.relayFrames > 0 && connected.length === 0
    && (evidence.route === 'automatic-fallback' ? transport.signalOffersSent > 0 : transport.signalOffersSent === 0);
}

export function replacementOutcome(evidence) {
  const before = evidence.before || [], candidates = evidence.race || [], after = evidence.after || [];
  const slot = evidence.victim?.slot;
  const expected = before.filter(row => row.label !== evidence.victim?.label);
  const winners = candidates.filter(row => admitted(row, slot));
  const losers = candidates.filter(refused);
  const winner = winners.length === 1 ? winners[0] : null;
  const live = winner && after.find(row => row.label === winner.label);
  const restore = live?.mesh?.recovery?.replacement;
  const boundary = restore?.boundary_tick, sequence = restore?.claim_seq;
  const originalSlots = slotsOf(before);
  const baseline = before.length === 6 && expected.length === 5
    && new Set(before.map(row => row.label)).size === 6 && new Set(originalSlots).size === 6
    && originalSlots.every(integer) && integer(slot)
    && before.every(row => row.mesh?.in_fleet && row.phase === 'InProgress' && row.fleet?.frozen);
  const lossRows = evidence.disconnected || [];
  const losses = lossRows.map(row => row.mesh?.recovery?.losses?.find(loss => loss.slot === slot));
  const disconnected = lossRows.length === 5 && new Set(lossRows.map(row => row.label)).size === 5
    && lossRows.every(row => expected.some(peer => peer.label === row.label && peer.mesh.slot === row.mesh?.slot))
    && losses.every(loss => loss && integer(loss.tick)) && new Set(losses.map(loss => loss?.tick)).size === 1
    && lossRows.every(row => !row.mesh.peers.includes(slot)
      && row.mesh.recovery.ships.some(ship => ship.slot === slot && ship.crewed === false));
  const starts = candidates.map(row => row.attempt?.startedMs);
  const raced = candidates.length === 2 && new Set(candidates.map(row => row.label)).size === 2
    && starts.every(integer) && candidates.every(row => row.attempt.result?.ok === true)
    && Math.abs(starts[0] - starts[1]) <= 250;
  const oneWinner = raced && winners.length === 1 && losers.length === 1;
  const recovered = restore?.result === 'recovered' && restore.slot === slot
    && integer(boundary) && integer(sequence) && boundary > losses[0]?.tick;
  const restoredRoster = after.length === 6 && new Set(after.map(row => row.label)).size === 6
    && same(slotsOf(after), originalSlots) && !!live
    && expected.every(row => after.some(peer => peer.label === row.label && peer.mesh?.slot === row.mesh.slot));
  const recoveryAgreed = restoredRoster && recovered && after.every(row => {
    const r = row.mesh?.recovery?.replacement;
    return r?.slot === slot && r.boundary_tick === boundary && r.claim_seq === sequence
      && r.leader === restore.leader
      && r.result === (row === live ? 'recovered' : row.mesh.slot === restore.leader ? 'led' : 'witnessed');
  });
  const claims = after.flatMap(row => (row.frames || []).filter(frame => frame.t === 'slot-claim').map(frame => ({ observer: row.mesh?.slot, ...frame.d })));
  const oneClaim = claims.length === 1 && claims[0].slot === slot && claims[0].claim_seq === sequence
    && claims[0].from === claims[0].observer && before.some(row => row.fleet?.owner && row.mesh?.slot === claims[0].from);
  const advancing = restoredRoster && after.every(row => row.phase === 'InProgress' && row.mesh?.in_fleet
    && row.mesh.tick > boundary + 30 && row.mesh.agreed === true && !row.mesh.disagreement
    && same([...row.mesh.peers].sort((a,b) => a-b), originalSlots.filter(slot => slot !== row.mesh.slot)));
  const protectedHolder = !!winner && evidence.challenger?.label === losers[0]?.label && refused(evidence.challenger)
    && integer(evidence.challenger?.challengeAttempt?.startedMs)
    && evidence.challenger.challengeAttempt.startedMs > Math.max(...starts)
    && evidence.challenger.challengeAttempt.result?.ok === true
    && admitted(live, slot) && integer(evidence.challengeTick) && live.mesh.tick > evidence.challengeTick + 30;
  const exact = checkpoints(after, Math.max(boundary ?? Infinity, evidence.challengeTick ?? Infinity));
  const agreement = !exact.contradictory && exact.common.length >= 2 && exact.common.every(row => row.agreed);
  const routeVerified = routeObserved(evidence);
  const overflow = [...before, ...lossRows, ...candidates, ...after, evidence.challenger].filter(Boolean).some(row => row.overflow);
  return { baseline, disconnected, raced, oneWinner, winner: winner?.label, recovered, boundary, sequence,
    restoredRoster, recoveryAgreed, oneClaim, advancing, protectedHolder, routeVerified, commonDigests: exact.common, agreement, overflow,
    passed: baseline && disconnected && oneWinner && recoveryAgreed && oneClaim && advancing && protectedHolder && agreement && routeVerified && !overflow };
}

export function replacementHook({ faultSeconds = 90 } = {}) {
  if (!Number.isInteger(faultSeconds) || faultSeconds < 1 || faultSeconds > 180) throw new Error('Invalid replacement faultSeconds');
  return async ({ result, ships, gms, newPage, base, query, code, step }) => {
    const peers = [...ships.map((page, index) => ({ page, label: `ship-${index + 1}` })),
      ...gms.map((page, index) => ({ page, label: `gm-${index + 1}` }))];
    const read = async peer => ({ label: peer.label, ...await peer.page.evaluate(() => ({
      mesh: window.__hostMeshStatus(), fleet: window.__hostFleetState(), phase: window.__saveSlotsPhase,
      ...window.__replacementEvidence,
    })) });
    await Promise.all(peers.map(peer => peer.page.evaluate(observeReplacement)));
    const candidates = await Promise.all([0, 1].map(async index => {
      const peer = { page: await newPage(`replacement-${index + 1}`), label: `replacement-${index + 1}` };
      await peer.page.goto(`${base}/?${query}&scenario=assets/worlds/probe_fleet_six_peer.toml&ship=assets/entities/alliance_cruiser.toml`);
      await peer.page.waitForFunction(() => window.__matrixPhoenixReady && window.__matrixEvidence.counts.hosted > 0);
      await peer.page.evaluate(observeReplacement);
      return peer;
    }));
    const before = await Promise.all(peers.map(read));
    const victim = peers[1], victimState = before.find(peer => peer.label === victim.label);
    const survivors = peers.filter(peer => peer !== victim);
    const evidence = result.recovery = { failure: 'replacement', route: result.route, before,
      victim: { label: victim.label, slot: victimState.mesh.slot }, startedUtc: new Date().toISOString(), samples: [] };
    const deadline = Date.now() + faultSeconds * 1000;
    const bounded = async (promise, label) => {
      const remaining = deadline - Date.now();
      if (remaining <= 0) throw new Error(`Replacement deadline before ${label}`);
      let timer;
      try { return await Promise.race([promise, new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`Replacement deadline during ${label}`)), remaining);
      })]); } finally { clearTimeout(timer); }
    };
    const poll = async (label, action) => {
      do {
        if (await bounded(action(), label)) return;
        await bounded(new Promise(resolve => setTimeout(resolve, 250)), label);
      } while (Date.now() < deadline);
      throw new Error(`Replacement deadline during ${label}`);
    };
    await bounded(victim.page.close(), 'victim close');
    step(`closed ${victim.label} before fixed-slot replacement race`);
    await poll('agreed loss and Backfill', async () => {
      evidence.disconnected = await Promise.all(survivors.map(read));
      evidence.outcome = replacementOutcome(evidence);
      return evidence.outcome.disconnected;
    });
    const startAt = Date.now() + 750;
    await bounded(Promise.all(candidates.map(peer => peer.page.evaluate(async ({ code, slot, startAt }) => {
      await new Promise(resolve => setTimeout(resolve, Math.max(0, startAt - Date.now())));
      window.__replacementEvidence.attempt = { startedMs: Date.now() };
      window.__replacementEvidence.attempt.result = await window.__hostFleetJoin(code, `slot-${slot}`);
    }, { code, slot: evidence.victim.slot, startAt }))), 'concurrent claims');
    await poll('one admission and one explicit refusal', async () => {
      evidence.race = await Promise.all(candidates.map(read));
      const winners = evidence.race.filter(row => admitted(row, evidence.victim.slot));
      if (winners.length > 1) throw new Error('Both replacements entered the same fixed slot');
      return winners.length === 1 && evidence.race.filter(refused).length === 1;
    });
    const winner = candidates.find(peer => peer.label === evidence.race.find(row => admitted(row, evidence.victim.slot)).label);
    const loser = candidates.find(peer => peer !== winner);
    const restored = [...survivors, winner];
    await poll('canonical replacement restore', async () => {
      evidence.after = await Promise.all(restored.map(read));
      evidence.outcome = replacementOutcome(evidence);
      return evidence.outcome.recoveryAgreed && evidence.outcome.advancing;
    });
    evidence.challengeTick = Math.max(...evidence.after.map(row => row.mesh.tick));
    await bounded(loser.page.evaluate(async ({ code, slot }) => {
      window.__replacementEvidence.challengeAttempt = { startedMs: Date.now() };
      window.__replacementEvidence.challengeAttempt.result = await window.__hostFleetJoin(code, `slot-${slot}`);
    }, { code, slot: evidence.victim.slot }), 'connected-slot challenge');
    step('race winner restored; loser attempted to displace the connected holder');
    await poll('protected holder and exact post-challenge digests', async () => {
      evidence.challenger = await read(loser);
      evidence.winnerTransport = await winner.page.evaluate(async () => {
        const { rtc, relayReady, relayFrames, signalOffersSent } = await window.__matrixRead();
        return { rtc, relayReady, relayFrames, signalOffersSent };
      });
      evidence.after = await Promise.all(restored.map(read));
      evidence.outcome = replacementOutcome(evidence);
      if (evidence.outcome.overflow) throw new Error('Replacement observer overflow');
      if (evidence.samples.length < 720) evidence.samples.push({ at: Date.now(), outcome: evidence.outcome });
      return evidence.outcome.passed;
    });
    evidence.finishedUtc = new Date().toISOString();
    step('one fixed-slot winner, canonical restore, protected holder and two matching checkpoints');
  };
}

