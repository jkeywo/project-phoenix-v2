# Coherent gameplay architecture — synthesis brief

Status: incremental implementation approved; baseline increment complete, 2026-10-13. This extends the [gameplay proposal](../README.md); it is not an adopted PASM design or an implementation instruction. The ordered review of 114 game systems and 102 ship systems is the input, rather than a list of 216 separate modules to build.

Start with the [feature reconciliation report](./feature-reconciliation.html): twelve proposed mechanism families, ten operable ship-System families, world/supporting responsibilities, local/remote data flow and a searchable mapping of all 216 catalogue entries. The [supporting architecture report](./index.html) retains the decision diagrams, and the focused [AI state-tree report](./ai-state-trees.html) develops the selected AI authoring direction. The groupings are proposed for review; detailed System contracts and migration planning are deferred.

## Current implementation scope

The user approved the [incremental implementation roadmap](../implementation/index.html) on 13 October 2026. Implementation follows dependency-ordered increments with the later approved isolated validation lanes; the requested increments 8-12 verification is collected with balance failures and work is now paused. The [canonical ledger](../implementation/migration-ledger.json) accounts for every catalogue entry and records completion evidence.

Preserve the selected mechanisms and ownership boundaries. Detailed System rules, schemas and tuning are specified when their increment is prepared; do not restart the earlier broad interview. Adopt the relevant design into PASM before runtime work, and keep shipped content functional. Increment 0 changes planning/design records and captures evidence without changing gameplay.

## Current verification checkpoint — 16 October 2026

Current integrated compatibility is protocol 45, snapshot 88 and simulation
rules 0.75 (`e027e568`). The selected four-owner union at `8ac62cce` / rules 0.74
passes 1618/1618; the eight current whole-host targets at `e027e568` pass 19/19,
with the selected energy resume checks passing 2/2. Earlier failed results stay
recorded. The empty-plan restore fix preserves authoritative absence and exact
UUID rebinding rather than retaining bootstrap avoidance motion.

The matching development host/client/preview bundle at `ded6653a` includes the
WASM-only import correction `a794ac49`. Its complete 15-case Chromium selection
passes at test source `dbfd0027` in 3.3 minutes, including browser reconnect and
save portability. Workshop's missing existing startup modules are packaged at
`2ea96d97`. After valid fixture corrections `956a385c` / `9bd7bab6`, the complete
real resource/factions/disposable-Test render selection passes at `9bd7bab6`:
3/3 in 6.1 minutes, including accepted source edits, history, reload/export and
mesh Preview/Test. This is the actual Workshop browser journey, not all authored
or Human workflows.

The current same-machine baseline at `9bd7bab6` passes six determinism checks,
but unchanged Cruiser balance fails 11/16 checks (zero wins, seven draws and
thirteen timeouts). Measured headless median/p95 tick costs rise 3.5–4.8 times,
with no observed 16.67 ms overruns. These results block automated exits 8–12;
no thresholds changed. Work is now paused and increments 13–24 remain parked.
Human/device, station-workload and pre-push integration acceptance stay separate.
Follow the [implementation summary](../implementation/README.md),
[verification record](../../acceptance/1562-integrated-8-12.md),
[comparison receipt](../implementation/verification-8-12.json) and
[source-pinned checkpoint](../implementation/parallel-checkpoint.json).
Historical diagrams and the baseline orientation table below remain design inputs.

## Requested outcome

Develop a coherent design from the selected gameplay rules using a limited library of reusable mechanisms. Cover world activity as well as ship operation, authored configuration, runtime state, local data-oriented processing, remote communication, consoles and Workshop. Present the resulting proposal as linked HTML reports with flow charts for review and approval. Identify current implementation, selected gameplay requirements, proposed architecture and unresolved choices distinctly.

Retain the existing networking architecture and individual AI per System with communication and limited knowledge. GUI and AI commands must continue to meet the same authority boundary. Mechanism reuse must not erase distinct operating decisions or give AI privileged information.

## User's architectural direction

- Reuse arcs, mounts, traversal and tracking for weapons, sensors and other aimed equipment where applicable.
- Represent deployables as ordinary configured ship entities: for example, a turret has a weapon, battery and simple AI; a decoy has spoofing, a battery and a finite lifetime.
- Use one configurable weapon System with composable behaviours and authored parameters for weapon families, projectiles and firing patterns.
- Compose Systems from reusable state/mechanisms such as capacity and incoming energy flow.
- A System owns generic command channels such as on/off, target, attack and jam. All its components can read those channels and respond to the ones they use; composition does not require a bespoke command pattern for every equipment combination.
- Components can also connect to external resources or permitted information: a coolant conduit draws from a central pump, while a targeting-data component obtains shared information from another Console.
- Define Stations on a ship, then assign Consoles to them, with both fixed and human-seeking placement.
- Give every System a physical compartment as well as its controlling Console. Physical containment and control ownership are separate relationships; Systems in one compartment need not share a Console, and a Console can control Systems in different compartments.
- Components belong generically to that compartment, without precise internal coordinates. Damage selects affected components from the compartment's eligible pool; scans identify equipment's compartment and targeting aims toward that region.
- Support multiple decks, each with a 2D physical layout for damage and repair. Incoming damage selects a weighted random deck, independently of shot Z height, then penetrates along its incoming planar direction through that deck's compartments.
- Configure weapon penetration separately from delivered damage/effects. Set material properties at compartment level only, with resistance charged per unit of 2D path length and progressively reduced as compartment HP falls. Compartments own HP; damage to a compartment applies status effects to its components. Components do not have separate structural HP.
- At zero compartment HP, a condition disables its contained components. AD50 clears it at a modest authored recovery threshold above zero and below full HP, leaving each component's other conditions intact. Once cleared, it reapplies only if structural HP reaches zero again.
- Compartment HP depletion does not itself physically destroy a ship. AD52 selects catastrophic destruction, with separate recoverable disablement and early crew abandonment. Losing engines and power can establish disablement; individual AI needs a clear evidence-based assessment that an opponent is out of action without waiting for an explosion.
- AD53 makes non-hostile standby the default abandonment procedure: issue ordinary cease-fire/standby orders, retaining supported evacuation, life support and distress functions where possible. Individual System AI executes authorised operations within normal physical constraints.
- AD54 requires a human to initiate abandonment when human players are aboard. AI can recommend it and execute authorised delegated steps; fully AI-crewed ships decide through ordinary per-System policies and permitted knowledge.
- AD55 permits authorised crews to return an abandoned ship to service during the same scenario through ordinary personnel transfer and restoration. Preserve the existing ship, Systems, Consoles and persistent state; abandonment alone imposes no port-recovery requirement.
- AD56 selects whole-ship operational takeover after supported capture requirements are met. Located teams can still sabotage equipment and secure areas, but do not acquire individual-System command authority before that handover. Each System retains its ordinary controller structure.
- AD57 makes retained, already-readable Console records available to newly authorised operators after capture, preserving source, age, uncertainty and Console scope. Ongoing feeds still require ordinary access; unread encrypted material and private AI memory do not become readable Console facts through takeover.
- AD58 uses authored compartment connections, including links between decks, for automatic team and delivery routing. Shared route queries retain each activity's access, safety and travel rules; damage penetration and resource conduits keep their own relationships.
- AD59 represents internal deliveries as aggregate batches in transit. Dispatch moves quantity out of the source stock; arrival deposits it at the destination. Route progress and actual location/state survive interruption, using shared resource and job mechanisms without per-item or individual-porter simulation.
- AD60 reserves compatible receiving capacity when a delivery is dispatched. Other ordinary loading respects the claim; undispatched queue entries reserve no destination space. Reservations neither create capacity nor guarantee access after damage or isolation.
- AD61 uses authored logistics throughput and concurrent-work limits for routine internal deliveries. Shared capacity and job mechanisms account for the work, subject to fitted resources, conditions and routes; no separate personnel team is assigned to each batch.
- Allow Workshop templates to be created from a component, a System or a compartment, including a compartment's fitted Systems and internal connections. Repeated placements retain separate identities and runtime state.
- Each Console supplies one GUI screen; a Station can present several Console screens as tabs.
- Explore a blackboard per Console. Systems publish filtered information into it and Systems/GUI consume it. Radar presentation should combine relevant permitted observations from multiple producers.
- Provide a reusable System status indicator: green when fine, amber with a condition icon when partially available, red with a condition icon when completely unavailable. Use a special icon for multiple statuses. Conditions include EMP, damage, destruction, overheating and no power. Clicking opens details. A System with no controls still appears with its status; AI-controlled Systems are hidden from the ordinary System list as requested. AD20 retains a compact Console status summary and System AI repair reports, including reports from entirely AI-operated Consoles.
- Make composition data-driven and integrated into Workshop through visual entity authoring: Stations, Consoles, Systems, reusable components and physical ship layout.

The attached sketch is a candidate model. The accompanying prose proposes Console blackboards whereas the image labels Station blackboards; ownership and data bindings require a deliberate resolution rather than treating the drawing as final authority.

## Baseline implementation evidence — retained design context

| Concern | Current seam | Consequence for the proposal |
|---|---|---|
| Seat and hosted surface | `StationConfig` in [ship/config.rs](../../../crates/phoenix-sim-gameplay/src/ship/config.rs); [Station guide](../../../wiki/entities/station.md) | Current Stations include fixed seats and auxiliary human-seeking tabs. A separate Console object changes authored vocabulary and bindings. |
| Fine capability | `SystemInstanceConfig` in [ship/config.rs](../../../crates/phoenix-sim-gameplay/src/ship/config.rs) | Systems currently name an owning Station, and ratings determine control depth/AI delegation. |
| Authority | [command_admission/policy.rs](../../../crates/phoenix-simulation/src/command_admission/policy.rs) | Admission resolves session tenure, System ownership and control policy. Mounting a GUI cannot grant authority. |
| Observations and console payloads | Typed blackboards in [messages.rs](../../../crates/phoenix-model/src/messages.rs), [console-state.js](../../../gui/console-state.js) | Existing typed projections are a starting point; no untyped global mutable blackboard is assumed. |
| Knowledge and AI | [Information-parity guide](../../../wiki/concepts/information-parity-audit.md) | AI reads permitted observations and delivered Coordination, retaining private policy memory; the proposed aggregation must preserve those limits. |
| Local processing and package boundaries | [Reusable layers](../../../wiki/concepts/reusable-layers.md) | Gameplay, World and Session are sibling domains composed by Simulation; shared mechanisms must respect those dependency boundaries. |
| Authoring | [Workshop guide](../../../wiki/entities/editor.md) | Extend the existing document/history, candidate validation, preview and disposable Test workflow. A visual editor must not create an independent content/runtime interpretation. |
| Console widgets and geometry | [Console UI guide](../../../wiki/concepts/console-ui-library.md), [console.css](../../../gui/console.css), [ph-scope-chrome.js](../../../gui/components/ph-scope-chrome.js) | Current HTML layouts compose reusable controls. Shared scope sizing and corner readouts already use the space outside a circular radar. Workshop-authored layout data and general shape-aware control groups are proposed extensions. |
| Remote flow | [Message flow](../../../wiki/concepts/message-flow.md) | Map observations, commands, reliable results and replicated simulation state onto existing lanes rather than replacing transport. |

These are orientation findings, not a completed audit of all consumers or network paths. Follow code before making claims about exact current behaviour.

## Report structure

1. **Composition and ownership:** Entity, deck, compartment, Station, Console, System, AI controller, blackboard and physical part identities; containment versus reference versus control authority.
2. **Reusable mechanism catalogue:** small set of code mechanisms, authored parameters, mutable state, inputs/outputs and participating gameplay systems; coverage matrix back to both catalogues.
3. **Data and execution:** authoritative state, observation production/filtering/merging, knowledge scope, decision inputs, admitted commands, fixed-tick processing and presentation. Show concrete end-to-end traces.
4. **World systems:** actor objectives, teams/jobs, inventories/transfers, contacts, Comms, hazards, damage and scenario consequences using shared mechanisms where their semantics match.
5. **Consoles and status:** screen/tab composition, complexity/delegation, reusable status/details, permitted radar aggregation and Viewscreen publication.
6. **Workshop:** reusable templates and instances, visual composition/layout, typed bindings, validation, preview/Test, and data compatibility/migration.
7. **Module and delivery map:** existing owners, required changes, reusable implementation modules, local versus remote contracts, migration and unresolved approval choices.

Use several diagrams rather than one graph that conflates object containment, physical simulation, knowledge flow, commands and network transport. Diagrams must label these relationships explicitly.

## Terms to resolve

AD03 distinguishes reusable ECS/data components from physical condition/repair targets. AD44 assigns HP to compartments and status effects to components; AD48 gives components compartment membership without precise internal coordinates. A physical part can affect several capability records without its own structural HP or local geometry. A capacity record is not automatically a separate repair target. Exact bindings and storage remain under review.

Also distinguish a gameplay **System** from a Bevy processing function. A proposed single weapon capability need not be one enormous update function, nor does every catalogue entry require its own System kind or crate.

## AD01 — Console control ownership: selected A

Should every operable System have one controlling Console, even when other Consoles display its information?

- **A — Console owns control bindings.** A System has one controlling Console; that Console's fixed/human-seeking Station assignment resolves human authority. Other permitted Consoles can consume published observations/status without acquiring control.
- **B — Station owns Systems independently.** Systems remain assigned directly to Stations. Consoles are screen/data compositions; their command access must be bound and checked separately, including when a Console seeks another human host.

**Selected: A.** Each operable System has one controlling Console. That Console's fixed/human-seeking Station assignment resolves the human operator through the authority rules; presenting another System's information does not grant control. Keep one effective Control Source per System and independent per-System AI. Read/publish bindings may be many-to-many without making control ownership many-to-many. This is a selected proposal direction, not a change to current code or PASM. AD02 resolves Console knowledge and AI access; AD12 resolves working-view continuity when the operator changes.

## AD02 — Console blackboard knowledge scope: selected A

Should all individual System AIs belonging to a Console be allowed to read its complete permitted blackboard, as that Console's human operator can?

- **A — Shared Console knowledge.** Each AI may read the same Console facts; policies use the relevant subset. Systems still act independently. Cross-Console information requires an explicit permitted publication or ordinary Coordination rather than automatic pooling because two tabs share a Station.
- **B — Per-System read bindings.** Each AI can read only its separately authored subset of the Console blackboard. The GUI may display the broader combined picture, so the human operator can know more than any one AI.

**Selected: A.** Every individual System AI on a Console may read its complete permitted blackboard, just as the human operating that Console receives those facts. Policies use the information relevant to their decisions; there is no additional per-System knowledge restriction within that Console. Producers still filter world observations, cross-Console sharing remains explicit, and co-hosting tabs does not automatically merge their blackboards. The shared facts do not grant another Console's control authority or give Captain a map. Each System retains its own AI and policy memory. Hiding an AI-operated System's GUI entry must not stop its simulation or required data publication. AD12 carries knowledge and view continuity through operator handoff. Exact aggregation, publication contracts, storage and retention still need specification.

## AD03 — Reusable mechanism components and physical parts: selected A

The gameplay proposal selects physical condition/repair targets. AD48 refines their location to compartment membership without internal coordinates. The architecture also needs reusable data components such as capacity, energy intake, heat and mount traversal. Decide how those concepts relate before defining the Workshop composition schema.

- **A — Distinct concepts, explicit bindings.** Physical parts are the damage/repair/layout units. A part can supply or affect several reusable mechanism components; for example, a capacitor assembly can affect storage capacity, intake and thermal behaviour. Mechanism components need not each become a separately damageable object.
- **B — One component per mechanism and damage unit.** A capacity component, flow component or aiming component can individually carry its location, conditions and repair definition. Physical assemblies group those components, while component-level damage remains the default granularity.

**Selected: A, refined by AD44 and AD48.** Physical parts are condition/repair targets with compartment membership, not precise internal positions; each can provide or affect several reusable mechanism components. Compartments own structural HP, and compartment damage applies component status effects. A capacitor assembly can affect capacity, intake and thermal behaviour while remaining one repair target, without a separate HP pool or geometric footprint. Workshop should show a part and its capabilities together, with explicit bindings from conditions to effective capability. Exact condition effects and ECS storage layout remain open; this does not prescribe a runtime object tree.

## AD04 — Shared services and integrated mounts: selected with clarification

Code reuse does not require hardware sharing. Decide whether the composition model should also allow one physical part to support several distinct Systems, such as a shared coolant pump or a physical mount carrying both a weapon and a sensor. The condition state and repair job would belong to the one part, and all dependent capabilities would respond to its condition.

- **A — Shared physical dependencies allowed.** Parts are identified on the ship layout and Systems reference the capabilities they depend on. Several Systems can depend on one part, with readable common failure consequences.
- **B — One System per part.** Each physical part belongs to one System. Cross-System effects travel through explicit System-to-System resource/service connections; shared machinery is represented by its own supplying System.

**Selected: A for shared services, with an explicit distinction for mounts.** A serviced System has a component representing its link to the shared service. That link can be damaged where applicable, such as a coolant conduit. A central pump with independent controls is its own System. Its physical source parts and each receiving link have distinct condition/state; a source failure can affect several consumers while a link failure affects delivery through that link. This does not duplicate source capacity. A pump mechanism can also be placed inside a turret System and share its on/off channel (AD05); independent System ownership is a control choice, not a restriction on component combinations. Exact flow and repair contracts remain open.

**A turret is a component within one System.** All capabilities sharing that turret must be mounted in that same System: for example, two different weapons, or a weapon and targeting array. Do not represent them as separate Systems independently sharing the turret. This supersedes the cross-System shared-mount example in the original question. AD01 still gives the composite System one controlling Console, and the retained architecture gives it one individual System AI. Reusable weapon, sensing and aiming mechanisms can compose inside that boundary; code reuse does not require a separate controller for every capability. Exact targeting/concurrency rules remain open.

Workshop needs two distinguishable relationships: a service connection to a receiving link component, and an internal capability binding to a mount component. Validation should reject cross-System turret references and conflicting ownership. The internal composition graph does not mandate a particular ECS entity tree.

**User's defining rule, refined by AD05: a System has shared command channels and one controller.** A turret receives a target and whether to attack, jam, or both against that target. Its components consume the relevant channels; they need not all perform the same kind of work. The user's pump-on-a-turret example is valid when the pump shares the System's on/off toggle. Give a service its own System when independent control is wanted. A composite System retains one owning Console and one individual System AI. This describes shared command ownership, not exactly one wire-message variant, one monolithic update function or a fixed family of compatible equipment.

Attack and jam are independently selectable parts of the shared intent, with simultaneous operation supported where equipment, geometry and resources permit. They share the selected target; do not invent a separate jammer target within this example System. One intent still produces several internal state updates/effects. An illustrative command shape is `target + attack enabled + jam enabled`, not a final field schema. Mechanism reuse and data composition must respect this command boundary.

## AD05 — Generic System command channels: selected user-supplied direction

The earlier question offered reusable whole-intent patterns versus fully authored schemas. The user instead specifies generic command channels on each System: for example, on/off, target, attack and jam. Every component can read those channels and responds to the ones it uses. The composition is not constrained to a bespoke target/actions, output/allocation or job/objective command type.

**Selected:** one System command-channel set, one effective controller, many component readers. A turret, weapon and jammer share the target channel; attack and jam independently enable their respective behaviours. A pump added to that System can use the same on/off channel. Do not manufacture a new command type or an independent component controller for that combination. Resource, geometry and condition constraints still determine what actually happens.

