# Increment 2: composable Systems and common status

Implementation reference: [contract](../gameplay-redesign/implementation/increment-2.md).

## Implemented scope

Active Dock uses typed mating and power-input components and declared boolean/
entity-target channels. Existing Dock/Undock controls and individual AI remain
System-addressed. All five shipped active Dock definitions use components;
passive berths keep their endpoint declarations. Tuning is unchanged.

The canonical channels, mating state and undock clearance survive save/restore
and participate in deterministic digests. Protocol 6, snapshot 38 and rules
0.10 establish explicit compatibility boundaries. Legacy active `[dock]`
authoring produces a conversion error.

All Console screens share current status indicators and keyboard-accessible
live details. Owned passive equipment is listed; delegated equipment uses a
summary. Current damage and Dock power/rig conditions supply reasons; unavailable
targets and historical refusals do not become equipment failures. Cross-Console
feeds do not grant internal hardware knowledge or command authority.

Workshop edits components, ordering, typed fields and channel bindings as one
validated history operation. Include overrides use System-local identities;
inherited composition requires explicit materialisation. Unsupported or
incomplete compositions cannot enter Preview/Test.

## Evidence

Automated and browser results are being collected. No playable exit is claimed
until the integrated checks and browser journeys pass. Review covered model,
Dock runtime/continuation, Workshop composition, status privacy and accessibility;
findings were corrected before final checks.

## Human/device acceptance

Physical phones, native multi-monitor operation and human workload judgement
remain pending. Increment 1's outstanding checks remain pending too; the user's
instruction to continue authorised Increment 2 without rewriting that history.

For human acceptance, operate the Cruiser Helm against a passive berth, inspect
Dock status while Engineering lowers/restores its power, delegate and reclaim
the System, and verify its details remain usable at the device's text size.
Check that the compact Equipment disclosure does not obscure ordinary controls.
Use Workshop to change the component range and channel bindings, undo/redo,
save/reload and enter disposable Test; confirm the original session is intact.
