// Issue #909 — the browser half of the native↔wasm libm proof.
//
// `src/simmath_vectors.rs` runs the same deterministic vector battery
// (native, in `cargo test`) and the wasm build (here) through every
// `crate::simmath` function *and* through the `nalgebra`/`glam` dependency
// probes, folding every (function, input, output) tuple into one canonical
// digest. Both sides assert against the exact same pinned constant — see
// `EXPECTED_DIGEST`/`EXPECTED_CASE_COUNT` in `simmath_vectors::tests` and
// `wasm_simmath_battery` in the same file. If the two ever disagree, one
// target has drifted off shared libm; that is exactly the class of bug issue
// #908/#909 exists to make impossible to ship unnoticed.
//
// No scenario, no lobby, no peers needed — `wasm_simmath_battery` is a pure
// computation reachable the moment the wasm module is instantiated, so this
// only needs `wasm_init` to have run. It is reachable from the page only
// because `server.html`'s export allowlist promotes it onto `window`
// alongside `wasm_perf_capture` and friends; if this test ever fails with
// "wasm_simmath_battery is not a function", that promotion is what went
// missing, not the Rust export.

import { test, expect, waitForWasmReady } from './fixtures';

// Keep these in lockstep with the two `const`s in
// `src/simmath_vectors.rs::tests` — see that file's doc comment on
// `EXPECTED_DIGEST` for the re-derivation procedure if either changes.
const EXPECTED_DIGEST = 'bbff93332c3b937e';
const EXPECTED_CASE_COUNT = 1300;

test('wasm: simmath vector battery matches the pinned native digest', async ({ context }) => {
  const page = await context.newPage();
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);

  const json = await page.evaluate(() => window.wasm_simmath_battery());
  const result = JSON.parse(json);

  expect(
    result.case_count,
    `wasm battery ran ${result.case_count} cases, native pinned ${EXPECTED_CASE_COUNT} — the ` +
      'battery shape disagrees between targets before comparing a single output',
  ).toBe(EXPECTED_CASE_COUNT);

  expect(
    result.digest,
    'wasm and native disagree on at least one simmath output bit pattern — a transcendental ' +
      'call somewhere is not actually routing through the shared pure-Rust libm on this target',
  ).toBe(EXPECTED_DIGEST);
});