Implementation proposal: define supported channel meanings/types and component inputs in the shared mechanism contracts; data composes and binds them. Human and AI requests go through admission before changing authoritative command state. Command intent remains distinct from actual operation and from facts on a Console blackboard. Local processors read shared data rather than requiring a separately serialised command per component. AD14 selects configurable AI state trees within the supported input/output vocabulary. Exact channel schema, update order and tree execution contracts remain open.

## AD06 — Resource and information link components: selected direction

The user adds two examples: a coolant conduit component drawing coolant from a central pump, and a component obtaining targeting data from another Console, such as Science's current scan target being available at Tactical. Components therefore cover external connections as well as local storage, heat, aiming and effect production.

| Connection | Meaning | Boundary |
|---|---|---|
| Coolant conduit | A receiving component draws from a shared central supply, subject to actual stock, allocation, delivery and physical conditions. | Source capacity is counted once. A conduit can have its own damageable physical part; damage changes delivery, not the operator's requested setting. |
| Targeting-data link | A component consumes an explicitly bound, permitted information feed from another Console. | It conveys known information rather than physical truth or command authority. Science's scan target does not automatically become Tactical's weapon target. |
| Internal command-channel read | A component consumes its own System's shared on/off, target or action intent. | It does not create a second controller or bypass admission. |

The independent Tactical target decision is already selected in the gameplay review's illumination clarification. Imported information must remain available to the relevant human operator as well as AI; AD02's Console knowledge parity still applies. An information link does not grant access to the source Console's entire blackboard, or make two Consoles' private knowledge identical.

Implementation proposal: Workshop shows typed resource links, information bindings and internal command-channel reads separately. Resource processors conserve stock/capacity; information readers preserve relevant source, age and uncertainty. Neither a logical data binding nor a mechanism record automatically becomes a new physical repair target (AD03). Whether an information binding is implemented as a permitted view or a local projection is a storage/scheduling choice still to resolve. No change to the existing network transport is implied.

## AD07 — Publishing across an authored information link: selected A

When Tactical has a targeting-data link to Science, what makes Science's current scan target available there?

- **A — Automatic, scoped feed.** The ship definition connects a specific published feed. Tactical sees its updates automatically while the link is available; unrelated Science knowledge stays private. Tactical still chooses its own weapon target.
- **B — Explicit publication.** Science chooses when to share a target/update across the connection; changing scan target does not itself publish another update.

**Selected: A.** An authored information connection automatically supplies updates to its specific permitted feed while available. A Tactical targeting-data link can therefore receive Science's changing scan target without Science repeatedly pressing Share. Other Science information stays private, and Tactical independently chooses its weapon target. This is scoped publication, not general pooling of Console knowledge. Players still communicate interpretation, requests and coordination. Exact payload contracts, availability, observation age, projection storage and scheduling remain to be specified; automatic publication does not imply zero latency or a new transport.

## AD08 — Status of a partially functioning System: selected with visual specification

A System can contain several capabilities and supporting components. Its reusable status indicator must distinguish the condition of a component from the availability of the whole assembly. For example, a turret's jammer could be EMP-disabled while its gun still works; loss of their common mount could instead affect both.

- **A — Availability summary plus condition markers.** Show operational, degraded or unavailable based on affected capabilities, with applicable condition markers such as EMP, damage, overheating or no power. The popup explains which capabilities are affected and why. A working gun with an EMP-disabled jammer is degraded with an EMP marker, rather than implying that the entire System is EMP-disabled.
- **B — Most severe component condition.** The main System indicator takes the highest-priority component condition, such as EMP or destroyed. The popup distinguishes the affected components and any surviving capabilities.

**Selected: availability and condition shown separately, with the user's visual specification.** Use a green light when the System is fine; an amber light with a condition icon when partly available; a red light with the status when completely unavailable. If multiple statuses apply, use a special multiple-status icon rather than selecting a single cause. The light still represents overall availability: multiple conditions do not automatically mean red. Clicking the indicator opens the existing detailed popup, listing the conditions, affected components and resulting capability. A turret with a working gun and EMP-disabled jammer therefore has an amber light with the EMP icon.

Status remains evidence-based under the existing diagnosis rules: a popup must not reveal an unconfirmed internal cause. Implementation proposal: expose a common observed-status contract with overall availability, known conditions and affected capabilities, consumed by the reusable GUI widget. Derive availability from the authored dependencies; keep actual System intent distinct from this status. AD20 selects aggregate visibility and repair reports for hidden AI-operated Systems. Specific icon artwork and treatment of deliberate shutdown/ordinary readiness remain to be resolved. Text/accessibility equivalents accompany the visual indicator.

## AD09 — Authoring Console screens: selected A for a first prototype

The design already gives each Console one GUI screen, with Consoles appearing as tabs when hosted together. Shared command channels, permitted blackboard feeds and the reusable status indicator provide standard binding targets. Decide how those become a complete Console screen in the visual Workshop.

- **A — Authored layouts using reusable widgets.** Authors arrange radar, System controls, status indicators, lists and other supported widgets, binding them to command channels or permitted blackboard data. Reusable Console templates provide defaults; adding a component need not add a separate panel. Workshop edits the composition as data.
- **B — Automatically generated screens.** Fitted Systems and components primarily determine the controls and layout. Authors provide grouping, ordering and presentation hints rather than directly composing the Console screen.

**Selected: try A first.** Authors arrange reusable widgets and bind them to Systems and permitted blackboard data. This is a prototype direction to evaluate, not a final commitment that this authoring model will fit every Console. Reusable Console templates and widget code should keep composition data-driven without bespoke UI code for every layout. The current hand-authored HTML layout model remains implementation truth until this proposal is adopted and built.

**User refinement: account for non-rectangular space.** A circular radar leaves usable gaps around it. Explore UI components that can occupy a rectangle or fit into those gaps, allowing buttons and other suitable controls to use the available space. These are GUI/layout components, distinct from the simulation's mechanism components. A rectangular-grid-only editor would not satisfy this prototype requirement.

Implementation proposal: separate each widget's command/data bindings from its supported layout forms. A button group can use a rectangular region or a region bounded by a radar circle, while preserving the same actions. The layout needs both occupied geometry and usable content/hit areas; merely clipping a rectangular panel could leave hidden, overlapping or unreachable buttons. Workshop should preview shape fit, label space, touch targets and radar input clearance at different viewport sizes. AD10 selects the initial shape vocabulary and AD11 shared responsive fallback; exact packing and fit thresholds need prototyping. Reuse the existing scope sizing/corner conventions where useful; do not claim the current widgets already support arbitrary shape-aware packing.

## AD10 — Initial layout shape vocabulary: selected A for the prototype

For the first authored-layout prototype, how freely should authors define the regions that widgets fill?

- **A — Reusable shape presets.** Start with rectangles and parameterised regions around a circle, including its corner gaps. Authors choose orientation, dimensions, spacing and supported control arrangements. This keeps authoring bounded while testing the actual radar/button case.
- **B — Free-form regions.** Authors can draw arbitrary outlines or exclusions, and the layout system arranges supported controls within the remaining area. This offers more freedom but requires more general packing, hit-area and resize rules.

**Selected: A.** Start the prototype with reusable rectangles and parameterised circle-cutout regions, including the corner gaps around a radar. Authors choose orientation, dimensions, spacing and supported arrangements. Free-form outlines and general arbitrary-region packing are outside this first prototype. Evaluate this small shape library on phone and larger Console viewports before expanding it. AD11 selects shared responsive fallback. Exact parameter schema and fit rules remain to be specified; choosing presets does not authorise clipping controls or shrinking their hit targets indefinitely.

## AD11 — Responsive layout when shaped controls no longer fit: selected A

A corner group that fits beside a large radar may be too cramped on a phone. The prototype needs a predictable way to retain the same controls at usable sizes across supported viewports. This is presentation layout, not a change to System ownership, AI delegation, command bindings or Console count.

- **A — Shared responsive fallback rules.** Reusable layout templates include fallback arrangements: for example, a radar-corner button group becomes a rectangular row or panel below the radar when it no longer fits. Preserve the group's actions and authored order. Authors can tune the templates without separately designing every screen size.
- **B — Explicit layouts per viewport class.** Authors provide the small-screen and larger-screen arrangements for each Console, using the same widgets and bindings. Workshop previews and validates each variant. This offers more direct placement control but requires more authoring and maintenance.

**Selected: A.** Shared responsive templates adapt widget groups to the available space. A radar-corner group that cannot fit becomes a rectangular row or panel while preserving its controls, authored order and bindings. Authors tune reusable templates rather than having to design every Console separately for each screen size. Layout changes do not change System ownership, delegation or Console identity. Fallback follows available space, not gameplay events, and must preserve readable labels, accessible controls and clear radar input. Exact breakpoints, minimum dimensions and scrolling rules remain prototype questions rather than untested constants.

## AD12 — Working view when a Console changes operator: selected A

AD01/AD02 place control bindings and permitted knowledge on the Console. G114 already preserves ongoing System work and viable settings through ordinary handoff, subject to normal AI decisions. Carry that continuity into the Console model: changing its human host must not recreate its Systems, erase its permitted history or merge it with another hosted Console. This is an implication of existing ownership/continuity choices, not a new alternative to ordinary Backfill.

The remaining choice here concerns the working GUI view: radar zoom, presentation filters and which information record is being inspected. Actual weapon/scan targets and other operational selections remain System state in either option. Personal accessibility settings remain the receiving player's own.

- **A — Resume the Console's working view.** The incoming operator inherits its last view, including zoom, filters and inspected record. Adapt the layout to the new device using AD11. A human-seeking Console therefore arrives showing the work left by its previous operator.
- **B — Use the incoming player's view.** Preserve the same Console knowledge and System operation, but open with that player's saved presentation settings or defaults. The previous operator's browsing context is not transferred.

**Selected: A.** The Console's working view follows it to the incoming operator, including radar zoom, presentation filters and the information record being inspected. AD11 adapts the layout to the receiving device. Personal accessibility settings remain the receiving player's own. Operational targets, commands and ongoing work remain System state; permitted knowledge and history remain scoped to the Console. Handoff does not reset these, pool another Console's knowledge or create a special temporary-backfill policy. Exact storage, view replication and behaviour while no human is present remain to be specified. S073's deliberate Viewscreen sharing still follows the publishing operator's view; this decision alone does not automatically start or transfer a Viewscreen publication.

### State ownership implied by the selected directions

This is a proposed ownership map for the later data-flow report, not a committed ECS schema or a new transport protocol.

| State | Logical owner | Flow and handoff consequence |
|---|---|---|
| Physical resources, geometry and conditions | Entity/System mechanisms and their physical-part bindings | Authoritative processors update actual state. Filtered observation production controls what reaches a Console. Moving the GUI does not move or recreate machinery. |
| Operational intent / command channels | System | The effective controller submits admitted changes. Components read shared intent. Ordinary AI can change that intent when it controls the System. |
| AI policy memory | Individual System AI | Preserves the existing private, per-System policy structure; it is not pooled into Console knowledge or attached to the player's device. |
| Permitted observations, shared feeds and retained evidence | Console blackboard | Producers and declared information links supply facts. GUI and its System AIs can read them. Handoff preserves Console scope and existing evidence limits. |
| Working view | Console | Zoom, filters and inspected record follow the Console to the next operator, with responsive layout on the receiving device. |
| Personal accessibility and display preferences | Player/device presentation | Apply to that player's view without replacing Console knowledge, operational selections or the inherited work context. |

Local processing should use these explicit owners and data contracts. Remote clients receive permitted projections and submit admitted commands/view changes through existing transport; precise payloads, ordering and recovery remain for the dedicated flow report. Shared UI view state must not become a route to change machinery outside command admission.

## AD13 — What Workshop data can define: selected A

The proposal needs data-driven combinations of a limited mechanism library. Decide whether authoring new simulation behaviour is also part of the visual Workshop's scope. Existing scenario scripting remains supported in either option; this choice concerns reusable simulation mechanisms inside Systems.

- **A — Compose supported mechanisms.** Engine modules implement capacity, flow, heat, aiming, weapon effects and other approved primitives. Workshop data chooses instances, parameters, links, command bindings and presentation. New combinations need no new code; a genuinely new mechanism requires extending the supported implementation library.
- **B — Also author new mechanism logic visually.** In addition to composition, Workshop provides a behaviour/rule graph for defining new component calculations and effects from supported operations. This expands what content authors can express but adds a visual programming model and its validation/runtime contracts.

**Selected: A.** Workshop composes supported mechanisms, parameters, links, command bindings and presentation. New combinations of supported equipment are authored data; a genuinely new simulation mechanism extends the code library. The user's AI addition in AD14 permits configurable state-tree control logic built from supported game inputs and outputs. This does not permit custom node implementations, programming or extension of that supported vocabulary inside Workshop. Existing scenario-script support is a separate current feature; it is not an escape hatch for new AI mechanisms or privileged AI data access.

## AD14 — Configurable, reusable and inheritable AI state trees: selected user direction

The user specifies an Unreal-StateTree-inspired authoring model. Each System retains its individual AI, limited knowledge and ordinary communication, with a configurable state tree that can be reused and inherited. Workshop offers the inputs, conditions and outputs the game supports; authors configure supported behaviour rather than write programs or implement new node types there.

The [AI state-tree report](./ai-state-trees.html) separates three layers:

- **Engine-supported vocabulary:** typed facts, conditions, tasks/actions and bindings with implemented meanings. Extending this vocabulary is code work outside Workshop.
- **Authored definitions:** states, transitions, parameters, supported task choices, bindings, reusable tree references and inherited variants. AD15 selects parameter and structural inheritance; AD16 selects calls through reference nodes. Detailed merge and execution semantics remain to be resolved.
- **Per-System execution:** one AI instance per System, with its own active state and private memory. Reusing a definition does not share mutable AI state or knowledge between Systems. GUI and AI still use the same command-admission boundary; cross-System requests use ordinary communication.

Inputs come from permitted Console facts, delivered Coordination, supported own-System observations and private policy state, not unrestricted world reads. Outputs use supported System command channels and communication actions with existing authority. A tree can operate several channels of a composite System; its tasks are not extra independent component AIs. Behaviour must retain ordinary orders, alert/mission context and hysteresis rather than add a special temporary-backfill policy.

Current [policy.rs](../../../crates/phoenix-sim-gameplay/src/ai/policy.rs) supplies typed verbs, prioritised channel rules and optional named-state machines with private memory; [host.rs](../../../crates/phoenix-sim-gameplay/src/ai/host.rs) gates execution on control source and declared policy. This is useful implementation groundwork, not proof that the proposed hierarchy, reusable tree assets, inheritance or visual editor already exist.

Primary references checked 2026-10-12: Epic's [StateTree overview](https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-state-tree-in-unreal-engine) describes hierarchical states, conditions, tasks, transitions and data bindings; its [external StateTree guide](https://dev.epicgames.com/documentation/en-us/unreal-engine/external-statetree-quickstart-guide) documents linked reusable assets. These support the reference model. Phoenix asset inheritance is the user's additional requirement, not a claim that Unreal's linked-tree feature supplies those exact semantics. Epic's extension mechanisms are outside the selected Phoenix Workshop scope.

## AD15 — Scope of inherited AI changes: selected A

An AI variant must remain a configuration of supported nodes. Decide how much of its base tree it can change.

- **A — Parameters and structure.** A derived tree can override parameters and add or replace named states, tasks and transitions. For example, a turret variant can add a supported jam task to its inherited Engage state. The resolved tree is validated as a whole.
- **B — Declared variation points only.** A derived tree changes exposed parameters and predeclared subtree slots; other structure remains fixed by its base. This constrains variation but makes base behaviour easier to predict.

**Selected: A.** Derived trees can override parameters and add or replace named states, tasks and transitions. Validate the resolved tree as a whole, including inherited and local contributions. This supports equipment variations without copying whole trees or adding node code. Proposed implementation safeguards are stable authored node IDs, explicit overrides and a Workshop view showing inherited/local origin. Exact merge, removal and conflict rules remain to be specified. Structural inheritance does not extend the supported input/output vocabulary or enable programming inside Workshop.

## AD16 — Calling reusable state trees through reference nodes: selected user direction

The user additionally selects a reference node that calls into another state tree. This is separate from inheritance: inheritance derives a definition; a reference node invokes a reusable definition as part of an executing tree. A base or derived tree can contain reference nodes.

The referenced tree executes within the calling System's individual AI, using the same permitted Console context and control authority. It does not become another System controller, borrow another System's knowledge, or share mutable runtime state with every user of that asset. Reuse the definition while keeping execution state with the caller. Supported parameters and input/output bindings should connect the call site to the referenced tree; their precise interface is an implementation proposal still to specify.

Workshop should show the referenced asset and its bindings, let authors inspect the referenced definition, and validate it against the caller's supported inputs, outputs and equipment. These are proposed authoring consequences of the selected call mechanism. Calls may span simulation ticks; AD17 selects parent interruption. Completion outcomes, detailed exit cleanup, re-entry and recursive-reference rules remain open. Existing per-System execution and communication boundaries still apply.

## AD17 — Can the caller interrupt a referenced tree?: selected A

For example, an Engage state calls a reusable engagement tree, then a parent transition becomes eligible because of an order change or an observed overheating condition.

- **A — Parent transitions can interrupt.** The caller can leave the calling state before the referenced tree completes. Exit the affected subtree and follow the new transition under defined cleanup rules. Other unaffected active work is governed by the normal tree execution rules.
- **B — Calls finish before normal parent transitions.** Defer those transitions until the referenced tree completes, unless an explicit abort action or higher-level control-source change stops it. This favours completion of a called procedure over immediate reaction.

**Selected: A, with immediate tree reaction and potentially gradual System response.** An eligible parent transition interrupts the affected referenced subtree when the AI processes the new permitted input. The tree changes state and can issue the new order without waiting for the old call or the machinery to finish. This is a change in decision and intent: a turret may still traverse, a reactor may still ramp, and other mechanisms may need time to reach the requested state. Completed physical effects are not rewound.

Keep System command channels distinct from actual mechanism state. Both human and AI orders pass through ordinary admission and obey the same physical constraints. Immediate reaction means no added wait for subtree completion; it does not bypass information delivery, S062's simulated reading time where applicable, or deterministic simulation scheduling. The precise event/tick evaluation contract remains to be specified against the existing [AI cadence](../../../crates/phoenix-simulation/src/ai/cadence.rs).

For the later data contract, expose requested state separately from observed actual state and known transition/blocking reasons. This is a proposed presentation contract, subject to the same knowledge limits as other status information. Exact completion outcomes, transition priority, exit cleanup, channel persistence, re-entry and cancellation of already-started operations remain detailed execution questions. A tree transition alone does not make a physical operation instantly reversible or cancellable.

## AD18 — Competing outputs from active states and referenced trees: selected user direction

Within one System AI, a parent task might propose a default target while a called engagement tree proposes a different target. Attack and jam can use separate channels concurrently; this question concerns two proposals for the same channel.

- **A — Resolve by explicit priority.** Active tasks may propose values for the same channel. Resolve one winning value before submitting the System's commands. Reusable behaviour can supply defaults that more specific behaviour overrides.
- **B — Require one active writer per channel.** Treat overlapping output ownership as an invalid tree. Authors arrange transitions or bindings so only one active task can supply each channel.

