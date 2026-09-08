# Publisher order proof

The fixture covers only the three pairs among Power, Shields and Repair.
Production marks precisely these three pairs with `ambiguous_with`.
The integrated annotated source passed targeted execution and comparison with
its unannotated baseline. It leaves the allowance unchanged.
The initial rebased native baseline passed all twelve children. A subsequent
run exposed an independent Comms producer/drain race during setup: identical
reliable Comms state arrived on tick 10 or 11. The follow-up orders Comms and
ObjectiveSummary before the simulation outbox drain and adds a same-tick host
transition assertion. The corrected native run passed all twelve children and
their parent, retaining the full setup and 24-tick comparisons.

`tests/publisher_ordering.rs` uses the ordinary headless graph and the same
production system instances. A build observer checks actual mutable access and
effective paths; empty test-only sets impose forward, reverse and rotated orders
inside the existing Publish boundary. Ordinary order remains a separate arm.
Every other incident conflict vector and external effective edge is retained.

Power inserts its registered Power, reactor and battery keys. Shields inserts
its aggregate and arc keys and reads the prior Viewscreen key. Repair inserts
its own aggregate key. None of these writers replaces another's key or changes
the Viewscreen value. Their physical mutable map access remains incompatible:
Bevy continues serializing it. The annotation declares order independence for
these three pairs; it does not permit concurrent mutation of the map. The live
proof requires the entire overlap to remain exactly `ShipSystemBlackboards`,
checks all three diagnostics are absent even in ordinary order, and retains
all external conflict vectors and multiplicities. A new overlap fails that
proof instead of inheriting the annotation's permission silently.

The parent starts twelve fresh children: four orders multiplied by two default
pool runs and one deterministic pinned run. Each child checks its actual seed,
pool and executor, uses at least two authored ships, changes Power allocation,
Shield focus and hull/Repair state through their real state APIs, and observes
six setup ticks and 24 subsequent advancing ticks. The fixture exercises missing then existing map entries,
the previous Viewscreen lock at Shields publication, the next aggregate lock,
whole maps, underlying state, actual message payloads/delivery classes and
blackboard/Hull caches, including both maps in the recipient Repair cache.
All setup and subsequent observations must agree across children. Typed wire
assertions require the selected Power and Shields publications, Engineering's
damaged-hull Repair projection, Science's explicit withheld projection and
changed Hull aggregates for both recipients. Unrelated HUD messages cannot
satisfy those checks; stationary repair-state observations also fail.

The 2026-09-08 R4 run used base `a57bcca4` plus this fixture and the Comms drain
ordering fix, with `host,headless,perf,ultralight`, six build jobs, and offline
locked dependencies. The source inventory was unchanged across execution
(`3E6C2D893A3ABD0DFC83686830AAB6DFBAB98174F2BA063FD699A424F2CA567E`).
Fresh-process reports and complete traces are retained in the local
`.t2-batch/rebase-focused-native-r4/` receipt. The same run passed 176 Comms
library tests, both three-process reconnect comparisons, projectile storage
ordering and the live schedule graph/debt checks. These are focused native
results, not the ordinary headless or final pre-push matrix.

This is source-state/publication testing, not a replacement for input admission
or physical-console acceptance. It does not yet cover Power's legacy resource
fallback or every external-repair candidate state. The other nineteen disjoint
publishers and their 228 additional pairs remain outside this fixture. The
shared helper keeps owner names and pair identities explicit so meaningful
additional owners can join the same binary without replacing the graph.

Dock remains excluded: it writes an authored instance key, whose disjointness
from aggregate keys is not established by the current config validator. The
four AI-side blackboard-only pairs are also excluded. No name-prefix exception,
raw-count-only claim or whole-PRD completion is supported by this preparation.

After independent source review and a coordinated compiler grant, the focused
binary is `cargo test --locked --offline --features headless --test publisher_ordering`.
The integrator may use the already warmed SDK feature configuration and must
record that configuration separately from the ordinary headless gate. Preserve
source-bound build/child outcomes, full traces and graph receipts, including any
failure. An adoption must compare the annotated source to a current unannotated
baseline, retaining all graph nodes and edges and removing exactly the three
named full conflict vectors. Full setup and gameplay/wire/cache traces must
match across that boundary as well as between pool/order roles.

## Annotated integration evidence

The next integration rebased the three T2 continuation commits onto local main
`ec2d7fd08c823d3ce9afbdd276551903a26437d7`, producing unannotated worktree
`5a3686e6faecfa85e6a7e1b2f5a68d23c82fdc2d`. The baseline and annotated candidate
each rebuilt their own source-bound library with locked offline dependencies,
six jobs and `host,headless,perf,ultralight`. Their full local receipts are in
`.t2-batch/publisher-adoption-baseline-r1/` and
`.t2-batch/publisher-adoption-candidate-r1/`.

Both stages passed 22 schedule diagnostics and both explicit captures. The
baseline passed twelve publisher children and nine admitted-producer children;
the candidate passed twelve publisher children and nine foreign-consumer
children. Each parent also passed. All candidate publisher setup observations,
24 advancing ticks, complete wire/cache observations and child graph reports
match the corresponding baseline in every order and pool role. The rebased
producer fixture retains nine identical complete 380-tick traces. These checks
do not replace the ordinary headless or full pre-push gates.

The live census falls from 1,543 to 1,540 rows: precisely the three named
`ShipSystemBlackboards` vectors disappear, with no added or other removed row.
All 576 graph nodes retain their complete metadata. Every actual system ID is
unchanged. Five unique SystemTypeSet IDs are assigned differently because the
Power annotation interns Shields' publisher type set earlier. Matching those
five sets by complete metadata gives a bijection; all 527 hierarchy edges,
321 authored dependency edges and 2,565 effective dependency edges match under
that mapping. Raw captures retain the original IDs. The graph reports conflict
vectors and flags, not every standalone ECS access; source review and the
actual publisher compatibility assertions establish the unchanged access here.

Two local tooling failures are retained with their corrections. The initial
baseline wrapper expected a single summary from a twelve-child parent after all
thirteen tests had passed; a continuation checked the aggregate totals and ran
only the remaining producer tests, without rebuilding or repeating passed tests.
The first graph comparison treated capture-local type-set IDs as stable; the
corrected comparison preserves all metadata and edges through the unique set
mapping. Neither correction changes runtime source or weakens a gameplay check.
PASM validate, scan and traceability passed for the recorded policy.
