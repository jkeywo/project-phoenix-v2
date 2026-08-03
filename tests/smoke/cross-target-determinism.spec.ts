// Issue #904 — the browser half of the native↔wasm lockstep proof.
//
// PRD #849's user story asks for "two instances under artificial delay". Two
// NATIVE instances is the one configuration that cannot detect the divergence
// most likely to occur: they share a build, a backend and a libm, so they
// agree with each other while disagreeing with the browser and the P2P mesh
// desyncs anyway. This spec is the configuration that can detect it.
//
// What runs where:
//
//   native  `tests/cross_target_probe.rs` drives `cross_target_probe::run_probe`
//           under EVEN pacing (one logical tick per App::update) and pins the
//           resulting checkpoint ledger to tests/fixtures/cross-target-ledger.json.
//   wasm    this spec calls `window.wasm_cross_target_probe()`, which drives the
//           IDENTICAL seeded world under BURSTY pacing (1, 2, 3, 4, 2 ticks per
//           update — the artificial delay) inside the shipped wasm build.
//
// Same seed, same command log, different frame pacing, different instruction
// set. Every checkpoint digest and the final digest must be bit-identical.
//
// This spec reads the pinned ledger from the fixture file rather than carrying
// its own copy of the numbers: one place to re-bless, and no chance of the two
// halves drifting apart silently. See the module docs on
// `tests/cross_target_probe.rs` for the re-bless procedure.
//
// `wasm_cross_target_probe` is reachable from the page only because
// `server.html`'s export allowlist promotes it onto `window` alongside
// `wasm_simmath_battery` and friends; if this test ever fails with
// "wasm_cross_target_probe is not a function", that promotion is what went
// missing, not the Rust export.

import { readFileSync } from 'fs';
import path from 'path';

import { test, expect, waitForWasmReady, captureServerPageErrors } from './fixtures';

type Checkpoint = { tick: number; digest: string };
type ProbeReport = {
  seed: number;
  ticks: number;
  interval: number;
  pacing: string;
  checkpoints: Checkpoint[];
  final_digest: string;
};

const LEDGER_PATH = path.resolve(__dirname, '../fixtures/cross-target-ledger.json');

/**
 * The first tick at which two ledgers disagree — the TypeScript twin of
 * `DigestLedger::first_divergence`.
 *
 * AC3 is explicit that the first divergent tick must be REPORTED, not merely
 * that a difference is detected. Pairing by tick rather than by index is what
 * makes that possible when the two instances sampled different tick sets;
 * here they should sample identical sets (every pacing cycle divides the
 * checkpoint interval), and a tick present on only one side is itself worth
 * naming, so it is reported rather than skipped.
 */
function firstDivergence(
  pinned: ProbeReport,
  observed: ProbeReport,
): { tick: number; after: number | null; pinnedDigest: string; observedDigest: string } | null {
  const byTick = new Map(observed.checkpoints.map((c) => [c.tick, c.digest]));
  let after: number | null = null;
  for (const checkpoint of pinned.checkpoints) {
    const seen = byTick.get(checkpoint.tick);
    if (seen === undefined) {
      return {
        tick: checkpoint.tick,
        after,
        pinnedDigest: checkpoint.digest,
        observedDigest: '<not sampled by wasm>',
      };
    }
    if (seen !== checkpoint.digest) {
      return { tick: checkpoint.tick, after, pinnedDigest: checkpoint.digest, observedDigest: seen };
    }
    after = checkpoint.tick;
  }
  if (pinned.final_digest !== observed.final_digest) {
    return {
      tick: pinned.ticks,
      after,
      pinnedDigest: pinned.final_digest,
      observedDigest: observed.final_digest,
    };
  }
  return null;
}

test('wasm: a delayed browser instance folds the same digests as the native pin', async ({
  context,
}) => {
  const pinned = JSON.parse(readFileSync(LEDGER_PATH, 'utf8')) as ProbeReport;

  const page = await context.newPage();
  const errors = captureServerPageErrors(page);
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);

  const started = Date.now();
  const json = await page.evaluate(
    () => (window as any).wasm_cross_target_probe() as string,
  );
  const elapsedMs = Date.now() - started;
  const observed = JSON.parse(json) as ProbeReport & { error?: string };

  expect(observed.error, `the wasm probe returned an error instead of a report`).toBeUndefined();

  // Wall-clock guard, kept as a soft assertion rather than a comment that
  // rots. The in-page run blocks the browser's main thread for `PROBE_TICKS`
  // App::update() calls; measured at **74 ms** for 240 ticks in release
  // Chromium on the authoring machine, against this file's 60 s spec timeout.
  // The ceiling below is therefore ~135x the measurement — generous enough to
  // survive a loaded CI runner, tight enough that a probe world which grows
  // into a multi-second stall is noticed while there is still headroom rather
  // than after it starts flaking.
  expect(
    elapsedMs,
    `the in-page probe took ${elapsedMs} ms for ${pinned.ticks} ticks — if the probe world has ` +
      'grown, shrink PROBE_TICKS or the world before this starts timing out on a loaded CI runner',
  ).toBeLessThan(10_000);

  // Shape first: comparing digests between runs of different lengths, seeds or
  // sampling intervals would be comparing two different simulations and
  // calling the difference a divergence.
  expect(observed.seed, 'wasm ran a different seed than the pin').toBe(pinned.seed);
  expect(observed.ticks, 'wasm ran a different number of ticks than the pin').toBe(pinned.ticks);
  expect(observed.interval, 'wasm sampled at a different interval than the pin').toBe(
    pinned.interval,
  );
  expect(
    observed.checkpoints.length,
    'wasm sampled a different number of checkpoints than the pin',
  ).toBe(pinned.checkpoints.length);

  // The delay is real, and asserted rather than assumed: if the browser ever
  // ran EVEN pacing this whole spec would collapse into a same-pacing,
  // cross-target check and quietly stop covering AC5.
  expect(
    observed.pacing,
    'the browser must run the BURSTY pacing — the comparison has to span injected delay as ' +
      'well as target, or it is only half of what #904 asks for',
  ).toBe('bursty');
  expect(pinned.pacing, 'the pinned native ledger must be the EVEN-paced run').toBe('even');

  const divergence = firstDivergence(pinned, observed);
  expect(
    divergence,
    divergence
      ? `native and wasm first disagree at tick ${divergence.tick}` +
          (divergence.after === null
            ? ' (they disagreed at the very first checkpoint)'
            : ` (last agreement at tick ${divergence.after})`) +
          `: native pinned ${divergence.pinnedDigest}, wasm produced ${divergence.observedDigest}.\n` +
          'The two targets have stopped simulating the same world. Do NOT re-bless the fixture ' +
          'from the browser numbers — that would erase the finding. Read ' +
          'tests/cross_target_probe.rs for what each half is claiming, and check the usual ' +
          'suspects first: a transcendental that stopped routing through crate::simmath ' +
          '(#908/#909), rapier regaining the `parallel` feature on native (#896), or a schedule ' +
          'whose order is a function of frame pacing (#895).'
      : 'native and wasm agree',
  ).toBeNull();

  expect(errors, `the page reported errors while running the probe: ${errors.join('; ')}`).toEqual(
    [],
  );
});
