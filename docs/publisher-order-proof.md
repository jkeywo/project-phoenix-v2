# Publisher order proof

The first prepared fixture covers only the three pairs among Power, Shields
and Repair. It does not annotate production conflicts or change the allowance.
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
failure, before deciding whether a precise production annotation is justified.
