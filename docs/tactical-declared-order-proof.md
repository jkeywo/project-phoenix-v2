# Tactical behavior under declared order (#1400)

The Tactical guard retains its three live cases: a human retarget, a human clear,
and weapon destruction followed by AI reacquisition. Every existing gameplay
assertion remains: the same-tick fire consumes the prior published combat lock,
both weapon families really fire, damage is attributed to the controlled ship,
the first hostile is actually destroyed in the reacquisition case, and the second
hostile then takes damage. Full per-tick observations and each command target's
admitted subsequence must agree.

The production Input sequence now orders the four observed owners:

`ai_phaser_auto_fire` → `ai_target_selection` → `handle_set_target` →
`tick_blaster_auto_fire`.

The five exact `ambiguous_with` annotations remain in production. Their earlier
opposed-order evidence established logical commutativity, but the complete
declared policy also orders those owners through their other conflicts. Forcing
either old test permutation would contradict that policy. The current guard
therefore changes actual plugin registration through the shared `SimFixture`:
canonical, `Shuffled(17)` and `Shuffled(991)`. It adds no ordering edges, runtime
systems or access declarations. Each case and registration runs in fresh
default/default/pinned processes, for 27 children in the registration parent;
the existing nine-child canonical baseline parent remains.

The build observer reads the actual four instances before Bevy moves them into
its executable. It retains all five exact raw access vectors, requires selection
before its applier, and checks each declared direction against the frozen
pre-change owner ranks. It also retains every raw external conflict and compares
the remaining unordered subset with Bevy's final census. A zero unordered subset
does not discard those physical conflicts. The shared graph proof retains actual
instances and hierarchy, distinguishes repeated functions using their concrete
ancestor metadata, and preserves existing execution and deferred-visibility
paths across genuinely different registration inventories.

Run the complete binary on the integrated scheduler:

```powershell
cargo test --features headless --test tactical_target_ordering -- --nocapture --test-threads=1
```

The two parent tests launch the ignored exact child themselves. Each child checks
the actual seed, compute-worker count and executor, and the registration parent
checks its process identity. Source formatting and inspection are complete;
native execution on the final integrated scheduler is pending. This proof covers
the preserved Tactical observations and command subsequences; it does not add a
claim about every shared command producer or every wire projection.
