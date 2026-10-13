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

Runtime commit `efac36b7`. [Validation receipts](../gameplay-redesign/implementation/composition-validation.json)
record 9,060 passing native tests (102 ignored), 8,688 passing JavaScript tests,
Clippy, formatting, layer boundaries, generated surfaces, strict strings, LODs,
PASM and wiki checks. Renderer-independent digest tests pass (59).

Four browser cases pass: no-Dock hull, real Cruiser docking, live Engineering
power loss/recovery in the open status popup, and Destroyer/Cruiser Umbilical
transfer. Keyboard opening, Escape and focus return pass. The rendered Workshop
journey passes invalid composition rejection, component/channel editing,
undo/redo, export/reload and disposable Test. Browser transport uses the existing
shim; it does not prove remote-network operation.

Visual inspection corrected literal names and Close button styling and verified
the resulting popup at 844×390 and 1280×720. The phone-sized check is limited to
status/dialog usability. Existing client connection diagnostics overlap the
landscape phone Dock button; real Dock operation was checked at desktop size.
This unresolved layout issue prevents a full phone acceptance claim.

Independent review covered model, Dock runtime/continuation, Workshop composition,
status privacy/accessibility and final browser fixtures; findings were corrected.
[Gameplay comparison](../gameplay-redesign/implementation/composition-baseline.md)
passes: both full bounded reports match Increment 1 exactly; replay/continuation
agree; the Cruiser wins 20/20 and passes all 16 unchanged thresholds. Automated
evidence is complete. Physical/device acceptance remains open, so this does not
close the milestone or start Increment 3.

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
