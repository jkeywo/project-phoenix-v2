# Increment 16 — Shared Comms evidence

This is an implementation record, not a verified increment exit. The approved
scope and dependency contract remain in
[`increment-16.md`](../gameplay-redesign/implementation/increment-16.md) and
[`parallel-work.md`](../gameplay-redesign/implementation/parallel-work.md).

## First structural migration: issuing-ship hail history

The existing admitted Hail consumer now processes every fleet hull in stable
identity order and uses that hull's range gate and fitted Comms energy. Its own
history records `(issuing ship, target)`. Clearing a Console or retiring an AI's
old candidacy changes only that ship's history. Urgent hail candidates come from
the ship's delivered unread messages. Existing scripted Hailed events and dialogue
responses retain their existing behaviour.

Model tests exercise source isolation, actor retirement and stable JSON history.
The Simulation proofs exercise the actual hail and clear consumers with two
contradictory LocalShip projections, checking that only the reachable fleet slot
pays work and records the hail. This is an ownership proof; its reach map is a
fixture, so it does not establish actual radio sampling or a full playable mission.

A narrow continuation test JSON-restores a fresh two-ship host, issues the same
hail/clear commands for 360 further steps and compares complete captures/digests.
The finite initial battery permits initial hails and later refusals; the test does
not simulate recharging or establish continuously funded transmission. It also
rejects malformed source identity before any live write. Snapshot format 54 and
simulation rules 0.26 reject older unscoped histories rather than assigning them
to whichever ship happens to be local. Existing transport vocabulary is unchanged.

Targeted execution passes: 3 Model tests and 170 renderer-independent Simulation
tests, with no failures or ignored tests in the passing batches. The initial
Simulation batch passed 161 and failed 9; its missing disabled-event and empty
weapon-intent fixture owners were corrected with assertions retained. PASM
validate/scan/traceability and wiki lint pass. Independent source review passes.
Commands, limitations and the passing log hash are recorded in the
[`checkpoint receipt`](../gameplay-redesign/implementation/comms-checkpoint.json).

## Outstanding increment scope

Shared human/AI text channels, contextual generic requests and returned receipts,
protected/public traffic and captured decoding evidence, equipment-backed relays,
human media, consolidated Comms System tree operation, Workshop editing and
shipped scenario integration remain open. The first migration does not establish
those consumers or unblock electronic operations by itself.

Full integration gates, browser/native device and media operation, activity and
performance comparisons, and 2/4/6/8-human acceptance remain separate pending
evidence. No increment completion, push or human acceptance is claimed here.