**Selected: deterministic top-down execution; the last write to a channel takes priority.** This replaces the suggested numeric-priority model for tree outputs. Several active tasks can write the same channel; their authored execution order determines its final value. A later write to `target` replaces the earlier target without replacing unrelated channels such as `jam`. This rule applies within one System AI, not between independent System controllers, and does not alter human/AI authority or physical limits. The existing policy implementation's numeric priority rule is current-code evidence, not the selected new tree semantics.

Proposed traversal mapping: execute active parent work before descendant work, retain a stable authored order among tasks/siblings, and evaluate a referenced tree at its call site's position before continuing the caller. Show this order in Workshop, including inherited and referenced contributions. Persist semantic ordering in the asset; it must not depend on ECS iteration, map/hash iteration, task completion timing or incidental diagram coordinates. Exact transition selection and scheduling remain separate contracts; last-write-wins does not by itself select a rule for competing transitions.

Proposed output mapping: collect ordered writes into pending System intent, then submit the final values through ordinary admission before mechanisms consume them. Intermediate overwritten writes must not cause transient physical effects. This is a per-evaluation resolution proposal, not a new network protocol or authority bypass. One-shot actions and discrete jobs still need their own execution contracts; overwriting a channel cannot undo an already-launched projectile or sent message. Workshop should show the write sequence and final writer for each channel. AD19 selects visibility of earlier internal writes to later nodes within the same evaluation.

## AD19 — Reading earlier writes during a tree evaluation: selected A

AD18 fixes execution and output precedence. AD19 resolves whether a later node can read values written earlier in the same pass. For example, a parent task chooses a target and a later task decides whether to jam that chosen target. Both options keep external observations fixed to the permitted inputs for that evaluation and defer physical effects until the resolved commands reach the mechanisms.

- **A — Later nodes see earlier internal writes.** Private AI memory and pending System command values update as the ordered evaluation proceeds. Later supported nodes can read those values, allowing a reusable sequence to build a decision in stages.
- **B — All nodes read the starting values.** Each node reads the private state and command values from the start of the evaluation. Ordered writes still resolve the final output, but other nodes see the changes on the next evaluation.

**Selected: A.** Later nodes read private AI memory and pending System command values written earlier in the same evaluation. A target-selection task can therefore feed a later jam decision in that pass. AD18's last-write-wins rule still determines the final value of each channel. External observations remain the permitted facts captured for the evaluation; private writes do not change shared Console facts, another System AI's private state or actual machinery.

Supported bindings distinguish observed actual values from pending requested values. Reading a new pending target does not mean the turret has reached it. Earlier nodes are not automatically rerun when a later node changes a value. Exact input capture, transition-loop semantics and simulation scheduling remain implementation contracts to specify. Proposed Workshop tracing should show the values each node read, its writes and the resulting final intent, making sequential dependencies visible.

## AD20 — Fault visibility for AI-controlled Systems: selected A with repair reporting

The user requires AI-controlled Systems to be hidden from the ordinary System list. AD08 already defines the status indicator and evidence-limited details for visible Systems. Resolve whether the Console also retains a compact summary of the condition of its delegated Systems. This changes presentation, not delegation, knowledge access or control authority.

- **A — Keep a compact aggregate status.** The Console shows a summary covering its assigned Systems, including those under AI control. Known degradation or unavailability can be inspected through the summary without taking manual control or restoring all their controls to the normal list. Ordinary AI requests and urgent notifications still operate.
- **B — Surface delegated faults through AI reports.** Keep delegated Systems out of the routine status overview as well as the control list. Their AI uses normal communication and urgency rules to report relevant problems or request help; the operator inspects details from those reports or an explicit diagnostic view.

**Selected: A, plus a repair report from the affected System's AI.** Keep a compact Console summary of assigned Systems, including delegated ones, with inspection of known degradation or unavailability without taking control. The affected System's AI reports its need for repair through ordinary communication to Repair. Repair must receive those reports from 100% AI-operated Consoles in the same way; neither a human host nor an open GUI is a prerequisite for reporting or receipt.

Repair's human operator or individual Repair AI uses the delivered report to prioritise supported work. The reporting System does not acquire control of repair teams or repair itself merely by sending a report. Existing diagnosis, travel, access, parts, one-team-per-component and repair/patch rules still govern restoration. The report conveys available evidence and capability needs; uncertain symptoms remain uncertain, and a temporary condition such as EMP need not become an unnecessary repair job.

Proposed flow: permitted local observations feed both the compact status projection and the affected System AI; its typed report travels through normal Coordination to the authored Repair recipient. Receipt makes the delivered information available to the Repair Console and its effective controller. This supports AI-to-human and AI-to-AI reporting with the same knowledge and authority boundaries. An entirely AI-operated Console retains its blackboard, bindings and communication endpoint. Hidden UI is not inactive simulation.

