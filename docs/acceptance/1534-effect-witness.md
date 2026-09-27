# Continuous direct-effect witness (#1534)

The divergence browser runner installs a private observer on both GM pages
before submitting its one correlated 5 HP damage action. It reads the existing
public GM activity projection every 50 ms. It does not change production
messages, the ring capacity, or the reducer. The original baseline still needs
one actual gm.direct damage event on the requested entity, a retained earlier
tick, and the same Applied canonical journal order on both GMs.

The observer in scripts/fleet-effect-witness.mjs keeps the previous full ring
and counts matching damage events with multiplicity. Identical repeated page
reads add nothing; two equal damage rows are two events. A new projection may
append rows at the previous latest tick or at newer ticks. Previously complete
ticks must be unchanged. Ring eviction requires a strictly earlier complete
overlapping tick; partial-tick truncation, lost overlap, retroactive insertion,
rewritten rows, backward history, and changed capacity permanently fail proof.
The observer retains the actual damage row after its original ring entry is
wholly evicted. A later matching damage event fails even if its amount differs.

Observation is independently bounded to 3,000 samples, 8 MiB of audit records,
and 120 seconds. A callback gap over 1,000 ms or clock rewind fails. The runner
retains summary samples during recovery and the complete bounded observation
trace under recovery.effectObserverLogs, including timestamps, removed prefix
counts and newly observed rows. The helper hash is included in provenance.
Observers stop in the divergence hook's finally block; a final observer failure
cannot turn into a passing recovery result.

These limits are deliberately conservative. A busy publication can evict part
of a tick between samples, even if the software behaved correctly; that run
fails because this projection cannot prove continuity. Sampling does not grant
permission to infer missed events from a canonical journal entry. No live
recovery pass is established by the helper's automated tests. The remaining
#1534 runtime matrix and source-matched reruns remain required.
