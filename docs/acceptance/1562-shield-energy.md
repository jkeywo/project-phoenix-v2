# Increment 9: shield energy and protection

Implementation and automated validation are in progress. This is an evidence
record, not a completed playable exit. The adopted contract is
[increment 9](../gameplay-redesign/implementation/increment-9.md).

## Implemented scope

Directional protection strength remains separate from each arc's canonical
local battery. Raised upkeep, actual recharge and accepted recovery credit use
the shared energy and thermal machinery. Partial upkeep permits proportional
interception, including unequal hull/shield payloads and shield pierce. Recovery
removes only strength corresponding to accepted battery charge; reactor intake
and recovery share one intake budget. Recipient strength rebuilds normally.

Stable arc System identities address raised, recovery and focus commands through
ordinary admission. Individual arc state trees consume Console-scoped fine
observations. Shared controls keep strength, battery, heat and effective
protection separate on existing screens. Static equipment uses the same physical
owner and command applier. Damage evidence is produced independently of AI;
voluntary work does not count as incoming damage.

Workshop edits physical coefficients, eligible destinations and closed state
trees in canonical shield-arc source. Includes retain named inheritance and
accepted-source undo/redo. Save format 44 and rules 0.16 retain operational intent,
partial interception and canonical energy state. The integrated redesign uses
protocol 7 and base-content epoch 2 so older clients and unconverted packs fail
the existing compatibility checks explicitly. The transport architecture is
retained.

## Evidence so far

- Before later AI/schedule integration, 93 shield/schema and 16 shared-energy
  tests passed, including accepted-charge conservation. These results do not
  cover the subsequently changed host, scheduling or final content.
- Human control slice `7e92e3b4`: 580 focused JavaScript tests passed. Strict
  strings reported 5,049 rows and no findings; layer validation passed.
- Shield tree authoring slice `cab13f48`: 152 focused JavaScript tests passed;
  layer validation and independent read review passed. New Rust metadata and
  include-composition tests await execution against the combined runtime.
- Integrated controls, physical authoring and tree editor: 33 tests in four
  suites passed against root `f15ef7f0` plus the then-current runtime changes,
  before the later tree-editor commit. A first sandboxed invocation could not
  start tests because temporary-file rename was denied; the normal runner passed.
- Independent read reviews identified and corrected schedule ordering, partial
  payload splitting, static equipment dispatch, target readiness, damage-history
  production and protection after thermal trip. The compiled checks below cover those corrections; broader exit checks remain pending.
- The integrated Gameplay shield filter passed 102 tests, including five static
  equipment/work regressions, partial payload delivery and malformed charge.
  The renderer-independent Simulation compile passed after correcting a child
  module import. A subsequent review tightened the negative fractional-charge
  boundary; the new negative-boundary regression still awaits execution.
- Two compiled ordinary shield-tree tests passed against protocol 7/epoch 2:
  permitted-feed recovery with donor-only authority, and static equipment
  actuation with private-memory suspension under human control. Physical damage
  evidence, strict shipped-hull loading, remaining policy-family comparison and
  required/competing shield-tree load checks also pass.
- The real headless host passed startup, automatic mission launch and exact
  deterministic repeat tests (three selected cases, `--features headless`). The
  new schedule runs successfully. Concurrent fixture apps emit the existing
  process-global logger warning; all assertions pass.
- The new fleet behavioral pin initially supplied no current command-channel
  inputs and was refused by the evaluator. The fixture now supplies the full
  supported interface, including an existing recovery intent in stop cases;
  execution of the corrected fleet pin remains pending. This does not weaken
  input validation or introduce a runtime fallback.

## Remaining exit evidence

1. Compile and exercise the combined runtime, including static equipment, partial
   upkeep, unequal weapon payloads, thermal transitions and conservative recovery.
2. Prove Console feed isolation, donor-only command authority, delegation and
   ordinary AI continuation with the new host; migrate obsolete controller pins.
3. Exercise real save/resume, deterministic replay and reconnect while recovering
   strength or suffering thermal/flow interruption. Reject malformed charge or
   operational state before restoration writes.
4. Run matching-bundle human controls and Workshop edit/undo/export/reload,
   validation, real Preview and disposable Test journeys.
5. Record combat/non-combat activity, performance and the unchanged published
   balance thresholds. Separate deliberate gameplay changes from regressions.
6. Record actual phone/native-monitor operation and human workload separately
   from automated results. Complete PASM checks and current-runtime wiki lint.

Increment 8's pinned validation continues independently. Its results do not
establish increment 9's compatibility or gameplay behavior.
