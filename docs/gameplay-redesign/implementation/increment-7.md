# Increment 7: shared mounts and configurable weapons

The user authorised serial implementation of increments 7–18, committing each,
on 14 October 2026. Earlier physical-device and human-workload checks remain
open; this instruction authorises advancement without claiming those results.

Adopt `pasm/spec/design/configurable-weapons.yaml` before runtime changes.
Consolidate the existing beam, straight bolt and guided torpedo strategies onto
common mount geometry, firing-pattern resolution and effect-delivery contracts.
Preserve System identity, Console admission, per-System ordinary AI and combat
parameters. Keep one authoritative lifecycle for each equipped weapon.

Reuse the existing barrel pattern implementation. Beam damage accumulates over
time; bolts retain linear lead prediction; torpedo patterns select barrels while
loaded rounds and burst cadence remain the ammunition/count authority. Common
mechanisms must preserve these distinctions rather than silently rebalance them.

Expose supported authored mechanisms through Workshop editing, history,
validation, export/reload, Preview and disposable Test. Integrate any new mutable
state immediately into snapshots, restore and digests. Read-only definitions and
derived adapters cannot become parallel simulation stores or control paths.

Verify shipped content, human/delegated fire, firing patterns, conservation,
authority, continuation, determinism and authoring. Compare representative
scenario and performance evidence against increment 6 and retain the existing
Cruiser balance thresholds. Record physical/native acceptance separately.

PDC interception, new payloads, shield probing and new engagement styles remain
increment 15; redesigned batteries, coolant and heat begin with increment 8.
