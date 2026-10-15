# Increment 14: sensing and environmental interaction

Adopted implementation contract prepared against main `ee2edf3e`. This is
the selected scope, not a claim of a playable exit. Source implementation began
after increment 13's integrated checkpoint and targeted physical-work and
continuation checks. Late increment 13 facility/recovery validation continues on
its pinned worktree under the authorised pipeline. The migration ledger records
that advancement; outstanding validation remains required.

## Outcome and complete selected scope

Make information an observation that equipment acquires and crews interpret.
Broad sensing maintains routine awareness while an independently powered focused
scanner investigates a useful question. Passive reception, history, uncertainty,
correlation and deliberate sharing create ways to improve a decision without
requiring contact-by-contact scan administration. Regions and substantial terrain
affect actual signal paths; neither hides networking failures nor grants AI a
different view of the world.

The [selected review](../system-review.md), [System catalogue](../ship-systems.md),
[AD69–71](../architecture/README.md#ad69--shared-spatial-fields-for-environments-and-equipment-selected-a)
and [focused PASM slice](../../../pasm/spec/design/sensing-and-environment.yaml)
govern this increment. Research informs examples, not additional requirements.

| Selected requirements | Required delivery or explicit dependency |
|---|---|
| G016–18; S056–57 | Relevant operational radars receive permitted basic returns. Automatic broad assessment progresses through supported identification/classification/assessment, with confidence by finding. No ordinary contact-count update budget, Captain map, complete dossier or inferred intent. |
| G019/G106; S097/S099 | Last observations, automatically predicted motion and growing position uncertainty; bearing-only evidence where appropriate. Confidence radii and a few meaningful alternative hypotheses. Viewscreen ambiguity uses generic wireframes, never an unknown hull's real mesh. Estimation is integrated into relevant sensors, without another computer System or compulsory arithmetic. |
| G020; S058–59 | Separate broad/focused Systems and resources. One active enquiry per focused scanner; its ordinary AI manages priorities. General assessment is the simple default, with optional equipment-supported aims. Intrusion occupies that same slot only on capable equipment and has detectable exposure/contextual consequences; no guaranteed protected answer. |
| G021; S060/S098 | Independently fitted passive receivers operate concurrently, using actual emissions with uncertainty. Directional/fixed/steerable equipment reuses weapon mount/arc/traversal mechanisms. Passive is neither perfect stealth nor automatic identification/decryption. |
| G022–23; S061–63 | Explicit user-selected Navigation/Sensors/Tactical faction feeds; recipients retain authority. Compatible actually received ship/probe observations may improve triangulation with useful geometry. Copies are not independent evidence. Restored links expose retained findings through normal history, preserving original sample time. Routine known-reference matching is automatic; unusual records require deliberate finding/flagging for attachment. |
| S062/S064 | Responsive retrieval/correlation of held information without an analysis-capacity pool. Ordinary AI has deterministic reading time before using relevant text. Environmental/resource/anomaly enquiries provide evidence-based conditional forecasts with visible assumptions and uncertainty. |
| G028–31; AD69–71 | Upgrade Regions in place for falloff, analytic evolution and bounded causal changes. Shared geometry/source transforms serve effect-specific local/path sampling. Applicable fields and authored substantial terrain affect sensing and in-world Comms paths. Static celestial bodies remain static; ordinary capital-ship controls remain planar. |
| S054/S055/S096 and later systems | Helm can show uncertain possible intersections from observed ordnance, preserving human motion authority. Keep Tactical's primary target and independent Science designation. New weapon assignment/doctrine, fire-control illumination, shield probing and guidance consumers land with increment 15. Probe launch/recovery/mobile survey lifecycles remain increment 19; 14 supports compatible observations from actual existing ship/probe sources without inventing a deployment subsystem. Expanded Comms/traffic/decryption and electronic operations remain 16–17. |

S027's low-emission intent marks relevant human controls but does not inhibit
them or require overrides. This increment supplies actual emission observations
and the equipment contract; broader emission/ECM controls follow their owning
capabilities. Cloaking, a universal scanner, freeform physics, manual decoding
puzzles, an obligatory mining loop and new relay/craft lifecycles are not implied.
Coolant recycling remains provisional. Numeric ranges, cadence, confidence
thresholds, queue bounds, forecast horizons and sensing curves are authored tuning
chosen and tested during implementation, not imported competitor constants.

## Current owners and structural migration

| Current source | Implementation responsibility |
|---|---|
| Gameplay `science/scan.rs`; Simulation `science/server.rs` | Reuse supported derivation, fidelity bands, publication gates and real machinery costs. `ShipScanRecord` currently holds one last reading/refusal, not a complete observation history or two scanners. Migrate that consumer before adding new sensing behaviours. |
| Model `observation.rs`; Console publications | Extend existing provenance, logical sample/receipt time and uncertainty. Preserve frozen per-System publications and typed Console knowledge bindings. Do not add a second truth store behind unchanged radar payloads. |
| Existing dossier evidence and delayed Sensors reports | Retain canonical records, original times and sample-and-hold behaviour. Build history/correlation over real recorded evidence; adapters must not maintain competing copies of mutable findings. |
| Simulation `ship/sensors.rs`, Helm/Tactical/Navigation publishers | Preserve Science Target, advisory designation and Combat Lock authority while replacing live detail shortcuts with supported evidence. Human and AI projections must share the same permitted contact/findings. |
| Gameplay mount/tracking, local energy, physical conditions and fitting | Separate fitted broad/focused/passive Systems with actual Console, placement, costs and ordinary AI. Share existing supported geometry; do not introduce another steering model or sensor capacity meter. |
| Gameplay `regions/shape.rs`, `regions/effects.rs`; Simulation Regions | Extend the current containment/effect owner. Existing binary effects become the step-profile case; local exposure, path attenuation and terrain obstruction remain distinct queries. |
| World, Content Preparation and Workshop | Author supported subjects/properties, source bindings, fields, signal materials and compatible scanner equipment. World owns admitted parameter changes and scenario consequences; Workshop uses its existing accepted-source/history/Preview/Test workflow. |

First migrate current infrastructure/physical-condition scan output into typed
findings with equivalent observable behaviour and its normal scenario completion
event. Preserve `publish=false`/withheld defaults. Then introduce independent
automatic broad sensing and timed focused enquiries. Existing `ScanTarget` may
translate to a focused request during this migration, with a named removal point
before the increment closes; it must not retain a second instant reveal path.
Scenario scripts still receive actual adequate-observation completion, never a
button press or unsuccessful attempt. Before changing a scenario's timing/design,
use its design digest and supported writeback, retaining required journeys.

## Findings, tracks and permitted knowledge

Each actual observation has stable origin/sample identity, source instrument and
observer, subject/contact association, finding type, logical sample time,
received time where applicable, and supported uncertainty/provenance. Confidence
belongs to the finding: known identity can coexist with uncertain position or
condition. Keep a small readable quality vocabulary and geometric uncertainty;
do not require operators to manage probabilities or refresh every record.

Physical producers may read actual world state to derive permitted measurements.
Controllers, prediction, correlation, GUI and forecasts consume only those
measurements and authorised records. A source UUID used internally for routing
does not prove contact identity or associate ambiguous reports. Track association
and alternative hypotheses must be justified by delivered evidence, not a hidden
ground-truth key. Unidentified contacts need an observer-local presentation/routing
identity that does not disclose a vessel's true name, faction, model or internals.

Maintain one authoritative lifecycle for each recorded finding. Extend/migrate
the existing scan/evidence owners and add real history/track consumers, rather
than copy current truth into another universal dossier. Console read views remain
explicit; two co-hosted tabs do not pool findings. Shared history belongs to the
receiving observer and retains permitted-source boundaries. A display, reference
attachment or assessment does not authorise a target selection or actuator.

Lost contacts retain their last actual observation and identity evidence where
appropriate. Predict only from observed motion and elapsed logical time, broaden
position uncertainty, mark predicted/stale tracks and stop presenting a live
precise return. Reacquisition requires supported association. Unseen manoeuvres,
actual target velocity and hidden route orders cannot update the prediction.
Age changing location/condition findings without erasing every historical fact.
Bound retained records/hypotheses with deterministic retention/cleanup rules;
cleanup cannot silently convert a stale prediction into confirmed destruction.

Measurements must not always be truth-centred, and repeated presentation/copying
must not reroll error or increase confidence. Any sampling randomness uses existing
seeded streams with stable observer/System/subject order and saved continuation.
Time, record identity and acquisition are simulation facts; locale, wall clock,
render cadence and which peer owns LocalShip must not alter them.

## Operating the scanners

Broad scanning automatically services supported returns within its actual coverage.
Adding harmless contacts alone does not divide an update budget. Range, geometry,
equipment, power, condition, concealment and interference still constrain results.
Routine unobfuscated assessment is automatic; it is not four manually approved
progress bars and does not expose protected internals by waiting indefinitely.

Focused sensing uses its own reserve, work/cooling and one active subject/aim.
Its ordinary AI executes a bounded prioritised queue. Explicit interruption keeps
earned findings; unfinished work has an authored retention/decay rule and cannot
complete while unfunded or out of valid geometry. Repeated selection does not
reroll favourable errors or create extra simultaneous work. Multiple separately
fitted scanners can operate concurrently; player rating never changes physics.
Start with general condition/structural enquiries already useful to repair,
rescue and recovery, then supported compartment membership/angle observations.
Do not expose the full actual interior or private damage dose/source as a shortcut.

Intrusive aims are optional focused-equipment operations, not a mandatory new
System or universal scan stage. Subject/property observation policy must explicitly
permit the supported measurement; old withheld data stays withheld unless authored
otherwise. Model actual exposure and observable reactions using normal machinery,
events and World rules. Neither acquisition nor reference matching reveals intent
or guarantees that cargo/personnel/security claims can be verified. New weapons
weakness/probing effects remain increment 15; encryption/ECM retains later owners.

Passive receivers have independent fitted state, resources and ordinary AI.
Actual active scans and supported major machinery operation produce readable
emission causes. A weak reception may provide only bearing/signature; suitable
movement, active confirmation or delivered observations can improve localisation.
No contact requires a compulsory triangulation procedure. Integrated estimators
handle arithmetic and present only a few consequential alternatives. Generic
Viewscreen wireframes and confidence areas must not disclose true unknown geometry.

All changed System AIs use the existing closed state-tree engine, admitted channels
and private continuation; retire their superseded controller alongside the owner.
Human controls remain ordinary System-addressed intents, with shared status,
refusal, delegation and stale-host protection. Low/mid complexity exposes useful
priorities and delegates execution; detailed controls do not grant better facts.

## Sharing, reference work and causal forecasts

An authorised operator selects the supported radar feed and faction audience;
omission grants no sharing. Existing in-world communication eligibility/link
rules determine delivery, independent of reliable technical transport. Receiving
a feed confers information only. Revocation stops future delivery; already received
findings remain honestly historical under supported retention. Resumed delivery
makes relevant retained findings available through normal history; no new catch-up
System or manual resend ritual. Old findings cannot overwrite newer samples.

Fuse only compatible delivered observations with useful geometry and age. Preserve
origin identity across forwarding and deduplicate copies; a report circulating
through three allies is one sample. Position refinement does not automatically
confirm identity or condition. Actually fitted probe/ship sources use the same
observation contract and real links; new launch, mobile survey and recovery
workflows are the later deployable consumer, not prerequisites for this foundation.

Automatically attach relevant known references, preserving attribution and age;
unusual records require deliberate finding/flagging by their permitted operator.
Show contradictions instead of selecting hidden truth. Retrieval/correlation has
no new resource pool or analysis timer. Ordinary AI uses relevant text only after
deterministic reading behaviour, using authored semantic work/cadence rather than
device speed or translated display length. Observation acquisition and existing
decryption/communication constraints retain their own costs.

Environmental forecasts use observed trends or known published models, and label
their assumptions, horizon and uncertainty. The physical field sampler may
evaluate analytic parameters at future ticks for simulation/authoring, but crew
forecasts cannot read undiscovered field parameters, pending scripts or future
actor decisions. A changed cause can invalidate a forecast without rewriting
the recorded prediction into a correct one after the fact.

## Fields and signal paths

Extend Regions with supported falloff, intensity envelopes and bounded motion or
growth. Sample authored parameters at the logical tick; causal actions replace
parameters through the existing admitted World/effect boundary. Suitable equipment
sources derive transforms/strength from their actual mount, enabled state, paid
work and condition. Not every emission spawns a Region or shares one scalar model.
Keep separate effect processors for damage, motion, sensing and applicable Comms.

Add a deterministic effect-specific segment query. A cloud between two unexposed
endpoints can attenuate a supported signal; endpoint-only membership is insufficient.
Specify overlap, endpoint/path combination, grazing boundaries and finite sampling
costs in the processor's implementation contract. Avoid counting the same path
exposure twice. Radiation may remain local; a receiver-side jammer is not therefore
an opaque wall. Migrate binary region effects as explicit equivalent step profiles
before intentional field tuning, retaining current signs/units and damage routes.

Substantial planets, moons and suitable large asteroids have explicit signal
properties and authored physical bounds. Use these bounds rather than rendered
mesh detail for signal-specific attenuation/occlusion; ordinary ships/debris do
not automatically become opaque terrain. Actual supported terrain clearing changes
that geometry's obstruction, without turning every rock into an objective.
History persists behind cover, but never refreshes through it as live truth.

Use the same physical query rules for human and AI equipment. Apply them to actual
in-world signal classes, including a current Comms delivery consumer; never to
network sockets, player connection, internal command admission or a universal
message-capacity budget. Expanded channel/relay/order lifecycles remain their
later increments. Celestial bodies do not orbit. Fields may have bounded authored
motion, and moving source equipment retains its actual transform. Capital ships
remain planar with the existing limited avoidance height; no orbit/reference-frame
or planetary-navigation redesign is introduced here.

## Ordered implementation and verification

1. Migrate current scan output/provenance and relevant radar details through real
   permitted findings, preserving current shipped readings and completion events.
   Establish history/track continuation and explicit display states.
2. Fit independent Cruiser broad/focused equipment and their ordinary AIs,
   resources/conditions/controls; automatic broad assessment and one focused
   enquiry demonstrate concurrent combat and rescue/infrastructure work.
3. Add independent passive reception, integrated uncertain motion/localisation,
   readable Viewscreen ambiguity, reference correlation and supported forecast
   enquiries. Initial emission/aim catalogues stay small but have real consumers.
4. Add deliberate faction radar feeds/history and compatible multi-source fusion,
   using actual ships and existing remote sources without new deployment machinery.
5. Upgrade Regions and actual sensor/Comms path consumers, then authored substantial
   terrain occlusion. Verify equivalent old profiles before intentional changes;
   exercise one causal field source and one useful concealment/cost trade-off.

Every step includes Workshop supported composition, closed AI editing, reference
validation, exact-source materialisation/history, Preview and disposable Test.
Migrate hulls, includes, generated Systems, fixtures and affected scenario overrides
together; other shipped equipment remains supported without mandatory retrofit
buttons. Add full candidate preflight for instrument/record/source/track identities,
queue state, private AI, selected feeds/audiences, field parameters and received
history before any restore writes. Capture/digest authoritative state immediately;
derived geometry indexes/publication caches remain derived. Advance affected
compatibility definitions explicitly, retaining existing networking/delivery classes.

Targeted positive and refusal cases must prove:

- Broad updates continue while focused work is active or unfunded; ordinary contact
  count alone does not dilute updates. One enquiry per fitted scanner, real costs,
  condition limits and ordinary delegation; co-hosting/stale hosts grant no access.
- Same human/AI contact evidence, explicit unknowns, confidence by finding, no live
  velocity/condition/hidden-ID leak after loss, no repeated-click reroll and no true
  hull geometry in ambiguous wireframes. Helm predictions use observed ordnance.
- Real knowledge-limited sharing: absent/revoked/wrong-faction feeds refuse new
  delivery, history survives link loss, catch-up preserves time, duplicate routes
  do not inflate confidence, poor geometry cannot produce exact localisation.
- Useful actual condition and environmental enquiries, withheld/intrusive refusals,
  ordinary versus unusual record attachment, deterministic AI reading and forecasts
  that cannot see undiscovered parameters or hidden future events.
- Outside-to-outside field attenuation, local exposure, overlap and tangent cases;
  terrain cover/clearing, static bodies and source motion. Current Comms is affected
  in fiction while technical player/peer traffic stays connected.
- JSON save/resume and different-LocalShip multiplayer agree across lost tracks,
  enquiries, passive findings, sharing gaps and field changes. Invalid complete
  candidate state refuses without live entity/layer/global writes.
- All shipped hulls/scenarios and 2/4/6/8-seat journeys load. Compare Cruiser combat
  and non-combat workload, gameplay activity and bounded performance against the
  baseline without weakening balance thresholds. Scenario design edits follow the
  digest/writeback workflow; browser/phone/native acceptance is separate evidence.

Use targeted owner tests and independent review during implementation. Run required
integrated gates once before a push. Record exact source/configuration, nonzero
counts and remaining evidence; initial source integration is not this playable exit.