Implementation groundwork already exists in [damage_sync.rs](../../../crates/phoenix-simulation/src/ship/damage_sync.rs), [coordination_systems.rs](../../../crates/phoenix-simulation/src/ship/coordination_systems.rs) and [Repair's receiver](../../../crates/phoenix-simulation/src/console/repair/server.rs): typed damage reports route to human recipients or the AI repair queue. That current implementation uses damage tiers and Station grouping; it does not implement the proposed component-condition model or prove the new Console routing. Map reports to observed conditions and stable System/part identities, preserve reporting when machinery is unavailable, and review payload filtering against the selected human/AI knowledge parity. AD21 selects a persistent work list updated by received reports and repair-team completion messages. Exact aggregation, thresholds, coalescing and record lifecycle remain to be designed.

## AD21 — Repair reports as a persistent work list: selected A with message-driven updates

AD20 settles generation and delivery, including fully AI-operated Consoles. Decide how Repair retains actionable reports after receipt. A report requests attention; it is not itself an admitted team-dispatch command.

- **A — Persistent Repair work list.** Human and AI Repair operators use the same outstanding-report records. Update a record as new evidence arrives, show whether it is waiting or being handled, and resolve it when supported reports or observations establish recovery. Routine receipt requires no acknowledgement click.
- **B — Messages and current status only.** Retain reports in normal communication history, but add no persistent outstanding-report list. Repair chooses work from those messages and currently available diagnostic information.

**Selected: A, updated by new reports and messages from completed repair teams.** Human and AI Repair operators use the same persistent outstanding-report records. Received reports update the known need; a team's completion message updates the work it performed and its reported outcome. Routine receipt requires no acknowledgement click. This applies equally to reports originating from fully AI-operated Consoles.

The work list is Repair's record of delivered information, separate from the authoritative machinery and team-job state. An underlying condition changing does not silently rewrite the list through an unrestricted world-state read. Recovery becomes known through a new report or the appropriate team message. A report remains advisory; the effective Repair controller still prioritises and dispatches actual teams through ordinary admission.

Completion must describe what was completed. A component repair does not imply that every fault on its System has cleared; an S077 fallback patch does not imply full restored effectiveness. Preserve reported remaining limitations. Proposed message contracts identify the relevant System/part, work item or job, source and observed sequence/time, so duplicate delivery or an old completion message cannot erase a later reported fault. Exact identifiers, matching/grouping, retention and reconciliation rules remain to specify. These are application data contracts carried through existing communication mechanisms, not a transport redesign.

The current AI request queue and human notification route are implementation groundwork, not the selected shared record model. In particular, existing pruning that inspects actual damage needs review: the proposed knowledge record changes through reports, with a separately owned physical execution record for the actual team work. Source publication, delivery and work-list reduction must preserve the same permitted information for human and AI Repair operators.

## AD22 — Shared job execution beyond the team framework: selected A

G037 already selects shared teams for facility work and ship repair/security. This question does not reopen that choice. It asks whether suitable finite activities beyond teams—such as fabrication, cargo transfers and focused scans—also share a job lifecycle. The AD21 outstanding-report list remains a knowledge record, distinct from any authoritative job execution state.

- **A — Shared job framework with activity-specific rules.** Reuse job identity, requirements/readiness, resource commitments, progress, interruption/cancellation and outcome reporting. Supported activity mechanisms define what work actually does and which lifecycle features apply. An individual System controller starts or changes its own jobs through admission.
- **B — Separate activity lifecycles.** Repair, fabrication, transfers and scans own separate execution records and state machines, while sharing lower-level mechanisms such as inventories, clocks and communication.

**Selected: A.** Repair, fabrication, cargo transfers, focused scans and other suitable finite activities reuse a common job framework. Shared contracts cover identity, requirements/readiness, resource commitments, progress, interruption/cancellation and outcome reporting. Activity-specific supported mechanisms define what physical work does, how it advances, which resources it uses, and how interruption, cancellation or recovery behave. Workshop composes supported job steps and parameters rather than implementing new mechanisms.

Keep three owners distinct: the individual System controller decides and submits admitted job commands; the authoritative job framework and activity mechanisms execute work; permitted observations and reports update Console knowledge. Shared processing is not a central AI scheduler. A job does not gain another System's command authority or pool private Console knowledge. AD21's Repair list remains a record of received information, not a direct view of authoritative execution state. Continuous mechanisms such as power flow need not become jobs.

Proposed implementation contracts:

| Shared job concern | Activity-specific meaning |
|---|---|
| Identity, owning System, target and authored definition | Supported target kinds, component/team/equipment bindings and actual admission rules. |
| Requirements and readiness | Repair access and suitable parts; fabricator inputs; valid cargo endpoints; scanner pointing/range/power. |
| Resource commitments | Which discrete stocks or equipment slots can be reserved, when consumption occurs, and what remains committed during recovery. AD23 reserves for the next stage when it can begin, rather than reserving all queued work up front. |
| Progress and lifecycle | Domain work updates shared lifecycle state. Phases need not use one universal percentage or advance by elapsed time alone. |
| Interruption, cancellation and outcomes | Domain rules preserve completed physical effects and govern safe stopping, retained progress, salvage and return travel. Ending work does not necessarily free a deployed team immediately. |
| Permitted reporting | Supported reports expose the appropriate outcome/evidence to their recipients; execution state is not automatically shared with all Consoles. |

This is a logical contract proposal, not a final ECS schema, module split or implementation claim. The existing pure [repair-team state machine](../../../crates/phoenix-sim-gameplay/src/modifiers/repair_teams.rs) is domain groundwork rather than proof of a generic job engine. Initial supported activities, concrete stage vocabulary, scheduling and migration still need mapping against current code and the selected catalogue rules.

## AD23 — When queued jobs reserve scarce resources: selected A

AD22 includes resource commitments. Decide when a job claims discrete resources such as parts, ammunition ingredients or equipment slots. This is distinct from consuming them, which remains activity-specific, and from continuous power/coolant allocation. Both options honour existing reservations and explicit reserve policies; queueing alone does not manufacture, move or duplicate stock.

- **A — Reserve for the next stage when it can begin.** Queued jobs express demand. Acquire the resources needed for the next executable stage when committing to that stage, including an explicit preparation stage where supported. Distant queued work does not lock all future requirements.
- **B — Reserve on queue acceptance.** Accept a job into the queue only with reservations for its required discrete resources. Its planned supplies remain committed until used, released or explicitly reassigned, even while the job waits to run.

**Selected: A.** Queued jobs express demand. Reserve the discrete resources needed for the next executable stage when committing to that stage, including an explicit preparation stage where supported. Do not reserve all future requirements merely because a job joins a queue. A long fabrication queue therefore leaves uncommitted supplies available for other work.

Reservation is distinct from consumption and physical delivery. The activity defines when reserved parts are consumed, how stocks reach the work site, and which equipment remains committed during stopping or recovery. A preparation stage may commit supplies before the main work starts; a deployed team does not become available elsewhere simply because its repair has finished. Existing resource owners, reserve policies and actual availability continue to apply. The shared job framework does not gain authority to take another System's committed stock or equipment.

Proposed Workshop/runtime presentation distinguishes queued demand, reserved resources, consumed resources and physical commitments. AD66 requires the next indivisible step to secure its required set together, retaining no new partial claims on a failed attempt. Exact allocation order, release on interruption, staging and scheduling remain implementation contracts to define against each activity. Continuous power/coolant allocation is governed by its existing selected rules, not by a queue reservation for an entire job.

## AD24 — State-tree authoring for job workflows: selected A

AD22 selects a shared execution framework and AD23 its reservation timing. Decide how Workshop defines a job's sequence of supported work, such as travel, inspect, repair and return. AI decision-making and actual job execution retain distinct instances, data and authority under either option.

- **A — Reuse the state-tree format and machinery.** Job definitions use the same authoring concepts and reusable editor/evaluator as AI trees, with an explicitly supported job-node vocabulary. Job nodes drive authorised activity steps, readiness and outcomes; AI nodes choose and request work. Each job has its own execution state.
- **B — Give jobs a dedicated stage format.** Define an ordered set of supported stages and lifecycle settings in a simpler job editor/executor. AI state trees request jobs but the job definitions do not use that tree format.

**Selected: A.** Job workflows reuse the state-tree format, editor and evaluator selected for AI, including reusable definitions, inheritance and reference calls. Each job has its own execution state and supported job-node vocabulary. The System's AI tree chooses and requests work from its permitted knowledge; a job tree executes the admitted work. A human can start the same job through ordinary admission without an AI decision tree being involved.

Keep the two execution contexts explicit in the schema and Workshop. AI nodes use permitted Console facts, private policy memory and ordinary commands/communication. Job nodes operate on the assigned work, its readiness, commitments and supported activity mechanisms within the admitted job's scope. Reusing the evaluator does not merge memory, expand AI knowledge or create another independent System controller. Physical work remains implemented by supported code mechanisms; Workshop supplies configuration and bindings, not new node implementations.

Use the same deterministic ordering and visibility of earlier internal writes for compatible tree operations. Pending job values are distinct from AI intent and observed physical state. An effectful job step must obey the activity's resource/progress/authority contract; an overwritten value or repeated evaluation cannot itself duplicate material consumption or completed work. Exact effect activation, exit cleanup and replay contracts still need specification against the shared evaluator. AD17's immediate decision changes do not make a physical stage complete instantly, and an AI state change alone does not reset existing job execution.

Proposed Workshop support shows an AI or Job context, filters node/binding choices accordingly, validates inherited and referenced definitions in that context, and traces decisions, stage progress and reports. References reuse behaviour within a calling instance; AD25 selects independent jobs without separately tracked child jobs in the initial framework. A repair workflow might travel, inspect, perform the supported repair/patch, report the result and return; this is an illustrative composition, not a final stage schema or a claim that return travel and repair completion are the same event.

## AD25 — Independent jobs or parent/child job hierarchies: selected A

AD24's reference nodes reuse a workflow within one job instance. Decide whether the first shared job framework also needs a job to create and supervise separate child jobs, with their own identities, resource commitments and outcomes. Independent jobs can already run concurrently under their System controllers; S075's parallel repairs on different components remain supported in either option.

- **A — Independent jobs initially.** Each job owns its workflow and can call reusable subtrees. System controllers start and coordinate separate jobs, using ordinary requests/reports across Systems. No additional parent/child job lifecycle is introduced in the first framework.
- **B — Nested jobs too.** A job can launch separately tracked child jobs within its admitted authority and wait for their outcomes. Define parent completion, cancellation propagation, child resource ownership and recovery explicitly. Work owned by another System still requires its normal request/admission path.

**Selected: A for the initial framework.** Jobs are independent execution instances. Each owns its workflow and can call reusable subtrees within that instance. System controllers start and coordinate separate jobs, including jobs running concurrently, and use ordinary requests/reports across System boundaries. Do not introduce separately tracked parent/child jobs, child spawning or parent-wide completion/cancellation propagation in this initial design.

S075's parallel component repairs remain separate jobs with their own teams and commitments. A reference call reuses behaviour without creating another job identity, resource owner or controller. Supported concurrency inside a tree remains possible under its execution rules; this choice limits job nesting, not all parallel work. Cancellation or interruption of one job follows its own activity rules and does not implicitly cancel other independent jobs. Review nesting later only if a concrete workflow requires it; no such expansion is selected now.

## AD26 — Shared condition and modifier framework: selected A

The catalogue selects component faults (G057), condition-specific escalation (G059), stress-dependent wear (G060), officer bonuses (G068) and temporary EMP/resistance effects (S010). Decide how much common infrastructure these use. This is an architecture choice, not a reopening of their selected gameplay behaviour.

- **A — Shared conditions and modifiers.** Use common condition identity/source tracking, supported duration/removal/reapplication/stacking rules and typed capability modifiers. Damage, environment and officer-assignment mechanisms apply appropriate definitions; each still owns the cause and activity-specific consequences.
- **B — Separate condition lifecycles.** Faults, temporary effects and officer bonuses retain separate records and lifecycle handling. Reuse only lower-level modifier arithmetic and status presentation where appropriate.

**Selected: A.** Component faults, temporary effects and Duty Officer bonuses use a shared condition and modifier framework. Reuse condition identity/source tracking, supported duration/removal/reapplication/stacking rules and typed capability modifiers. Domain mechanisms retain each condition's cause, physical consequences and removal requirements. Repair can address a damaged part, EMP can expire, and reassignment can withdraw an officer's contribution without treating these as the same remedy.

This provides common Workshop authoring, provenance and effective-capability calculations. Authors compose supported rules and parameters; genuinely new effect operations require code under AD13. Exact target kinds, stacking/composition formulas, effect ordering and data contracts remain to be proposed. AD27 selects compact lifecycle rules for condition progression in the initial framework.

Current [ShipModifiers](../../../crates/phoenix-sim-gameplay/src/modifiers/cache.rs) already keys numeric contributions by source and slot, replaces an existing contribution with the same key, combines distinct contributions deterministically, and tracks source-backed flags. Its float calculation sums bonuses then converts that sum to a multiplier; integer bonuses sum directly. This is implementation groundwork, not a complete generic condition engine or a choice to retain every existing formula for the new design.

Keep base parameters, actual physical state, effective capability and observed status distinct. Removing an officer bonus removes that source's contribution; it does not restore damaged hardware. Expiry of a temporary effect is not physical repair. Destroyed parts, energy stocks, heat and spatial damage remain facts owned by their mechanisms, with supported conditions reflecting or affecting capability. G059's escalation/spread remains condition-specific causal work; sharing condition storage does not make every effect spread or add routine deterioration to healthy operation. Status publication still preserves the selected diagnosis and Console knowledge limits.

## AD27 — Authoring condition progression: selected A

AD26 selects common condition records and modifier handling. G059 permits specific faults to escalate or spread, while S010 already specifies refreshable EMP and stacking resistance. Decide whether evolving conditions need the state-tree authoring machinery selected for AI and jobs, or a smaller set of supported lifecycle rules. Neither option adds a component AI or changes the already selected gameplay rules.

- **A — Compact lifecycle rules initially.** Authors configure supported application, refresh/stack, expiry/removal and event/threshold reactions. The owning physical mechanism performs supported consequences, including eligible escalation or propagation. No separate state-tree instance runs for each condition; genuinely new reaction operations require code.
- **B — Optional condition state trees.** Simple conditions still use common lifecycle fields, but more elaborate conditions can reference a state tree with a restricted condition-node vocabulary and their own execution state. Reuse the editor/evaluator, with explicit contracts for tree lifetime, reapplication, stacking and removal.

**Selected: A for the initial framework.** Workshop authors configure compact supported rules for application, refresh/stacking, expiry/removal and event/threshold reactions. The owning physical mechanisms perform supported consequences, including condition-specific escalation or propagation. Conditions do not each run a separate state-tree instance. New reaction operations require code under AD13; this is not an arbitrary expression or scripting extension.

The existing EMP refresh/resistance rules, stable faults, stress-dependent wear and condition-specific spread remain the gameplay requirements. Common authoring should expose their source, duration, stacks and supported consequences without requiring authors to build a workflow tree for each effect. Optional condition trees can be reconsidered if a concrete behaviour requires them; they are not part of the selected initial scope.

Propagation follows supported physical dependencies and containment rather than arbitrary target selection. Physical mechanisms may read the authoritative state they simulate; human and AI decision inputs still come from permitted observations. The condition framework does not grant control of another System, share private AI memory or expose undiscovered faults. Exact rule vocabulary, evaluation timing, reaction ordering and repeat/once semantics remain design contracts to specify.

## AD28 — Authoring default resource-service connections: selected A

AD04 and AD06 already require consumer-owned resource-link components, actual shared supply and distinct damage state for the source and its receiving links. This choice concerns how Workshop authors connect a reusable System to a particular ship's services; it does not reopen that runtime model or choose automatic rerouting.

- **A — Named ship defaults with explicit overrides.** A reusable consumer can request a typed named service such as `main_coolant`. The ship binds that name to a concrete provider. Individual connections can override the default; Workshop shows the resolved provider and whether the binding is inherited or local.
- **B — Explicit connection for every consumer.** Each consumer names its concrete provider directly in the ship definition. Templates can preserve those connections, but there is no named default-service resolution step.

**Selected: A.** Reusable consumers can request typed named services such as `main_coolant`; the ship binds each service name to a concrete provider. An individual connection can override that binding. Workshop shows the resolved provider and whether the binding is inherited or local. This supports ordinary supply defaults and dedicated pumps without reconnecting every reusable System by hand.

The default is authored data resolved during content preparation, not a nearest-provider search or a runtime allocator. It selects the supplier; it does not create a free physical path. S039's coarse damageable conduit routes still determine actual connectivity and delivery. Exact route authoring, inheritance and override syntax remain open.

Proposed preparation resolves named services to concrete typed endpoints using the same content validation for Workshop Test and normal loading. Missing required services, incompatible types and ambiguous bindings should be reported before launch; explicitly optional or intentionally disconnected inputs need their own supported representation. Exact inheritance and override syntax remain open.

Retain individual receiver links and count provider stock/capacity once. Damage or depletion changes actual delivery through the resolved connection; it does not silently bind the consumer to a different provider. Any bypass or switching mechanism must be explicitly supported and operated through its normal authority. This selection is scoped to resource services: it does not introduce default sharing of Console knowledge, cross-System turret references or new command authority.

## AD29 — Shared recipes for batch and continuous resource conversion: selected A

The gameplay review selects onboard fabrication (G054), optional lossy coolant recycling (S034), real actor stocks/production (G038) and bounded recovery/processing (G046, S089–S092). AD22–25 provide finite jobs; AD04/06/28 provide resource connections. Decide whether finite production and continuous processing also share a reusable recipe definition, rather than each family defining its own input/output schema.

- **A — Shared recipes, separate execution modes.** A supported recipe describes typed inputs, outputs, ratios and operating requirements. Batch jobs and continuous processors use that same vocabulary and accounting contracts, while retaining their distinct progress, rate, readiness and interruption rules.
- **B — Recipes for batch jobs only.** Fabrication and other finite jobs share recipe definitions. Continuous recycling, generation and similar processes retain their own mechanism-specific conversion parameters and accounting, reusing stores and service links where applicable.

**Selected: A.** Batch and continuous resource processes share recipe definitions for typed inputs, outputs, ratios and operating requirements, together with common accounting contracts. Finite jobs and continuous processors retain distinct execution modes: progress, rate, readiness, interruption and physical consequences remain appropriate to the activity. Ship equipment and facility production use the same vocabulary where those semantics match.

The shared definition does not require every process to use one solver or make continuous operation a queued job. Supported code still defines each execution mode, resource type and additional physical behaviour. A recipe describes the resource conversion; the job or processor owns its execution, and the actual stores own material quantities. An individual System controller chooses supported operation through ordinary authority.

For example, ammunition fabrication would run a finite job consuming supported material stocks; a fitted recycler could process used coolant at a bounded rate with an authored loss. These are illustrative mappings, not a decision about recycling mode, numerical yields or when every input is consumed. Actor production uses the same mechanisms where applicable, with ordinary System control, actual stocks and capacity.

Keep real resource state, reservations, transferred stock and output distinct. AD23's stage-start reservation applies to discrete job commitments; continuous power/cooling retains its selected allocation rules. A shared recipe cannot bypass physical delivery, output storage capacity, reactor ramping, local battery heat, thermal limits, diagnosis or control authority. It does not add a fuel loop to every reactor or turn every sensor/weapon effect into material production.

AD62 constrains conversion when a required stored output cannot fit. Exact recipe fields, supported resource units, fractional-progress accounting, operating-cost timing and cancellation recovery remain to be specified; AD63 clarifies disposal when changing recipes. AD22 already leaves interruption and physical recovery rules to the activity; shared recipes do not imply a universal refund or progress reset. Workshop supplies supported data and bindings under AD13, not arbitrary conversion code. This is a selected proposal boundary, not a claim that a generic recipe engine already exists.

## AD30 — Visual state trees for scenario progression: selected A

AD14–19 select the shared tree authoring/evaluation machinery for System AI, and AD24 adds a separate Job context. AD27 keeps condition lifecycles compact. Decide whether mission phases, objective progression and authored event responses should also gain a visual tree context. Existing scenario scripting remains supported under AD13; neither option removes it or changes the selected gameplay outcomes.

- **A — Add a Scenario tree context.** Reuse the visual tree editor and supported evaluator machinery for scenario progression, with a restricted vocabulary of scenario observations, objectives, timers and effects. Reusable mission structures can be authored visually. Keep existing Rhai scripts supported; precise integration and ownership between scripts and trees must be specified.
- **B — Keep scenario progression in the existing authoring model.** Continue using Rhai, declarative scenario data and supporting Workshop forms. Reuse the underlying world/gameplay mechanisms, while keeping state-tree authoring scoped to AI and jobs.

**Selected: A.** Add a Scenario context to the shared visual state-tree authoring and supported evaluation machinery. Authors can compose mission phases, objective progression, timers and authored event responses through supported scenario nodes and reusable structures. Scenario instances have their own state, inputs and effect vocabulary; sharing the editor/evaluator does not expand individual System AI knowledge or authority. Existing Rhai scripts remain supported.

Current evidence: [Workshop Scripts](../../../editor/workshop-scripts.js) routes script editing through the existing document workflow. The [world scripting host](../../../crates/phoenix-sim-world/src/world/script/mod.rs) describes Rhai and declarative triggers using the same trigger evaluator, and script effects using the existing `ActionCmd` vocabulary. A Scenario tree should reuse those world contracts where appropriate, rather than establish a second physical simulation or a parallel effect path. Whether tree assets execute directly or lower into existing world structures is an implementation decision still to examine.

Scenario execution has its own authoritative context and state in the proposal. NPC System AI still acts on permitted Console knowledge, received orders and ordinary commands. A scenario transition cannot stand in for actual material delivery, team work or repair completion; world rules observe physical results and produce supported consequences under G080. Optional GM intervention remains separate, and trees must not require a GM to advance normal play.

Before implementation, define stable identities, script/tree interaction, effect ordering and single ownership of progression state, snapshot/resume, tracing and PASM design traceability. AD31 selects explicit Scenario script-call nodes. Do not silently translate or replace existing authored scenarios. No scenario timing, deadlines, world file or PASM slice is changed by this selected architecture direction.

## AD31 — Rhai integration within Scenario trees: selected A

AD30 retains existing scenario scripts while adding visual progression. Decide whether a Scenario tree can invoke a Rhai routine as an explicit task, or whether scripts only communicate with it through separately handled events and declared scenario facts. This concerns the Scenario context only; it does not introduce custom script nodes for System AI or jobs.

- **A — Explicit script-call nodes.** A Scenario node names a validated Rhai routine and supplies supported inputs. The call can produce supported results and buffered world effects; the tree uses its declared transitions to continue. Workshop shows the call in the visual flow and links to its script source.
- **B — Separate script handlers.** Scenario trees use only the built-in scenario-node vocabulary. Scripts remain separate handlers, communicating through declared events/facts; no tree node calls a Rhai routine directly.

**Selected: A.** Scenario trees support explicit script-call nodes. A node names a validated Rhai routine, supplies supported inputs and receives declared results alongside supported buffered world effects. The tree continues through its declared transitions. Workshop shows the call at its place in the visual flow and links to the script source. Existing independently triggered scenario scripts remain supported.

Current [script invocation](../../../crates/phoenix-simulation/src/world/script_invocation.rs) centralises retained-script lookup, the shared tick budget and scoped read context; effect calls return the existing buffered `CallEffects`. [Script effects](../../../crates/phoenix-sim-world/src/world/script/effects.rs) route through the established world action vocabulary and separate Comms request path. These are integration seams, not an implemented generic tree-call/result API.

The proposed node uses the existing sandbox and invocation boundary. Script code does not write a tree's private active-state record or access another controller's memory; progression reacts through declared results, events and transitions. World facts/effects keep their own ownership and application contracts. This facility is specific to the Scenario context: it does not introduce script-call extensions for System AI or Job trees, add new simulation mechanisms or bypass physical work, information limits or System command admission.

Exact input/result schemas, call activation, completion/error outcomes, effect application order, delayed callbacks and cancellation/save-resume behaviour remain to be specified. A successful call is not evidence that a requested physical action has finished. AD18's last-write rule for System command channels must not be applied indiscriminately to irreversible world effects such as spawns. This selection does not automatically convert existing scripts or authorise a second writer of private tree state.

## AD32 — Independent live instances of reusable situations: selected A

G084–G085 already select bounded Director pacing and reusable situation libraries; G112 permits actual work by outside actors to change or resolve needs. AD30–31 add visual Scenario trees and script-call nodes. Decide how a reusable situation runs when the world contains several concurrent encounters, potentially using the same template with different subjects.

- **A — Independently instantiated situations.** Give each live situation its own instance identity, parameters, entity/location bindings, local progression state and lifecycle. Several instances can reuse one Scenario tree/template at once. The containing scenario or Director starts suitable instances under the agreed pacing and causality constraints.
- **B — One world progression instance.** Reuse situations as referenced subtrees inside the world's main Scenario tree. That world instance owns their progress and lifetime together; concurrent situations are composed into its authored structure rather than started as independently tracked instances.

**Selected: A.** Reusable situations run as independent live instances, each with its own identity, parameters, entity/location bindings, local progression state and lifecycle. Several instances can use the same Scenario tree/template concurrently. The containing scenario or bounded Director starts suitable instances under the agreed pacing and causality constraints. Two rescues can therefore reuse one definition with different ships and outcomes, and each instance remains individually inspectable.

Current [world layer loading](../../../crates/phoenix-simulation/src/world/layers.rs) already supports additive content, but `LayerLoadOutcome::AlreadyLoaded` treats an already-present path as a no-op. Existing layer-scoped registrations and continuation are useful groundwork, not proof of multiple simultaneous instances of the same template. The selected model needs distinct template and live instance identities rather than assuming a source path is both.

Proposed ownership mapping for the selected direction:

| Record | Owner and purpose |
|---|---|
| Situation definition | Reusable immutable Scenario tree, parameters and supported setup/constraints. |
| Live situation instance | Its bound subjects, local progression, scheduled work and outcomes; stable identity for inspection and save/resume. |
| Physical entities and work | The shared world and ordinary System/job mechanisms; an encounter references them without cloning their stocks, damage or AI knowledge. |
| Objectives and recipients | Existing explicit objective-instance and assignment semantics; situation membership alone does not award completion or broaden visibility. |

Situation completion is not automatic world rollback or despawn. Physical consequences, surviving actors, committed work and received information follow their normal lifetime rules. Two situations can affect the same actor only through supported shared facts/effects and explicit bindings; private progression is not pooled. The Director remains a bounded pacing service, not a controller for all ships. GM intervention remains optional.

Exact activation/admission, binding conflicts, persistent outcomes, scheduling, teardown, content compatibility and snapshot mapping remain to be specified. AD33 selects declared inputs, events and outcomes for coordination between instances. AD25's independent-job choice is unchanged; a situation instance is scenario progression, not a parent job with authority over other Systems. This selection changes neither networking nor individual AI knowledge limits.

## AD33 — Coordination between situation instances: selected A

Independent instances still share one physical world. G083 already allows supported causal interactions to combine during play, and G112 allows outside actors to change a need through actual work. Do not require a bespoke inter-situation message before ordinary resource shortages, damage or actor actions have their physical consequences. This question concerns deliberate dependencies between authored progression logic, such as an investigation waiting for a rescue outcome.

- **A — Declared inputs, events and outcomes.** A situation publishes explicitly supported facts/outcomes and events. Other instances bind to that interface, with durable outputs available when needed, rather than inspecting its internal tree phase or local variables. Authored requests are handled by the receiving instance's own rules.
- **B — Direct read bindings to another instance's internals.** Authors can bind a condition to another named instance's active state or local variables. Each instance still owns its own writes, but its internal tree structure becomes part of the dependency.

**Selected: A.** Situations coordinate authored progression through declared inputs, events and outcomes. Consumers bind to that interface; they do not inspect another instance's private tree state or local variables. Keep durable outputs where later consumers need them, and handle requests through the receiving instance's own rules. An investigation can depend on `survivors_recovered` while the rescue's internal implementation changes independently.

Proposed mapping: use typed world/situation contracts and scoped bindings, drawing on the existing [WorldEvent vocabulary](../../../crates/phoenix-sim-world/src/world/content.rs) and [Objective lifecycle/recipient contracts](../../../wiki/concepts/objectives.md). This does not require a network message for every local event or a second physics/event simulation. Exact persistent-output storage, delivery, replay/deduplication, late binding and instance-end retention remain to be specified. A transient event alone must not stand in for a durable outcome that a later situation needs to inspect.

Published here means available to declared scenario consumers, not broadcast to players or every System AI. Console observations, Comms and Objective recipient rules still determine who learns what. Scenario-private state remains distinct from world state; a public outcome is only valid when its supported completion conditions are met, not merely because a script reached a conveniently named state. Shared actor/resource bindings preserve normal ownership and do not reserve all of an actor's capabilities for one situation.

## AD34 — Shared observation and report records: selected A

AD02/07 select permitted Console knowledge and explicit information feeds. G018–G022 already require finding-specific confidence, observed history, provenance and handling of conflicting information; S063 preserves observation time and evidence identity when delayed probe/sensor history arrives. AD20–21 add repair reports and completion messages. Decide whether these informational records share a common metadata contract, while retaining typed domain payloads.

- **A — Common record metadata, typed findings.** Observations and structured reports share identity, subject, provenance and observation/receipt timing conventions, with confidence or uncertainty where applicable. Radar readings, repair findings and shared intelligence retain their own typed contents and interpretation. Shared history, copying and deduplication support can serve Console views and individual System AI.
- **B — Separate domain record contracts.** Sensors, repair and intelligence each define their own metadata and history handling. Console adapters translate their records when a display or AI needs to combine them.

**Selected: A.** Observations and structured reports share identity, subject, provenance and observation/receipt timing conventions, with confidence or uncertainty where applicable. Each domain retains its own typed findings and interpretation. Console views and individual System AI reuse history, copying and deduplication support. Evidence forwarded from a probe through Science to Tactical retains its original age and lineage; forwarding does not create an independent observation.

Current groundwork is narrower: the [dossier evidence log](../../../crates/phoenix-sim-world/src/dossier/evidence.rs) tracks typed provenance and deduplicates authored findings by subject, provenance and text; [ShipScanRecord](../../../crates/phoenix-simulation/src/science/server.rs) retains a scan reading, while [ScanReadingSnapshot](../../../crates/phoenix-model/src/messages.rs) carries the observation tick and measurement precision. These are separate implemented contracts, not an existing universal Console observation store. The dossier's current text-based deduplication rule must not become a universal identity rule for repeated measurements.

The proposed contract covers learned observations and structured reports, not every physical state update, command or private Scenario event. AD33 progression outcomes remain authoritative scenario interfaces; knowledge records express what an observer has learned or been told. A recorded claim can be inaccurate, disputed or incomplete. Source labels alone do not guarantee truth, and a contact reference must not expose an identity the observer has not discovered.

Scope and sharing rules remain explicit: automatic feeds publish only their permitted content, co-hosted tabs do not pool knowledge, and humans and AI read the same permitted Console facts. Receipt time belongs to the receiving context; forwarding preserves the original observation and evidence lineage. Do not invent a fresh confidence score when no meaningful estimate exists, or treat copies as corroboration. Free-text conversation does not automatically become a parsed structured finding.

Exact schemas, identity/correlation rules, revisions, fusion/contradiction handling, retention limits and storage layout remain to be specified. A common contract need not use one physical store or retain every sample forever. Local data-oriented access and remote projections still use their appropriate paths; this adds no new transport protocol and does not require every record to become a network message.

## AD35 — Ownership of combined Console knowledge: selected A

AD02 already allows humans and individual System AIs to read the same combined Console picture, and AD34 selects shared record metadata. The remaining architecture choice is where multiple permitted findings become a usable current assessment. This does not reopen automatic broad assessment, faction feed selection, confidence by finding or the decision to preserve contradictory evidence.

- **A — Shared Console knowledge processors.** Reusable typed processors derive the Console's combined contact, status and other supported views from its permitted records. Humans and System AIs consume those same assessments and can inspect their supporting evidence. Producers own observation acquisition; Console processing owns the derived view.
- **B — Producer Systems own combination.** Relevant Systems consume explicitly permitted feeds and publish their own combined assessments. The Console stores and presents those results without a separate shared processing stage. Humans and AI still have equal access to the published results.

**Selected: A.** Reusable typed Console knowledge processors derive combined contact, status and other supported views from the Console's permitted records. Humans and individual System AIs read those same assessments and can inspect supporting evidence. Producers own observation acquisition; Console processing owns the derived view. Domain-specific processors retain their own rules: position estimation and repair-report updates share metadata/history support without using one universal combination formula.

Current [sensor publication](../../../crates/phoenix-simulation/src/ship/sensors.rs) builds typed blackboards within the simulation, and [Console view builders](../../../gui/console-state.js) construct presentation views in JavaScript. These are integration points; neither establishes the proposed generic Console knowledge pipeline. A common derived assessment can be projected to the existing pure-JS client while System AI reads the corresponding permitted local data. Pixel geometry, zoom and presentation filtering remain GUI concerns.

Workshop binds supported processors and their typed inputs/outputs as Console data. These are reusable processing mechanisms, not an additional AI controller or a separately operated ship System. Implementations can batch data-oriented work across Consoles while preserving logical knowledge scopes. New processor operations still require code under AD13; authors cannot insert arbitrary inference scripts.

Keep source records and derived estimates distinguishable, with traceable contributing evidence and meaningful uncertainty. No universal newest-wins rule: an old identity, a new uncertain position, a claim and a diagnosis have different semantics. Flag unresolved contradictions instead of silently turning one claim into truth; forwarding an estimate must preserve its evidence lineage. Record retention remains separately bounded, rather than requiring every raw sample forever.

Shared processing grants no hidden world identity, position, faults or intentions, does not change another System's target, and creates no cross-Console sharing. Correlation may use only available evidence; exact association and confidence rules remain open. Interpretation of significance, trust and action stays with the operator or ordinary individual System AI. This processing does not add a gameplay analysis timer or bypass S062's AI reading-time requirement. Exact scheduling, invalidation, revision tracking, persistence and network projections remain implementation contracts to specify.

## AD36 — Reusable assemblies containing several Systems: selected A

The user wants visual composition of ships from a reusable library. AD04/05 establish that a System is one command scope; independently aimed turrets and a separately controlled coolant pump remain separate Systems. Decide whether a single Workshop library item can package those connected Systems for repeated placement, or whether equipment library placement initially stops at individual Systems.

- **A — Reusable multi-System assemblies.** An authored assembly can contain several Systems, their parts and internal connections. Place it more than once with distinct instance identities, local parameters, placement and external bindings. For example, a two-turret battery with its pump can be placed port and starboard without rebuilding its internal links.
- **B — Individual System templates initially.** Reuse each turret and pump definition, but place and connect them individually on the hull. Keep whole-entity composition/inheritance; defer separately instantiable equipment assemblies.

**Selected: A.** Workshop supports reusable assemblies containing several connected Systems, their parts and internal links. Each placement has distinct identities, parameters, placement and external bindings. The assembly is resolved content, not another command owner: each turret retains its target, controlling Console and individual AI, and the pump retains its own controls. Shared service names and per-link overrides continue to work under AD28. AD37 adds the user's physical compartment ownership and component/System/compartment template units.

Current [entity include resolution](../../../crates/phoenix-content/src/include_resolve.rs) already composes ordered fragments into one resolved template with field provenance, then applies world-instance overrides separately. Keyed arrays support inherited extensions/overrides under specific merge policies. [Workshop entity composition](../../../editor/workshop-entity-composition.js) exposes source-preserving include and override operations. Preserve that foundation. Repeated local assembly instances need additional identity and binding semantics; a fragment included into one entity is not proof that the same connected subassembly can already be independently instantiated twice.

A reusable definition declares internal references and supported external bindings. Expansion must give each placed System/part/link a distinct identity and remap internal references within that placement, so port and starboard batteries cannot accidentally share a capacitor, AI memory or turret target. Shared ship services remain explicit external references; they must not be copied with the assembly. Exact syntax, definition inheritance, nested assembly scope, placement transforms, override/removal semantics and compatibility remain to be designed.

An authoring assembly is not itself a physical ship entity, a new repair target, a shared blackboard or a controller. Its contents can include a real compartment under AD37. Damage follows the selected layout rules with compartment HP and component conditions under AD44, and each System still has exactly one controlling Console. Assembly placement does not grant access to another Console's information or turn several independently controlled devices into one System. Deployment continues to create ordinary configured ship entities under S019; this choice does not introduce a second deployable type.

Reuse normal content preparation and Workshop validation/preview/Test. No live-session hot reload, retroactive runtime mutation or migration of existing content is selected here. A library edit's effect on derived content must remain inspectable and validated before use.

## AD37 — Physical compartments and template units: selected user direction

**Selected:** Systems live physically in a compartment and belong to a Console for control. Workshop can create a reusable template from a component, a System or a compartment. This makes physical containment an explicit part of entity composition alongside the separate Station/Console control structure.

| Relationship | Meaning |
|---|---|
| Ship → deck → compartment → System | Physical home on a deck's 2D layout, relevant to damage, local hazards, containment and access. AD45 adds multiple decks. |
| Ship → Station → hosted Console → controlled System | Control and presentation assignment, with one controlling Console per System. |
| System → components / physical-part bindings | Shared command channels and composed mechanisms; AD03 distinguishes mechanism records from physical condition/repair targets. AD44 assigns HP to compartments. |

The two relationships reference the same System instance. Moving or hosting a Console elsewhere does not move machinery. Several Systems in one compartment can be controlled by different Consoles, and one Console can control Systems spread across compartments. Physical proximity does not pool Console knowledge, transfer command authority or add a compartment AI.

| Template unit | Reusable authored content and placement needs |
|---|---|
| Component | Supported mechanism configuration and explicit bindings. Place it within a System; bind required command channels, resources, information or physical parts. A purely logical component does not acquire a damageable body just because it is templated. |
| System | Components, shared command channels, individual AI definition and physical-part bindings. Assignment supplies its physical compartment, controlling Console and external connections; components have no precise position within the compartment. |
| Compartment | Physical definition, material and HP configuration, fitted Systems, local layout and internal connections. Placement supplies a ship/deck location and external resource, access and control bindings. Current HP and containment/safety state belong to the live compartment; component conditions and System authority retain their separate owners. |

These extend the existing entity, Console and state-tree reuse directions rather than replacing them. Creating a template extracts authored definitions and internal references, not live battery contents, damage, AI memory or observed knowledge. Independent placements remap internal identities and keep external references explicit under AD36. Exact template file schema, stable IDs, inheritance/override syntax, exposed bindings, preview and dependency migration remain to be specified.

S029 already selects coarse physical compartments; S102 selects coarse atmosphere conditions and safety margins. S016 uses the physical layout for damage and S039 selects damageable conduit routes. This refinement joins those gameplay concepts to the composition model without adding detailed pressure simulation. AD38 keeps a System's equipment within its compartment while retaining cross-compartment service routes.

Current [damage/repair implementation](../../../wiki/concepts/damage-and-repair-intent.md) is organised around fine-System damage and Station-oriented visibility/work. It is grounding for migration, not an implementation of this compartment composition. Keep intended physical layout and control ownership explicit in the proposed schema before mapping them to ECS storage, repair routing and published views.

## AD38 — Can a System's equipment span compartments?: selected A

AD37 establishes one physical home compartment per System. Resolve whether that also contains all its equipment, or whether the home is an anchor while individual physical parts can be placed elsewhere. This is distinct from resource conduits crossing compartment boundaries, which S039 already requires.

- **A — Contained Systems.** A System's equipment stays in its compartment. Independently located equipment is represented by another System, with explicit service/data connections. Cross-compartment conduits retain their authored routes and damage locations; their extent does not relocate the connected equipment.
- **B — Distributed equipment.** A System has a home compartment, but its physical parts can be assigned to other compartments. They still share that System's commands and AI; damage, containment and repair access follow each part's actual location.

**Selected: A for the initial model.** A System's physical equipment stays within its compartment. Equipment in another compartment is represented by another System with explicit service/data connections. Compartment templates therefore package complete fitted Systems, while a central pump can serve other compartments through physical links. Each System retains its controlling Console, shared command channels and individual AI.

Preserve AD03's distinction between mechanisms and physical parts, AD04's single-System turret rule, routed conduits, permitted knowledge and ordinary System command admission. Cross-compartment conduits retain their authored routes and damage locations; their extent does not relocate the equipment they connect. Precise conduit-segment ownership, boundary equipment and exterior mounts still need a consistent layout contract. This selection must not erase routes or turn every conduit segment into an independently operated System.

## AD39 — Dimensionality of the physical ship layout: selected B with clarification

**Selected: B, explicitly directional damage on the 2D plane, extended by AD45 to multiple decks and AD48's compartment-only component location.** Compartments have 2D geometry on each deck. Systems/components belong to compartments without precise internal positions; services retain their connections and affected-compartment routes. Incoming damage selects a weighted random deck, then arrives from a direction on that plane and can penetrate from that direction. A 3D impact point or projectile height does not select the deck or internal damage path.

This retains S016's physical path through intervening compartments in order. Targeting engines from the front still encounters compartments along the planar approach before reaching the engine compartment. AD44 locates structural damage in compartment HP and applies status effects to components as a consequence. Progressive penetration is therefore resolved at compartment granularity, without a separate grid of holes or individual wall/part HP. AD46 selects progressively falling resistance as compartment HP falls. Incoming direction determines a path through the selected deck, not merely a facing followed by arbitrary compartment damage.

Small craft and torpedoes retain their selected 3D movement and presentation. Their hit handling must supply the appropriate planar incoming direction to this damage model. The exact conversion for turning projectiles, blast sources or near-vertical approaches remains an integration contract; do not silently substitute a firing ship's bearing when it differs from the direction damage arrives from. AD45 supersedes the earlier exclusion of multiple decks, while retaining planar penetration rather than a 3D internal trajectory.

Workshop authors each deck's compartment geometry using the component/System/compartment template units, with each System's equipment contained under AD38. Components are assigned to their compartment without finer coordinates under AD48. Actual geometry remains separate from what a Console or attacker knows. Scanning can identify which compartment contains equipment; firing-angle advice then uses that region and intervening protection. Targeting does not bypass weighted deck selection or guarantee a particular component failure.

Current [weapon arc geometry](../../../crates/phoenix-sim-gameplay/src/weapons/arc_geometry.rs) provides planar ship-relative/world-bearing conventions, and [shield facing](../../../crates/phoenix-sim-gameplay/src/weapons/shield.rs) uses bearing. Those are integration groundwork, not an implemented interior penetration model. Current [ship damage](../../../crates/phoenix-sim-gameplay/src/ship/damage.rs) uses weighted damage among HP entries, and [repair teams](../../../crates/phoenix-sim-gameplay/src/modifiers/repair_teams.rs) use configured travel duration. Both need mapping to the selected physical layout; current 3D detonation positions must not accidentally dictate the new internal damage contract.

The exact 2D shape vocabulary, penetration limits, payload deposition and repair interaction remain open. AD42–44 select compartment material, distance-based resistance and compartment HP with component status effects. AD40 selects distributed lines across the facing profile for ordinary ship targeting, governed by supported accuracy/spread rules. Direction fixes the approach orientation; the exact distribution and its integration with deck selection and existing hit/miss handling remain to be specified.

Team traversal still follows supported passages/access, and hazards follow condition/boundary rules. A penetration path does not automatically become a walkable route or spread every incident. S102's coarse atmosphere model and existing networking remain unchanged. The previous recommendation for 3D internal volumes is superseded by this user selection.

## AD40 — Penetration line for ordinary ship targeting: selected A

AD39 supplies the incoming planar direction; S016 permits aiming toward a known component while still hitting intervening material. For ordinary ship targeting without a selected component, decide which line through the ship's facing profile the hit follows. Both options trace that line in the agreed direction and apply the same material/penetration rules.

- **A — Distributed lines across the facing profile.** Normal hits can enter along different parallel lines through the ship's 2D profile, using supported weapon accuracy/spread rules. Focused subsystem targeting uses the known component's location and applicable accuracy rather than revealing or guaranteeing the actual component position.
- **B — Centre line by default.** Ordinary fire follows the incoming direction through the ship's centre. A supported known-component target moves that line toward the chosen location. Repeated ordinary hits from a stable direction tend to use the same channel.

**Selected: A, refined by AD48.** Ordinary hits use distributed penetration lines across the facing profile, governed by supported weapon accuracy/spread rules. Each line follows the incoming planar direction and encounters compartments in order. Deliberate subsystem targeting concentrates aim toward the compartment legitimately known to contain that equipment, subject to accuracy, protection and weighted deck selection. There is no exact internal component point to aim at. Sustained approach geometry and focused targeting can exploit weakened compartments; ordinary fire does not always drill a centre line.

This is a question about placing the 2D damage path, not adding a second hit/miss roll or restoring 3D impact-point damage. Exact sampling, repeatability, existing accuracy integration and hit/miss ordering remain to be designed. Use the same rules for human and AI attackers, retain limited targeting knowledge, and avoid assigning an arbitrary hidden component as the automatic target of every normal shot.

## AD41 — Separate penetration from delivered damage/effects: selected A

AD39–40 define the planar path and its placement. The generic weapon direction already includes authored damage amount/type, projectile behaviour and area effects; S012 leaves exact armour penetration unresolved. Decide whether material penetration and the payload delivered along that path are independently configurable, or derive from one common strength budget.

- **A — Separate penetration and payload.** Author penetration strength/profile separately from delivered damage and supported effects. Common material interactions determine how far the hit continues and what it deposits at each encounter. This can express a deeply penetrating shot with modest local damage or a destructive payload with little penetration.
- **B — One shared strength budget.** A single remaining impact-strength value drives both barrier penetration and damaging effects. Material interactions spend that budget as the hit travels; weapon variation comes from its initial strength, damage type and other existing parameters.

**Selected: A.** Configure penetration strength/profile separately from delivered damage and supported effects. Common material interactions determine continuation and deposition along the planar path. The generic weapon library can therefore express deep penetration with modest damage or heavy damage with little penetration through authored data. Keep one supported impact-resolution framework; Workshop configures its mechanisms under AD13. AD42 limits material authoring to the compartment level.

This does not apply the full payload to every traversed compartment or affected component. Exact attenuation, deposition, stopping, residual penetration and any delayed detonation need explicit rules so traversal does not duplicate damage/effects. AD43 selects distance-based resistance and AD44 assigns structural HP to compartments, with component status effects arising from their damage. This does not restore System percentage-damage bars. EMP refresh/resistance and other condition effects keep their own selected semantics.

Current [damage routing](../../../crates/phoenix-sim-gameplay/src/ship/damage.rs) distinguishes shield handling and leaked hull damage; its `shield_pierce` split bypasses shields. That is not the proposed resistance to penetrating armour or internal material. [Torpedo configuration](../../../crates/phoenix-sim-gameplay/src/weapons/torpedo.rs) likewise has existing shield/hull payload fields. Treat these as migration inputs, not a complete generic impact model or a reason to reuse one ambiguous pierce field for both shields and material.

All effects still require supported delivery, geometry and ordinary authority. A penetration parameter alone grants neither shield bypass nor knowledge of hidden equipment. The hit resolver may use physical truth for its simulation; attackers and their individual System AI learn results only through permitted observations. Exact units, damage-type responses, blast/beam integration and numerical balancing remain open.

## AD42 — Material properties are authored per compartment: selected user direction

**Selected:** set materials per compartment, with no more detailed material authoring. The compartment definition/template owns the material selection and associated properties used by the supported damage interactions. Systems, individual parts, bulkheads and map cells do not have separate material selections or local material overrides.

The material setting travels with a compartment template and can be varied at that compartment's authored scope. Reusing a component or System template in another compartment uses the destination compartment's material context; it does not bring a hidden finer material assignment. A reusable material-definition library can supply the properties, but exact fields, units and reference syntax remain an implementation proposal.

Material authoring and damage state are distinct. AD44 gives each compartment structural HP; damage to it applies status effects to components, which retain their condition/repair identities. Hits follow the 2D path through compartments on the selected deck. Impact-generated faults can differ between occupants. AD49 separately applies the common compartment-sourced disabling condition at zero HP, with recovery under AD50. There is no finer structural HP grid or separately authored material at damaged locations. AD46 selects progressive weakening with HP loss; the exact resistance curve and condition-selection rules remain to be specified.

Update Workshop and runtime validation around that single ownership: the compartment supplies the material context to impact processing, while the supported mechanism and current physical state determine consequences. Shared boundaries and routed service segments need an unambiguous compartment context without double-counting protection or inventing finer material overrides. AD43 charges resistance by distance along the penetration path.

## AD43 — Resistance along a path through a compartment: selected A

AD41 separates penetration from payload; AD42 authors material at compartment level only. Decide whether traversing more distance inside the same compartment costs more penetration, or whether entering that compartment incurs one fixed resistance cost.

- **A — Resistance scales with path length.** The compartment material supplies effective resistance per unit of 2D distance. A longer path through that compartment spends more of the shot's remaining penetration. This uses one material setting for the compartment, not finer material assignments.
- **B — Fixed cost per compartment.** Entering a compartment spends its authored resistance cost once, regardless of how far the line travels within it. Continue to the next compartment if enough penetration remains.

**Selected: A.** The compartment material supplies effective resistance per unit of 2D path length. A longer traversal spends more penetration. Approach angle and depth matter through the layout, and splitting an otherwise unchanged region into more compartments does not by itself multiply its resistance. This is a coarse effective resistance model, without modelling every wall layer, cable or piece of machinery.

AD44 assigns structural HP to the traversed compartments and applies component conditions from their damage. AD46 selects progressively weaker resistance as HP falls. Exact deposition, stopping inside a compartment, damage-type response and the resistance curve remain open. These rules are proposed, not current behaviour: [current hull damage](../../../crates/phoenix-sim-gameplay/src/ship/damage.rs) distributes damage among System HP entries rather than tracing compartment material.

## AD44 — Compartment HP and component status effects: selected user direction

**Selected:** compartments have HP. Damage to a compartment applies status effects to its components. Structural damage is represented at compartment granularity; components retain conditions and repair targets rather than individual numeric HP pools.

This refines AD03 and S016: a physical component can supply several reusable mechanisms, and its status effects modify those capabilities. A damaged capacitor can affect storage, intake and heat through one condition target. Ordinary hit processing traces the selected deck's compartments, resolves penetration and payload, reduces compartment HP and applies supported component effects. AD48 selects affected components from the compartment-wide eligible pool, without precise positions. Exact effect weights, condition types and severity remain open; shared compartment HP does not imply identical effects on all occupants.

Keep authored compartment HP parameters separate from current runtime HP. Component conditions remain independently observable and repairable through the selected diagnosis/team/report rules. Repairing a component does not restore compartment structure. S025 already permits resource-dependent structural restoration. AD49 selects a compartment-sourced disabling condition at zero HP, removed when the compartment is restored; it does not delete contents or repair unrelated component faults. AD50 selects recovery at a modest authored HP fraction; its exact value, work rules and ship defeat criteria remain open.

This replaces interpretations of progressive breaches that would require individual wall/part HP, local hole geometry or a fine damage grid. AD46 makes damaged compartments progressively easier to penetrate as HP falls. Workshop, authoritative state, Console observations and status indicators must agree about these owners, while keeping hidden damage out of unauthorised views. No networking or individual System AI change is implied.

## AD45 — Multiple decks and weighted hit selection: selected user direction

**Selected:** ships support multiple decks. Incoming damage hits a weighted random deck, independently of the shot's Z height. The selected deck supplies the 2D compartment layout for the hit's incoming direction and penetration path.

The physical authoring relationships are ship → decks → compartments → fitted Systems/components. Systems still have their separate controlling Console; neither a deck nor a compartment creates another controller or pooled blackboard. Each compartment retains its material and HP, with component status effects under AD44. This extends AD39's layout to several planar decks, not 3D internal ray tracing.

Weighted deck selection precedes resolution through that deck's compartments. Under AD48, knowledge of the compartment containing equipment informs planar aim but does not guarantee selecting its deck; shot height does not provide a targeting override. Exact sources/defaults for weights, eligibility when deck footprints differ, accuracy integration and repeatable random sampling remain to be specified. The rule does not yet decide whether a supported blast or propagating hazard can affect other decks.

Workshop needs deck-aware compartment placement and a clear way to inspect the selection weights. Repair/access layout also needs explicit connections between decks, preserving automatic team routing; random hit selection does not choose where teams move. This does not select a separate deck-template feature, deck HP pool, Z-sensitive damage or new movement/network architecture.

## AD46 — How compartment HP changes penetration resistance: selected A

AD43 charges material resistance per unit of path length; AD44 gives compartments HP. Decide how damage changes the effective resistance encountered by later hits, using that compartment state rather than finer local damage.

- **A — Progressive weakening.** Effective resistance falls as compartment HP falls, so repeated damage makes that compartment progressively easier to penetrate. The exact curve remains tunable.
- **B — Intact resistance until zero HP.** The compartment retains its full resistance while it has HP; its resistance drops only when HP reaches zero.

**Selected: A.** Effective resistance falls progressively as compartment HP falls. Repeated damage therefore makes that compartment easier for later shots to penetrate. Material properties, current compartment HP and distance traversed supply the resistance calculation; the exact curve remains tunable rather than implicitly selecting a linear formula.

Different paths through the same damaged compartment use its current resistance. This preserves gradual breakthrough without individual holes or weakened cells. Component status effects retain their separate owner; gradual HP loss does not prescribe identical impact faults, while zero HP applies AD49's common disabling condition. Structural repair, component destruction and ship defeat remain separate decisions. Exact residual resistance at zero HP and whether a hit's own damage affects its remaining traversal still need execution rules.

## AD47 — Triggering component status effects from compartment damage: selected A

AD44 places HP on compartments and status effects on components; AD46 now determines the direction of structural weakening. Decide whether a damage event itself generates component effects, or whether effects are principally triggered when compartment HP crosses authored thresholds. This concerns impact-generated faults; existing EMP refresh/resistance, condition escalation and other supported causes keep their own semantics.

- **A — Damage-event rules.** Each damaging event can apply supported component status effects. Authored rules can use deposited damage amount/type and component susceptibility to determine eligible effects, chance and severity. A small hit need not produce a new fault, and later hits can cause new faults or affect existing ones under explicit condition rules.
- **B — Compartment HP thresholds.** Crossing authored compartment HP thresholds triggers the configured component effects. Fault generation is chiefly tied to structural deterioration milestones rather than resolved for each damaging event.

**Selected: A.** Individual damaging events can apply component status effects through authored rules using deposited damage amount/type and component susceptibility. Those rules determine eligible effects, chance and severity. Small hits need not generate faults; later hits can create new faults or affect existing conditions through the supported application rules. Crossing a compartment HP threshold is not the primary trigger for these faults.

This supports distinct weapon failure patterns while retaining compartment HP and separate component repair. Reuse AD26–27's common condition mechanism and supported data configuration. Existing EMP refresh/resistance, escalation and other condition causes retain their selected semantics. A damaged compartment does not continuously reapply a repaired impact fault merely because its HP remains low. AD49 adds a distinct compartment-sourced disabling condition at zero HP; restoring the compartment removes that condition without erasing impact faults.

Exact component selection, effect counts, severity curves, repeat application and probability scaling remain open. Continuous beams, rapid fire and damage split across simulation ticks need consistent event accounting; choosing damage events must not inadvertently make fault rates depend on update frequency. This selection guarantees neither a fault for every hit nor failure of the component an attacker intended to target. AD48 resolves how the path influences component selection within an affected compartment.

## AD48 — Spatial selection of component effects within a compartment: selected B with clarification

S016 retains scanned equipment positions and useful firing angles; AD44 makes compartments the structural HP units, and AD47 generates component effects from damage events. Decide how much the hit's path matters after reaching a compartment. Both options keep the chosen deck, intervening compartments, penetration limits and component susceptibility; neither adds component HP.

- **A — Favour components near the damage path.** Component positions influence which occupants receive effects. A direct hit favours components close to its path, while supported area effects can reach more widely. Known equipment positions can improve aim within a shared compartment, without guaranteeing failure.
- **B — Select across the whole compartment.** Once damaged, the compartment supplies a weighted pool of eligible components based on susceptibility and damage type. Exact positions within it do not change selection; targeting improves the chance of reaching the relevant compartment.

**Selected: B, with no precise component positions.** Components exist generically within a compartment. They have identity, System membership, conditions and relevant susceptibility, but no authored internal coordinates or geometric footprint. Once a compartment receives damage, its eligible component pool supplies the targets of supported effects, weighted by damage type and susceptibility under AD47. There is no additional proximity-to-path weighting within that compartment.

This refines S016 and AD03/39/40: scans identify equipment's compartment, and attack-angle advice aims through the relevant compartment region and intervening protection. Targeting does not choose a precise point on a component, bias selection toward an intended component independently of the physical damage rules, or bypass weighted deck selection. Different components in the same compartment can receive different effects through their authored susceptibility and condition rules. Exact weights and effect counts remain open.

Workshop lays out compartments on decks and assigns Systems/components to their contents; it does not require component placement within those regions. Diagrams or UI lists may arrange component icons for readability without giving those positions simulation meaning. Repair teams route to a compartment and work on the identified component; no intra-compartment coordinate or path is needed. Conduits still identify affected compartments along their routes, while weapon/sensor mount arcs and external presentation retain their separate geometry contracts.

Structural repair need not be reopened: S025 already permits resource-dependent full field restoration of armour/structure, subject to parts, teams, access and time, with G064's large/catastrophic exceptions. Apply that direction to compartment HP separately from component-condition repair. Exact recipes, work rates and limits remain open.

## AD49 — Component survival when a compartment reaches zero HP: selected user direction

AD44 separates structural HP from component conditions, and S025 already covers field structural restoration. Decide whether reaching zero compartment HP automatically destroys its contents, or whether their own conditions still decide what works.

- **A — Components retain their own condition state.** Zero structural HP does not automatically destroy all contents. Components can remain operational if their conditions and required services permit; further incoming damage can still apply status effects even when compartment HP cannot fall farther. Supported hazards and access rules determine whether work there is safe or possible.
- **B — Zero HP destroys all contents.** Reaching zero compartment HP applies destruction to every contained physical component, regardless of its previous condition. Recovery follows applicable structural/component replacement and catastrophic-loss rules.

**Selected user direction, replacing the original A/B alternatives:** at zero compartment HP, its components are disabled by a condition. That condition is removed when the compartment is restored. This is recoverable disablement caused by the compartment, not automatic destruction of every component or continued operation of otherwise healthy equipment.

Use the common condition mechanism with the compartment as the source. Component repair can address its own faults, but cannot clear the condition whose remedy is compartment restoration. Clearing that source removes only its disabling contribution: independent damage, EMP, destruction, overheating, missing supply or other supported constraints still apply. Restoring the compartment therefore makes components eligible to operate, not guaranteed to work or commanded to start. Existing System intent, authority and physical startup rules remain in force.

The status indicator should expose this cause through permitted knowledge and link the repair need to the affected compartment. A compartment restoration job uses the agreed team/material/access model under S025, while individual component repair retains its own targets. Conditions from multiple causes remain distinguishable for correct removal and reporting; this does not require another controller or transport.

AD50 selects a modest authored recovery threshold above zero and below full HP to clear the condition. Residual penetration resistance, safe access, continuing damage at zero HP and whole-ship defeat retain their separate rules. Do not silently make the compartment or its contents invulnerable while disabled, erase existing faults on recovery, or convert zero HP into automatic physical deletion.

## AD50 — Structural recovery needed to remove compartment disablement: selected A

AD49 selects disablement at zero compartment HP and removal on restoration. Clarify what counts as restored for that condition; this does not reopen full field restoration under S025 or decide the separate armour-resistance curve.

- **A — A modest authored recovery threshold.** Disable at zero HP; clear the condition once repairs reach a configured HP fraction above zero, below full restoration. This provides recovery hysteresis without requiring the entire structural repair to finish.
- **B — Any positive HP.** Clear the condition as soon as any structural HP is restored. A return to zero disables contents again.
- **C — Full structural restoration.** Clear the condition only at maximum compartment HP.

**Selected: A.** Disable the contents when compartment HP reaches zero; remove the compartment-sourced condition once structural repairs reach a modest authored HP fraction above zero and below full restoration. Further structural work can continue restoring protection. The exact fraction remains a tuning value rather than a fixed percentage selected here.

The failure and recovery thresholds provide hysteresis. Once applied, the condition remains until the recovery threshold is reached. Once cleared, losing HP below that threshold does not reapply it unless HP reaches zero again. Clearing this condition neither removes other component faults nor issues a restart command; current intent, services, startup requirements and other conditions still determine operation. No extra manual restart step is introduced.

AD21–25 already provide the shared report list and independent repair-job execution. Compartment restoration and individual component remedies fit that framework as distinct targets, without a new repair controller or parent/child job lifecycle. AD51 permits parallel structural and component work when its actual prerequisites are satisfied; the compartment's disabling condition alone does not block repair.

## AD51 — Component repair during structural restoration: selected A

Consider a compartment that is safe and accessible to repair teams but whose zero-HP disabling condition is still active. Decide whether that condition also blocks component repair until structural recovery, or only blocks equipment operation. This does not waive hazards, missing parts, required tools or other real job prerequisites.

- **A — Permit parallel work.** A team can repair a component while another restores compartment structure, when each job's prerequisites are met. Repaired equipment remains disabled until the compartment reaches its recovery threshold. Keep S075's one team per component and the shared independent-job framework.
- **B — Structure first.** Component repair cannot begin while the compartment-sourced disabling condition is active, even if access and other prerequisites are satisfied. Reach the recovery threshold before starting component work.

**Selected: A.** Component repairs can proceed while another team restores compartment structure, provided the compartment is safe and accessible and each job has its actual parts, tools and other requirements. The compartment-sourced disabling condition blocks operation, not repair work by itself. A repaired component still waits for that condition to clear at AD50's recovery threshold before it can operate, subject to its remaining conditions and normal requirements.

Use independent jobs and the same report-driven Repair work list under AD21–25. Structural and component jobs retain distinct targets, progress and material commitments. A component completion report must preserve the known compartment-related limitation; structural recovery must not imply that unrelated component faults were repaired. The effective Repair controller or human chooses allocation from permitted knowledge, with ordinary AI supporting delegated complexity.

Keep S075's one team per component and ordinary automatic routing. No universal structure-first gate is introduced. This does not select structural-team stacking, automatically dispatch extra teams, waive unsafe conditions or add a shared omniscient controller. Exact rates and resource recipes remain tuning/content contracts.

## AD52 — Physical ship destruction versus recoverable disablement: selected B with clarification

AD49–51 allow disabled compartments and equipment to recover through real work. G064 already separates disabled vessels from actual ship loss, and G012 retains survivor/team participation. Decide whether severe depletion of compartment HP can itself destroy the ship, or whether it only disables the vessel unless an explicit catastrophic event occurs. Mission failure, surrender and inability to fight are separate from physical destruction.

- **A — Structural depletion can destroy the ship.** Derive overall structural condition from remaining compartment HP compared with its total maximum. Falling below an authored destruction threshold causes physical ship loss. Supported catastrophic causes can also destroy a ship. No independent ship HP pool is added.
- **B — Catastrophic events only.** Compartment HP depletion alone leaves a disabled vessel, even when all compartments reach zero. Physical destruction requires a supported catastrophic event, such as a reactor explosion or completed self-destruct procedure.

**Selected: B, with a clear operational end to combat before physical destruction.** Compartment HP depletion alone does not destroy a ship, even when all compartments reach zero. Physical destruction needs a supported catastrophic cause such as a reactor explosion or completed self-destruct procedure. The user separately requires a clear point at which AI can regard an opponent as out of action: crew may abandon ship before catastrophic loss, and losing engines **and** power can establish disablement.

| Distinction | Meaning and consequences |
|---|---|
| Physically destroyed | A supported catastrophic event has caused actual ship loss. Exact debris/salvage and personnel consequences follow their own rules. No aggregate compartment-HP destruction threshold is selected. |
| Disabled | The vessel has lost usable propulsion and power. It can remain physically present and potentially recover through teams, supplies, towing and repairs. Capability loss is distinct from remaining HP. |
| Abandoned | Crew evacuation can occur before the ship is disabled or destroyed. Physical evacuation uses the ordinary personnel/craft/work mechanisms; this is not merely an empty human seat or a client disconnect. |
| Assessed out of action | An observer has enough permitted evidence to treat the ship as disabled or abandoned for its current objective. This is a usable decision input for individual System AI and the corresponding human Console, not access to the target's hidden world state. |

Keep physical truth, delivered evidence and AI decisions separate. Existing observation/report records and Console knowledge processing (AD34–35) can express disablement or abandonment assessments, their evidence and uncertainty. Observed loss of capability, visible evacuation and normal Comms can inform them; a target's private component records cannot be read as an omniscient death flag. No universal wait for an explosion or demand to drain every compartment is introduced.

G039/G044 already make reactions depend on objectives, doctrine and available knowledge. Individual System state trees can use the assessment to stop treating the vessel as an active combat target, reprioritise or support recovery, with ordinary communication and hysteresis. Keep actual hostile activity and later observed recovery relevant: a disabled/abandoned label does not physically prevent weapons firing, confer invulnerability or compel every actor to respond identically. Objective success, surrender and physical destruction remain distinct predicates.

The capability test needs authored roles rather than hard-coded engine System IDs. An intentionally engineless turret or facility has not suffered propulsion loss merely because none is fitted. Existing local batteries also mean reactor failure alone is not proof of complete power loss; exact usable-power criteria, observation confidence and persistence remain to be specified. These details must preserve the user's engines-and-power criterion for ordinary propelled ships, not silently change it to either loss alone.

Early abandonment is a supported crew decision with actual evacuation consequences. AD53 selects non-hostile standby orders for its normal procedure; AD54 requires human initiation when humans are aboard, with ordinary AI execution of authorised delegated steps. Fully AI-crewed ships use ordinary AI policies. Exact policy triggers and evacuation execution details still need definition. Preserve individual AI, ordinary command admission, limited knowledge and G012's escape-pod/team participation after loss. No central ship brain or guaranteed safe escape is selected here.

Current [SystemHull::is_destroyed](../../../crates/phoenix-sim-gameplay/src/ship/damage.rs) returns true only when its non-empty collection of System HP entries is entirely at zero. The proposed model supersedes that rule: zero compartment HP produces disablement conditions, not automatic physical destruction. Migration must distinguish destruction from operational assessments in targeting, objectives, reports and entity lifetime; it must not globally substitute one new `dead` boolean for all those meanings. Networking remains unchanged.

## AD53 — Default System orders during abandonment: selected A

AD52 permits early abandonment while some equipment still works. Decide the default abandonment procedure's orders for remaining Systems, using ordinary command channels and individual AI rather than changing behaviour simply because a player seat is vacant.

- **A — Order non-hostile standby.** The normal abandonment procedure issues cease-fire/standby orders through the relevant Systems. Keep supported evacuation, life support and distress functions operating where possible. Actual machinery follows ordinary authority, physical constraints and transition timing.
- **B — Continue existing orders.** Evacuation changes personnel state but normally leaves System orders unchanged. Capable automated weapons and movement can continue their existing tasks after the crew leaves.

**Selected: A.** The normal abandonment procedure issues cease-fire/standby orders through the relevant Systems, retaining supported evacuation, life support and distress functions where possible. Use ordinary command channels and individual System AI; each operation still requires authority and follows physical constraints and transition timing. Human-held Systems retain their normal control authority. No special temporary-Backfill behaviour or central ship controller is introduced.

This supplies a legible end to active fighting and an opportunity for rescue. It does not instantly shut down machinery, erase damage or already-fired projectiles, trigger self-destruct, force surrender or guarantee that an abandoned ship is safe. Other actors still assess available evidence and respond through their own objectives/doctrine under AD52. AD54 selects initiation authority; supported authored exceptions remain to be defined.

## AD54 — Initiating abandonment with humans aboard: selected A

AD53 defines the default orders after abandonment begins. Decide whether delegated AI can initiate this consequential crew procedure while humans are present. S030 already requires human initiation for self-destruct, with ordinary AI carrying out authorised steps on AI-controlled Systems; that decision does not automatically settle abandonment.

- **A — Human initiation when humans are present.** AI may report danger and recommend abandonment, but a human starts the procedure. Ordinary individual AI then executes authorised steps for delegated Systems. Ships without human players can initiate through their ordinary AI policy using permitted knowledge and objectives.
- **B — Permit delegated AI initiation.** An appropriately delegated System AI may also start abandonment on a human-crewed ship under authored emergency criteria, announcing the decision. Its authority still does not extend to taking over human-held Systems.

**Selected: A.** When human players are aboard, a human starts abandonment. AI may report danger and recommend evacuation, then execute authorised steps for AI-controlled Systems through ordinary command channels. It does not initiate abandonment for the human crew or take over human-held Systems. Ships without human players may initiate through their ordinary individual System AI policies, using permitted knowledge and objectives.

The controlling System/Console remains data-driven. This adds no vote, mandatory captain approval, central ship controller or special Backfill behaviour. AD53 supplies the normal standby orders; actual evacuation still uses available personnel, craft and work mechanisms. Exact AI triggers, execution steps and cancellation rules remain content/contracts to define. Physical evacuation remains distinct from a player leaving a seat or disconnecting.

## AD55 — Returning an abandoned ship to service during a scenario: selected A

AD52 keeps physical destruction separate from abandonment. G064 permits field restoration of supported damage, G012 preserves survivor participation and S100 uses physical recovery. Decide whether an abandoned vessel can return to operation during the same scenario once crew can safely return and its capabilities permit. This concerns a ship the returning crew is authorised to operate; hostile capture and ownership transfer remain separate.

- **A — Allow return to service in the field.** A crew can return through supported personnel-transfer mechanisms, restore what is needed and resume operating the existing ship and its Consoles. Damage, depleted stores and other persistent state remain. Abandonment is reversible crew/operational state, not a permanent salvage-only lifecycle.
- **B — Require port recovery.** Once abandoned, the vessel can be towed or salvaged, but resuming it as an operated ship requires recovery at a suitable port. This limits in-field crew/control transitions and makes abandonment a stronger commitment.

**Selected: A.** An authorised crew can return through supported personnel-transfer mechanisms, restore needed capabilities and resume operating the abandoned ship during the same scenario. Reuse ordinary ship, team, repair and Console mechanisms. Abandonment is reversible crew/operational state and does not itself require recovery at a port.

Keep the same entity and its Systems and Consoles, with damage, conditions, depleted supplies, ongoing physical work and permitted retained knowledge intact. Returning personnel does not repair machinery, refill stores or revive equipment lost to an irreparable catastrophe. Particular repairs can still require facilities under their own rules. Normal control admission, individual System AI and resource/startup constraints govern renewed operation; reboarding does not automatically resume previous hostile orders or clear another observer's assessment.

This selection concerns a vessel the crew is authorised to operate. It does not grant enemy-ship control merely by arriving aboard. Exact transfer/access requirements and hostile capture remain separate; AD56 selects whole-ship operational takeover for a successful capture. Existing transport is unchanged.

## AD56 — Operational control after hostile capture: selected A

G070/S082 already use located teams with whole-team objectives, local encounters and automatic routing. G042's behavioural surrender does not itself transfer ownership or lock actuators. Decide whether a supported capture operation transfers ship-wide operational authority, or whether opposing sides can operate different Systems of one hull while the boarding contest continues.

- **A — Whole-ship takeover.** Completing the authored capture requirements transfers operational authority for the vessel as a whole. Before then, teams may still disable equipment, secure locations or pursue other supported local objectives, but occupying one System does not confer partial command authority.
- **B — System-by-System takeover.** Successful local capture objectives transfer authority for individual Systems. Attackers could control propulsion while defenders retain weapons, with explicit contested-control and knowledge rules for their respective Consoles.

**Selected: A for the initial architecture.** Completing supported authored capture requirements transfers operational authority for the whole vessel. Before that handover, located teams can disable equipment, secure areas and pursue other supported objectives, but occupying one System does not grant partial command authority. Boarding remains a spatial team activity without opposing sides simultaneously operating different Systems of the hull through capture.

Retain the existing ship, its physical state, System/Console topology and individual controller structure. Authority transfer is a gameplay admission change, not a central ship AI or a replacement network architecture. Former operators no longer gain command authority from their former tenure; exact admission, reassignment and publication contracts must make that handover coherent. Damage and other physical consequences persist.

G042's behavioural surrender alone still does not transfer control; actual supported capture requirements must be met. Exact requirements, remaining defenders, post-capture orders and displaced-player participation still need contracts. G012's useful participation after loss remains a constraint. AD57 permits access to retained, already-readable Console intelligence; control transfer itself grants no hidden world knowledge.

## AD57 — Access to intelligence stored on captured Consoles: selected A

AD12 preserves Console knowledge and working views through ordinary operator handoff; AD55 preserves the same ship when its authorised crew returns. AD56 now permits hostile whole-ship takeover. Decide whether newly authorised operators can immediately read the captured Consoles' retained sensor observations, decoded Comms and reports, or need a separate recovery job for historical intelligence.

- **A — Inherit readable local records.** Captured Consoles expose their retained, already-readable records to the new operators, keeping source, age and uncertainty. Immediate operating information and stored intelligence use the same Console knowledge model; there is no universal extraction step.
- **B — Recover historical intelligence separately.** Normal operating information becomes available after takeover, but the previous crew's historical intelligence requires supported recovery/analysis work before it is readable. Records remain stored rather than being automatically erased.

**Selected: A.** Newly authorised operators can immediately read the captured Consoles' retained, already-readable sensor observations, decoded Comms and reports. Preserve each record's source, observation time, receipt history where relevant and uncertainty. Use the same Console knowledge model without a universal extraction or analysis job before access. Capture grants access to the records that still exist; it does not recreate lost history or make old observations current.

Each Console retains its own knowledge scope. Its newly authorised human operator and individual System AIs read the same permitted records under AD02; copying or forwarding follows ordinary information-sharing rules and preserves evidence lineage. Private AI memory, other ships' private knowledge and encrypted material not yet decoded do not become readable Console facts through capture. Normal AI reading time and equipment/access requirements still apply where relevant; immediate record access is not instantaneous AI comprehension.

Ongoing feeds remain subject to their normal faction, sharing, communication and authorisation rules. Taking a vessel does not automatically grant continuing access to its former faction's live intelligence or merge captured records into every friendly Console. Exact sharing, revocation, bounded retention and any separately supported protected-record mechanics remain contracts to specify. No new transport protocol or mandatory intelligence-recovery System is introduced.

## AD58 — Internal routes for teams and deliveries: selected A

G070/S082 already select located team work with automatic routing; S043 requires reachable internal delivery paths and allows ordinary AI to choose supported alternatives around blocked or hazardous routes. AD37–48 place Systems in compartments across multiple decks, without precise internal component coordinates. Decide whether internal movement uses coarse compartment connections or detailed geometric paths within those spaces.

- **A — Authored compartment connections.** Compartments are route nodes, with explicit passage links and inter-deck connections. Supported access, hazard and travel-cost rules determine which routes each activity can use. Teams and deliveries route automatically over this shared topology without tracing positions within rooms.
- **B — Geometric interior routes.** Define walkable areas, corridors and inter-deck transitions in the layout, and automatically calculate movement through that geometry. Players still assign whole-team objectives; the extra detail is in authoring and simulation, not manual steering.

**Selected: A.** Compartments are route nodes connected by authored passages, including explicit inter-deck connections. Teams and internal deliveries automatically choose permitted routes over this topology, without calculating detailed movement inside rooms. Players assign destinations, work and priorities rather than routine path clicks. Workshop shows and edits the connections alongside the compartment layouts.

Share topology and route-query mechanisms, while each activity supplies its own access permissions, safety limits, travel costs, speed and any supported capacity constraints. This is no new routing controller: ordinary individual System AI or human commands choose the work, and independent jobs execute it. Current known conditions inform planning; authoritative execution enforces actual access. An unobserved blockage must be encountered/reported rather than silently supplying the controller with hidden information. Existing observation/report and Console knowledge mechanisms carry route updates.

A movement link does not automatically become a power/coolant conduit or an atmosphere connection. Service links retain AD06/AD28's explicit bindings, and each mechanism uses its appropriate relationships. Damage penetration continues through compartment geometry under AD39–48, independently of internal movement routes. This selection does not require individual porter simulation, precise component positions or a walkable interior. Exact passage equipment, costs, hazards and rerouting/interruption behaviour remain contracts to define; AD59 selects aggregate batches for supplies in transit.

## AD59 — Representing internal supplies in transit: selected A

S043 and AD58 require timed deliveries over actual compartment routes, with ordinary automation and no per-item or individual-porter simulation. AD23 separates reservation from consumption and physical delivery. Decide whether a transfer has an aggregate cargo batch in transit, or reserves stock at its origin until a route-dependent timer completes.

- **A — Aggregate batches in transit.** Dispatch removes the batch quantity from its source store into an in-transit record. Its route stage and progress are tracked; arrival deposits it at the destination. Interruptions retain the batch's actual location/state for supported waiting, rerouting or recovery. Actual hazards can affect it under their own rules.
- **B — Reserved stock until arrival.** Supplies stay reserved in the origin store while a route-dependent delivery timer runs. Completion moves the quantity directly to the destination. Blockages delay or interrupt the transfer, without separately located cargo in transit.

**Selected: A.** Internal deliveries use aggregate batches with a resource quantity, source, destination and route stage/progress. Dispatch moves the quantity from its source store into the in-transit record; arrival moves it into the destination store. The same quantity must never remain spendable in the source while also being counted in transit or at its destination. Reservation remains a claim on stock, not another copy of it.

| Delivery event | Stock consequence |
|---|---|
| Dispatch quantity q | Source decreases by q; in-transit quantity increases by q. |
| Arrival quantity q | In-transit quantity decreases by q; destination increases by q. |
| Delay, interruption or cancellation | Preserve the batch's actual location and remaining quantity. A job-state change alone neither refunds it to the source nor deletes it. |
| Supported recovery, diversion or physical loss | Move or alter the actual batch through the applicable mechanism; report that outcome. No automatic loss chance or teleporting refund is implied. |

Reuse shared resource records, AD22–25's independent jobs and AD58's compartment route progress. Ordinary logistics AI handles routine replenishment and supported alternatives; a human need not dispatch every batch. No rendered cargo actor, individual item or porter is required. A record's simulation identity does not require a new network protocol or replication of every internal update.

Interrupted batches can wait, reroute or be recovered as the actual routes, authority, safety and activity rules permit. Preserve source/observation timing and normal reporting so a Console or AI does not learn an unseen loss merely by reading authoritative stock state. Destination capacity is still real; AD60 reserves receiving space at dispatch and AD61 supplies general handling capacity without per-batch personnel teams. Exact batch sizing, handling rates, loss and interruption policies remain open. Power/coolant conduits retain their own flow contracts.

## AD60 — Reserving receiving space for dispatched deliveries: selected A

AD59 puts supplies physically in transit. Decide whether dispatch reserves compatible destination capacity, or whether deliveries compete for whatever space is available when they arrive. AD23 already prevents queued work from reserving all future requirements; this question concerns an actual delivery starting its travel stage.

- **A — Reserve receiving space at dispatch.** Before launching a batch, reserve enough compatible free capacity for it at the destination. Other ordinary arrivals and loading respect that claim. Queued, undispatched requests do not reserve destination space.
- **B — Allocate receiving space on arrival.** Dispatch can proceed without a capacity reservation. If the destination fills before arrival, the batch must wait, divert or return under supported logistics rules.

**Selected: A.** Reserve enough compatible receiving capacity before a batch starts travelling. Other ordinary deliveries and loading must respect that claim. Undispatched requests reserve no destination space, preserving AD23's stage-start commitment rule rather than locking capacity for an entire future queue.

Reuse shared reservation records, tied to the destination, delivery and compatible quantity/capacity. A successful dispatch must establish its receiving claim alongside the source-to-transit stock movement; a refused dispatch leaves no departed batch or dangling capacity claim. Arrival converts the claim into actual occupancy as the batch enters destination stock. The precise atomic processing contract and shared record schema remain implementation work, not a second simulation or transport path.

A reservation is a claim, not guaranteed physical capacity. Damage, isolation or changed compatibility can still prevent receipt, leaving the batch physically in transit under AD59. Redirecting, cancelling or losing a batch must reconcile its claims without moving or restoring cargo through bookkeeping alone. Exact arbitration, invalidation, waiting and recovery rules remain to be specified. This choice adds no immunity, over-capacity storage or cross-System control authority. Ordinary logistics AI continues routine execution and reports actionable exceptions.

## AD61 — What provides routine internal delivery capacity?: selected A

AD58–60 establish routes, physical batches and receiving-space claims. S043 leaves handling capacity and staffing open. Decide whether ordinary delivery throughput is a configured logistics capability, or requires assigning finite personnel teams to the batches. Both options keep routine dispatch and travel automatic.

- **A — General handling capacity.** The logistics System provides authored throughput and concurrent-work limits, subject to its resources, conditions and the route rules. Deliveries do not each require a separately assigned personnel team. Reuse shared capacity and job mechanisms to bound the work.
- **B — Explicit logistics teams.** Each delivery needs an available team, with authored carrying/work capacity, that remains committed through its actual trip and recovery. Reuse the ordinary team framework and automatic assignment/routing; track groups rather than individual porters.

**Selected: A for routine deliveries.** The logistics System provides authored throughput and concurrent-work limits, affected by its fitted requirements, resources, conditions and route constraints. Reuse shared capacity and job mechanisms to bound deliveries. Each batch requires neither a separately assigned personnel team nor an individual porter; ordinary logistics AI schedules routine replenishment under its priorities and permitted knowledge.

Keep AD58's compartment travel, AD59's in-transit stock and AD60's receiving-space reservations. Reduced handling capability can limit delivery work, but a capacity change alone does not erase or relocate an existing batch. Report actionable shortages, delays and blockages through the shared job/report mechanisms; normal automation should not require the human to assign every delivery.

Repair/security teams and G037's shared framework for any explicitly modelled workforce remain unchanged. A handling capacity is not an extra population or staff simulation. Exact rates, concurrency accounting, equipment costs, allocation/interruption policy and any separately supported specialist transport work remain contracts to define. This selection does not introduce another AI controller or networking path.

## AD62 — Stored-output blockage in shared recipes: selected A

AD29 selects common recipe definitions for batch and continuous processing, with typed inputs/outputs and separate execution modes. Decide the default when a recipe has several required stored products and one has insufficient compatible output space. For example, a salvage recipe may produce useful parts and scrap, but its scrap store is full. This is an illustrative authored recipe, not a new mandatory waste-handling loop.

- **A — Constrain the whole recipe.** Slow or pause conversion so every required stored output can be accounted for. A batch's next indivisible production step waits until its outputs fit; continuous processing is limited by its most constrained required output. Already completed work and stored products persist.
- **B — Continue with overflow disposal.** Where supported disposal is physically available, keep producing outputs that fit and discard the excess products. Report the loss and apply any authored disposal costs/consequences. If disposal is unavailable, the process must still wait.

**Selected: A.** Slow or pause the whole recipe when a required stored product cannot fit. The next indivisible batch production step waits for sufficient compatible space; continuous conversion is limited by the most constrained required output. Completed work and stored products persist through this blockage. A full store does not automatically dispose of overflow.

AD63 resolves disposal on a deliberate recipe change: discard unfinished work, retain unused inputs and completed products where compatible capacity permits, and dispose of the excess that cannot be retained.

This concerns stored recipe products, not all physical effects: pausing conversion does not freeze a hot machine, erase its existing heat or remove passive resource demands. Exact batch commitment, continuous-rate calculation, safe stopping and any supported disposal mechanisms still need contracts. Use the owning System's normal controls, AI and knowledge rules; no central production planner or universal new byproduct is selected here.

## AD63 — What is disposed of when changing recipes?: selected A with a capacity exception

This resolves the addition to AD62. A deliberate recipe switch is distinct from an output blockage, loss of power or another temporary interruption. The original options were:

- **A — Only the unfinished batch.** Discard material already consumed into unfinished work on the old recipe. Keep completed products and unused inputs, including local stock; release unused reservations normally.
- **B — Also clear the old recipe's local buffers.** Discard the unfinished batch and leftover input/output stock held locally for the old recipe. Other ship stores and already delivered products remain unaffected.

**Selected: A, unless there is no room to retain the materials.** A deliberate recipe change discards material already consumed into unfinished work. Preserve unused inputs and completed products where compatible storage capacity permits; dispose of only the excess that cannot be retained. Release unused reservations normally. Stock that still fits need not be cleared simply because the recipe changed.

Use shared resource accounting to distinguish retained stock, discarded material and released claims. Moving retained stock to another store uses ordinary physical delivery and capacity rules; recipe switching does not teleport it into the main hold. Other ship stores and already delivered products are unaffected. The GUI and ordinary System AI should receive the resulting retained/disposed quantities through normal permitted status/reporting.

AD62 still slows or pauses ordinary production at a full required output; its blockage does not trigger this recipe-switch disposal rule. Exact disposal timing, physical consequences and storage-transfer sequencing remain execution contracts. This selection does not introduce a universal cancellation/refund rule, a mandatory waste stock or another AI controller.

## AD64 — Who defines physical mechanism execution order?: selected A

The existing simulation already defines fixed phases and explicit conflicting-owner order in [sim_sets/order.rs](../../../crates/phoenix-simulation/src/sim_sets/order.rs). The redesign must decide whether Workshop exposes scheduling of composed physical mechanisms, separately from the selected top-down ordering **inside an AI state tree**. For example, composing a battery, weapon and cooling mechanism must have defined same-tick resource and thermal semantics.

- **A — Engine-defined phases and dependencies.** Code defines how resource flow, operation, effects and observation processing relate. Workshop authors choose mechanisms, connections and supported parameters; they can inspect execution dependencies but cannot reorder physical processors. Authored AI and job workflows retain their selected ordering within their own execution contexts.
- **B — Authored ordering within engine limits.** Workshop additionally allows explicit before/after dependencies between supported physical mechanisms within permitted phases. Validate cycles and forbidden dependencies. This permits unusual timing-sensitive equipment, but adds scheduling choices and validation to content authoring.

**Selected: A.** Engine code owns physical processing phases and dependencies. Workshop authors compose supported mechanisms, connections and parameters; they can inspect the resulting dependencies but cannot author processor scheduling. Equipment behaviour must not depend on incidental component insertion order or Workshop diagram placement. New physical timing semantics require a supported mechanism contract.

AD18–19 still govern authored execution inside each AI state tree, and AD24's job workflows retain their supported sequencing. Engine scheduling does not add a ship-wide controller or grant AI hidden state. Dependency ordering need not serialize independent work: data-oriented processors may batch compatible components, with safe independent work remaining eligible for parallel execution. Exact phases, dependency edges, same-tick visibility and stable resource arbitration remain to be specified against the existing simulation. Networking remains unchanged.

## AD65 — How do components compete for a shared resource?: selected A

AD64 fixes who owns scheduling; it does not decide whether that schedule should determine who consumes a scarce resource first. Consider two weapon capabilities sharing one System's battery, with insufficient energy for both requested shots. Their common command channels and individual System controller remain as selected; this question concerns physical accounting after intent is admitted.

The existing [power-network Plan](../../gdd/mechanics/power-resource-network.md) already describes demand and delivered supply. This question extends that pattern to composed mechanisms and shared stores; it does not adopt that Plan's older battery-exhaustion gameplay over the redesign's selected flow-cut rules.

- **A — Request, resolve, then apply.** Components declare resource requirements. Shared accounting resolves what each can actually use within supply, capacity and the selected allocation rules, then operations consume only their granted amounts. Resolve at the relevant store/network boundary, rather than giving every component direct competing withdrawals.
- **B — Consume in fixed processor order.** Each operation checks and consumes available stock when its engine-defined turn executes. Later operations receive whatever remains. The order is deterministic, but precedence becomes part of the mechanism's resource behaviour.

**Selected: A — request, resolve, then consume/apply.** Composed mechanisms declare requirements; accounting at the relevant shared store or network resolves usable amounts under actual supply, capacity, reservations and the selected allocation rules. Operations consume only granted resources. Requests, grants and actual consumption remain distinct, so denied or constrained work has an explicit result and competing components cannot spend the same stock twice. Resource precedence must be defined by allocation policy rather than incidental processor order.

Keep this physical accounting separate from individual System decision-making. The human or ordinary System AI chooses intent through normal authority; the resolver does not select targets, set another System's orders or share hidden state. Resource-specific processors may share the request/result vocabulary without becoming one universal solver. The selected power rationing and flow cuts, local battery and heat rules, recipe output limits and job reservations remain constraints.

AD64 owns deterministic scheduling of the accounting stages. Exact arbitration, same-tick production visibility, grant lifetime, commit/revalidation and handling of coupled input requirements still need contracts. AD66 selects all-or-nothing commitment of the next indivisible step's required set, with no new partial claims retained on a failed attempt. Runtime observations of the outcome use ordinary permitted publication; internal physical accounting is not a Console knowledge shortcut.

## AD66 — Can incomplete work hold partial resource reservations?: selected A

AD23 reserves for the next executable stage, and AD65 resolves requirements before consumption. Decide whether a step that cannot obtain all its required inputs may nevertheless hold some of them while waiting. For example, a fabrication step needs parts and energy, but only the parts are currently available. This is reservation policy for that step, not whether machinery may have real local stores.

- **A — Commit the required set together.** Grant the next indivisible work step only when its required resources and capacities can be secured together. An unsuccessful attempt retains no new partial claims, leaving those resources available to other work. Explicit preparation or delivery stages can still make their own valid commitments under AD23.
- **B — Hold partial claims while waiting.** Let a step reserve available inputs and keep those claims until its other requirements arrive or its controller cancels/releases them. This can protect preparation for important work, but blocked steps can withhold useful stock from work that could otherwise proceed.

**Selected: A.** Secure the required resources and capacities for the next indivisible work step together. If that set cannot be obtained, the attempt leaves no new partial reservations or consumption, keeping otherwise uncommitted resources available to other work. This is a commitment boundary for a work step, not a reservation for every future stage of a job.

Explicit preparation or delivery stages can still make their own valid commitments under AD23. Already loaded ammunition, charged local batteries and physically delivered supplies persist as real state; a failed work-step attempt neither refunds nor empties them. Existing valid reservations from other stages or jobs are not released by this failure. Passive demands and continuous flow keep their selected rules.

The shared accounting must represent the coupled requirements and accept or refuse the set consistently, while AD64 retains engine-owned processing order. Exact arbitration, revalidation, claim lifetime and scalable continuous-operation accounting remain execution contracts. This choice does not introduce a universal cross-resource solver or guarantee that committed work cannot later be interrupted by physical conditions.

## AD67 — Does changing priority interrupt active work?: selected A

AD66 governs new commitments; now distinguish choosing the next work from stopping work already committed. The [current repair-team implementation](../../../crates/phoenix-sim-gameplay/src/modifiers/repair_teams.rs) pins a priority System as the next job at hand-off. It provides a comparison for the shared job framework, not a universal rule already selected for all redesigned activities.

- **A — Priority changes what starts next.** Reorder waiting work and subsequent uncommitted stages. The active committed step continues unless a separate supported stop, cancel or retask instruction is issued, or physical conditions interrupt it. The human or ordinary individual System AI can issue that instruction through normal authority.
- **B — Priority also requests interruption.** Raising an eligible waiting job above active work normally causes the owning controller to request a supported interruption of the lower-priority work. Start the higher-priority job only once the needed resources or equipment are actually available.

**Selected: A.** Priority changes reorder waiting work and subsequent uncommitted stages. They do not, by themselves, interrupt an active committed step. The human or ordinary individual System AI can separately issue a supported stop, cancel or retask instruction through normal authority; physical conditions may also interrupt work. Finishing one step does not reserve priority for every later stage of its job.

Execution still respects activity-specific stopping, recovery and resource rules. A priority change or stop instruction does not instantly free a deployed team, relocate in-transit cargo, refund consumption or take another System's commitments. AI can respond immediately in its decision tree under AD17 while the machinery or team takes time to stop. Continuous power/coolant allocation keeps its selected behaviour. Precise interruption outcomes and arbitration among ready steps remain implementation contracts.

## AD68 — A common event catalogue for reusable reactions?: selected A

AD27 selects condition reactions, AD24/30 give jobs and scenarios separate state-tree contexts, and AD33 selects declared situation events/outcomes. Decide whether supported physical occurrences also have a common typed catalogue those contexts can bind to, rather than separate vocabularies joined by bespoke adapters. Examples include a component becoming disabled, a delivery arriving or a compartment becoming unsafe.

The current [WorldEvent vocabulary](../../../crates/phoenix-sim-world/src/world/content.rs) already exposes occurrences such as attacks, destruction and flag changes to world triggers. It is groundwork, not proof of a unified cross-domain event catalogue or permission to expose physical truth to System AI.

- **A — Shared typed event catalogue, with scoped bindings.** A mechanism publishes a supported occurrence once at its logical boundary. Eligible job, condition and scenario consumers bind to the same contract and use their own supported reactions. Common metadata can identify occurrence, source/subject and simulation time, while payloads remain specific to the event type.
- **B — Separate context-specific catalogues.** Jobs, conditions and scenarios each expose their own supported event vocabulary. Explicit code adapters translate the relevant domain occurrences into each context. This keeps interfaces tailored but needs more integration work when a new occurrence serves several contexts.

**Selected: A.** Supported physical occurrences use a shared typed event catalogue. Each owning mechanism publishes the occurrence at its logical boundary; eligible job, condition and scenario consumers bind to that contract and react using their own supported operations. Reuse common metadata alongside event-specific payloads rather than separately redefining the same occurrence for each context. Exact record fields remain to be specified.

Workshop exposes supported event types, typed selectors, valid filters and scoped bindings. New event vocabulary or reaction operations require code under AD13. A subscription conveys neither command authority nor ownership of the publishing System. Reactions still pass through their normal execution/authority rules, and AD64 owns deterministic scheduling.

Physical events remain distinct from AD34's evidence records. System AI and GUI receive permitted observations/reports, not unrestricted physical event subscriptions. A common catalogue does not require a single global queue, a network message per local event or event-driven replacement of all simulation processing. Durable state and AD33 outcomes remain available independently of transient notifications. Identity, ordering, delivery, retention, duplicate handling and bounded reaction chains remain execution contracts.

## AD69 — Shared spatial fields for environments and equipment?: selected A

G029–30 already select useful environmental hazards and bounded causal evolution of parametric fields. S018 already gives directional jammers shared arc/range/turret behaviour. This question concerns their reusable spatial machinery, not whether those gameplay features should exist.

Current [RegionShape](../../../crates/phoenix-sim-gameplay/src/regions/shape.rs) and [RegionEffectKind](../../../crates/phoenix-sim-gameplay/src/regions/effects.rs) separate spatial containment from damage, dampening, jamming and other effects. Extend this foundation without treating it as an already implemented generic falloff/emitter framework.

- **A — Shared field geometry and sampling.** Natural regions and suitable equipment emissions compose supported shapes, source transforms and falloff/sampling mechanisms. Separate effect processors turn the sampled influence into interference, damage or other supported consequences. A directional emitter still uses its real mount/arc/traversal and resource limits.
- **B — Separate domain field models.** Environmental regions, jammers and other emitters keep separate representations and sampling pipelines, sharing lower-level geometry helpers where useful. Each domain can specialise its model, at the cost of more authoring and integration paths.

**Selected: A.** Environmental fields and suitable equipment emissions share supported geometry, source transforms and falloff/sampling mechanisms. Separate effect processors interpret the sampled influence as interference, damage or other supported consequences. Equipment emissions remain constrained by their real source: a directional jammer follows its mount, arc, traversal, enabled state, resource delivery and condition. Shared coverage does not require identical effects.

Build on the existing Regions foundation and G030's analytic/parametric field direction rather than introducing a second environmental simulation. A shared sampling contract does not require every emission to spawn a Region entity, one universal scalar strength or one effect-stacking/occlusion rule. Stationary and changing natural fields remain possible; moving equipment sources retain their ordinary transforms. This does not restore orbital motion for the static bodies selected under G031.

Source entities and Systems retain normal ownership and control. Physical effect processing may sample actual fields; human and AI decisions use permitted sensor observations and reports, including the selected uncertainty and forecast limits. Workshop composes supported geometry, source bindings, curves and effect parameters under AD13. Exact shape vocabulary, dimensions, overlap rules, sampling cost and module boundaries remain to be designed; AD70 addresses signal-path sampling.

## AD70 — Can fields affect signals passing between ships?: selected A

AD69 selects shared coverage machinery; now distinguish local exposure from a signal crossing a field. For example, two ships sit outside an interfering cloud on opposite sides. Should the cloud affect their sensing or Comms link because the signal passes through it?

- **A — Support path attenuation for applicable effects.** Authored signal-attenuating fields can weaken sensing or Comms according to the field crossed along the relevant path, as well as any supported local endpoint effects. Other effects still use their appropriate sampling rule: radiation exposure may be local, and a jammer may interfere at a receiver.
- **B — Endpoint exposure only initially.** Field effects on sensing/Comms depend on exposure at the observing, transmitting or receiving entities. A cloud crossed only between two unexposed endpoints does not affect their link. Defer path attenuation.

**Selected: A.** Applicable authored fields attenuate sensing or in-world Comms along the relevant signal path, even when both endpoints are outside the field. Retain supported local endpoint effects as well. A cloud between two ships can therefore reduce contact quality or link quality without either ship entering it.

Use AD69's shared geometry/sampling foundation with an effect-specific path query. The effect defines how sampled influence changes the signal; crossing a field is not automatically a binary block. Radiation exposure can remain local, and a jammer can interfere at a receiver without becoming an opaque wall across every signal path. AD71 additionally selects authored signal occlusion by substantial solid terrain; it remains distinct from field attenuation.

Mechanisms apply the same physical rules for human- and AI-operated equipment. Operators receive permitted observations of degraded sensing or communication, retaining uncertainty and history rather than being told hidden field parameters or exact unseen contact positions. The path is a gameplay signal path, not the network transport route; existing networking and player connection remain intact. Exact integration, attenuation curves, endpoint/path combination, channel applicability, thresholds and sampling frequency remain contracts to specify.

## AD71 — Should substantial solid terrain also obstruct signals?: selected A

AD70 covers interference from fields. G028/G031 leave supported solid-terrain occlusion unresolved, and G105 allows geometry to affect equipment-specific engagement support. Decide whether substantial terrain should also obstruct sensor and in-world Comms paths in the initial redesign.

- **A — Include solid-terrain occlusion.** Give planets, moons and suitable large asteroids authored signal properties. Their physical bounds can obstruct an applicable signal path where the material is opaque or attenuating. Shared path queries serve sensing and Comms, with signal-specific responses.
- **B — Fields only initially.** Implement AD70 field attenuation but defer solid-terrain signal occlusion. Solid terrain keeps its other collision/navigation/damage rules, without automatically blocking sensing or Comms.

**Selected: A.** Planets, moons and suitable large asteroids can obstruct or attenuate sensor and in-world Comms paths according to authored physical bounds and signal properties. Reuse the path-query foundation with signal-specific responses, rather than deriving cover from rendered mesh detail. This gives the selected static terrain a role in concealment and approach planning without restoring orbital motion.

Apply the same physical rules to human- and AI-operated equipment. Console observations report only supported evidence of signal loss or degradation; they do not automatically identify an unknown obstruction or reveal hidden contacts behind it. Existing history remains evidence, not a live view through the obstruction. S101's already-selected relay equipment may route permitted communication around blocked paths through actual viable links; ordinary routing remains automated and does not grant direct sensor visibility through terrain.

This selection concerns substantial terrain, not automatic occlusion by every ship, craft or decorative fragment. Exact eligible bodies, physical geometry, channel/material rules, grazing paths and sampling costs remain contracts to define. Projectile collision behaviour and internal compartment damage retain their own rules. Signal obstruction is gameplay state, not a change to Phoenix networking or a player disconnect.

## AD72 — How long should undelivered remote orders remain valid?: selected A

S101 already supports automatic routing over equipment-backed gameplay relays. AD70–71 make field interference and terrain relevant to those links. Decide how queued, structured remote-craft orders behave when communication returns after a delay. This concerns supported machine-readable orders, not free-text chat or an instruction to change network reliability.

- **A — Explicit lifetime and supersession rules.** Time-sensitive orders can expire; newer intent can replace older pending intent for the same supported command scope. Durable mission orders can be explicitly persistent. Apply these policies only where the relevant order/version is known, with normal authority and feasibility checks on receipt.
- **B — Persist until delivered or explicitly cancelled.** Pending orders remain eligible for delivery regardless of age. Receivers still validate authority and feasibility; stopping an old but still feasible order requires an explicit cancellation or replacement reaching the relevant holder.

**Selected: A.** Supported structured remote orders carry authored validity and supersession rules. Time-sensitive orders can expire while undelivered; newer intent can replace older pending intent within the same supported scope. Durable mission orders can be explicitly persistent. Defaults belong to supported command definitions, so routine use does not require players to configure a timeout for each message.

A sender, relay or recipient applies replacement only where the newer order is known. An undelivered replacement or cancellation cannot instantly update a remote craft's knowledge or undo completed work. Receivers check validity, authority and feasibility before applying an order. Exact versioning and duplicate handling remain contracts to design; message arrival order alone must not make known superseded intent current again.

Expiry of an undelivered order does not stop an already accepted onboard mission. Existing orders and ordinary individual System AI continue under the selected link-loss rules until normal received instructions or supported local policies change them. Reports and chat history retain separate delivery/retention rules; this does not turn free text into actuator commands or erase historical evidence. Exact command classes, lifetimes, replacement keys, delivery feedback and persistence remain open. This is gameplay communication over unchanged Phoenix networking.

## AD73 — Should remote orders return automatic receipts?: selected A

AD72 defines which undelivered orders remain valid. Decide how the sender learns that a structured remote order reached its recipient and whether the recipient accepted it. Shared status/report records can support this without another controller or a manual acknowledgement ritual.

- **A — Automatic delivery and decision receipts.** Supported recipient processing returns separate receipt and accepted/refused outcomes over viable in-world links. Delivery does not mean read or accepted; acceptance does not mean the action finished. Missing return communication leaves the remote outcome unconfirmed at the sender.
- **B — Ordinary reports only.** Do not add dedicated receipt states. The sender learns through the recipient's normal Comms/progress reports or independent observations, with silence remaining inconclusive.

**Selected: A.** Supported recipient processing automatically produces delivery and accepted/refused receipts for structured remote orders. Keep sending/awaiting confirmation, delivery, interpretation, acceptance and completion distinct. Delivery does not claim that a human or AI has read or accepted an order; acceptance does not claim physical completion. Required interpretation, authority, feasibility and reading-time rules still apply.

Return receipts through viable in-world communication paths, subject to normal equipment, security and knowledge limits. A sender lacking the return receipt has an unconfirmed remote outcome even if the game host internally knows what occurred. A missing receipt or sender-side timeout does not prove that the order was never received or acted on. Local queue/expiry facts can be shown as local facts without presenting them as remote acknowledgements.

Reuse shared status/report records and ordinary Console projections. This adds no manual acknowledgement chore, automatic acceptance on behalf of a human, or extra AI controller. Completion and physical success remain separate reports or observations, with the usual source/age/evidence rules. Exact receipt fields, retry/deduplication, history, authenticity and UI remain to be specified. Phoenix networking and its transport receipts remain distinct from these gameplay receipts.

## AD74 — Message-capacity tracking: rejected; no capacity model selected

**Selected user direction: do not track message capacity.** Both offered throughput-budget alternatives are rejected. Conversations, sensor feeds, structured orders and receipts do not consume a simulated message-capacity pool or separate traffic-class budgets. Do not add volume-based congestion, message quotas or a priority-management activity for rationing bandwidth.

Communication still depends on fitted equipment, power/operating mode, range, interference, terrain occlusion, compatible channels, security and permitted knowledge. S101 retains equipment-supported relays and automatic viable routing. Its earlier open references to bandwidth/capacity are superseded where they meant message throughput. Equipment power use is not a reason to introduce a message-volume accounting model.

AD72's pending orders during link loss, expiry and supersession remain; AD73's delivery/decision receipts still require a viable return path. Pending during an outage is different from waiting because other traffic consumed a quota. AI interpretation/reading time, decoding work and S066's finite listening/search coverage retain their separate selected meanings; none is a message-capacity pool. Existing networking and implementation-level buffering remain technical mechanisms, not player-managed resources.

## AD75 — One physical boundary state for access and containment?: selected A

AD58 uses compartment connections for automatic team and delivery routing. S029/S102 select sealing, venting and coarse compartment safety, without numerical pressure equalisation. Decide how a real door or hatch shared by these mechanisms is represented in Workshop and runtime data.

- **A — Shared boundary state with typed uses.** Define the physical boundary once and explicitly bind movement/access and applicable atmosphere/hazard rules to its supported state. Closing a sealed hatch can block traversal and contain a supported hazard; each mechanism still interprets its own properties and permissions.
- **B — Separate links with explicit coordination.** Movement edges and containment links keep separate state records. Authored bindings or supported reactions coordinate them when they represent the same door/hatch. This permits independent structures but adds more consistency work for authors.

**Selected: A.** Author one physical boundary state for a door or hatch. Movement/access and applicable atmosphere/hazard mechanisms explicitly bind to that state, each applying its own supported properties, permissions and consequences. A seal can therefore affect both team access and containment without separately maintained copies of the door state. Workshop exposes the boundary and its typed uses alongside the compartment connections.

Keep commanded intent separate from the actual boundary state: a requested closure is not a completed seal while equipment is still moving or unable to comply. Physical processors use the actual state; humans and individual System AI receive permitted observations through the normal Console knowledge rules. This adds no boundary-level AI or control shortcut.

Not every passage carries atmosphere, and power/coolant conduits retain their own explicit relationships. Different hazards retain different propagation rules; S102 remains a coarse condition/reserve model without pressure equalisation. A normally closed but operable door need not prevent routine authorised passage. AD76 requires automatic routing to respect a deliberate containment seal and request an authorised access change. Exact boundary ownership, door equipment, damage/repair bindings and inter-deck authoring remain to be specified under the existing System/compartment rules.


## AD76 — May automatic routing reopen a deliberate seal?: selected A

AD58 selects automatic team/delivery routing, and AD75 lets access and containment share physical boundary state. S029 makes sealing a response with consequences for access and safety. Resolve whether a team or delivery needing passage is sufficient reason to reopen a boundary deliberately sealed for containment or security. Ordinary closed doors that permit routine passage are outside this question.

- **A — Respect the seal; request an access change.** Route around it or wait and report the blocked work. The boundary's responsible human or ordinary System AI decides whether to change the sealing order through normal command authority. A movement job cannot itself override the seal.
- **B — Permit automatic temporary passage under authored safety rules.** A passage request may cause the responsible controller to reopen and reseal the boundary automatically when its permitted knowledge and authored policy allow it, without a separate decision to lift the sealing order.

**Selected: A.** Automatic routing respects a deliberate seal: reroute or wait and report the blockage. The responsible human or ordinary System AI decides whether to change the sealing order through normal authority. Ordinary permitted doors can still open for routine travel. Further boundary behaviour and implementation details are deferred under the current feature-reconciliation scope.

## Mission planning increments 22–24

User-approved revision, 14 October 2026; future design only. This elaborates AD30–33
and the bounded Director while preserving their ownership and execution contracts.
It replaces the earlier workflow-only interpretation of increment 22 and makes
increment 24's generation include discovery from existing world state.

### HTN composition and execution

Partial-order HTNs select parameterised activities from explicitly imported domain
libraries. Scenario state trees execute their lifecycles; explicit Rhai calls use
the established sandbox/effect boundary. Physical work stays with admitted Systems
and jobs. This does not reverse AD25's independent-job decision or merge private
controller memory. One owner advances each activity; predicted effects are not
world facts. Surviving entities and obligations outlive situation completion.

### Scheduling and pressure

Schedule using distinct ship capability, human attention and shared physical
resource inputs. Occupied Stations expose capability bundles through their hosted
Consoles; one person's Consoles share attention. Minute buckets forecast workload;
exact simulation times govern dependencies and deadlines. Larger crews receive
concurrent meaningful responsibility; combat strength remains separately balanced.
Planning forecasts do not claim physical stock. Ordinary mechanisms revalidate
and reserve at execution. Announced/committed deadlines change only through
in-world causes. Preserve recovery slack and earned advantages. Attention modelling
does not reintroduce AD74's rejected message-capacity accounting.

### Systemic opportunity discovery

Bounded GOAP-like search combines the same activity operators around authored goals
and values using actual entities, capabilities and needs. It can discover vessel
return, repair-and-evacuate or escort opportunities without bespoke templates for
every combination. Revalidate candidates and admit selected ones as optional HTN
tasks, revealed through ordinary observations, Comms and objectives. An offer is
not acceptance; neither planning nor acceptance controls human-owned equipment.
Hypothetical search does not mutate the world or guarantee player/NPC choices.

The director adapts only unannounced work within authored pacing bounds, considers
continuing obligations and prefers useful existing entities before new setup.
Low input activity alone does not establish boredom; delegation does not trigger
automatic replacement chores. Depth/candidate/expansion bounds and stable ordering
keep search bounded and deterministic; exhaustion leaves existing execution intact.

### Ownership, persistence and implementation sequence

World owns mission planning/progression, Gameplay owns physical work and Session
supplies participation/control facts. Simulation connects shared contracts without
sibling dependencies. Keep authoritative world state, plan forecasts and permitted
player/AI knowledge separate. Persist commitments, bindings, execution, deadlines,
director history and necessary search continuation through existing save/digest
mechanisms. Do not add a parallel network or individual-System controller.

- [Increment 22](../implementation/increment-22.md): contracts, HTN libraries, scheduling, monitored execution and Workshop.
- [Increment 23](../implementation/increment-23.md): authored Patrol, War and Sandbox; persistent assets and consequences.
- [Increment 24](../implementation/increment-24.md): opportunity search, admission, live pacing and original final audits.

These documents preserve the programme's playable exits and define acceptance.
Detailed encodings, solver choice and calibrated tuning are settled before their
owning runtime increment; estimates are not human-workload measurements.
