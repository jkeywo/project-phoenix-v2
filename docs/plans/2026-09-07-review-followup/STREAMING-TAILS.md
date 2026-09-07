# Bounded streaming-tail follow-up

Source base: `6e77d705153fc145e103663103e0cd46ed549abe`. The original named
headless results in [RESULTS.md](RESULTS.md) measured Combat streaming bodies
at 13.68–17.27 ms and subsequent marker-sync bodies at 3.31–5.14 ms on six
repeatable first-minute ticks. They did not measure the clone or sidecar
sub-operations separately.

The follow-up changes two bounded paths:

- Gameplay and cosmetic asteroid spawning call the existing single-template
  accessor shared by native and WASM, instead of copying every cached template
  for each rock. The current entry is still read for every spawn.
- One authoritative marker-sync invocation reuses resolved geometry for the
  same exact sidecar path. This temporary lookup ends at function return.
  Variants remain distinct, each entity keeps its own transform, candidate
  order and immediate insertion remain intact, and no work moves out of
  PreUpdate/FixedLast. Missing or malformed sidecars retain their existing
  fallback; pending WASM fetches are retried on the next invocation.

No persistent marker cache, gameplay tuning, cell scheduling, identity change,
or presentation policy is introduced. Content must remain stable during one
synchronous invocation; later invocations still read the current source for
new entities. Existing entities retain their already-attached markers.

## Validation pending integration

The new integration regression runs the installed asteroid lifecycle through
missing-template fallback, delivery and replacement. It checks authoritative
collider/health/radar/mesh values, both cosmetic layers, and identities,
positions and order when revisiting the same cells. It owns a separate process
because the native config cache is global. Marker tests check multiple
entities sharing a rig, distinct variants and transforms, and missing,
malformed, delivered and replaced sidecars across successive invocations.
Existing marker fixed-boundary, profile parity, streaming-window and snapshot
continuation tests remain applicable.

Rust execution, WASM compilation and before/after performance measurements are
pending the integrator's compiler and quiet-machine slots. Source formatting
alone does not demonstrate runtime correctness or a speedup.

Preparation checks passed on this worktree diff: independent production/test
source review, direct Rust formatting/check, `git diff --check`, and 88 local
wiki/index/follow-up links and source paths. The existing pinned PASM executable
ran `validate`, `scan` and `traceability`, all exit 0: validation reports
859 entities and 216 informational findings with `Status: OK`; traceability
reports 239 rows. These are static checks, not Rust execution.

## Minimal controlled measurement

Build clean base and candidate artifacts with the same release profile,
toolchain, features, content, seed 42 and Alliance Destroyer. Run three paired
120-second Combat captures, alternating arm order, plus three paired 60-second
Falling Skyway captures as a non-streaming control. Retain all raw tick samples,
validity/background receipts and matching final digests. Compare per-tick
streaming tails, counts above the fixed-step budget, maxima and p99 without
pooling unrelated run percentiles. Do not assume the old 194a digests remain
the right reference for this newer base; compare the two newly built arms.

If ordinary release results justify adoption, one before/after named Combat
pair can confirm attribution at the same streaming ticks; named observer
overhead remains separate from release timing. A fixed-boundary/continuation
failure rejects the change regardless of faster timings. Native 3D frame-rate
and Station smoothness claims require their own evidence and are not outcomes
of this bounded headless comparison.
