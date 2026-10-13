# Ordered gameplay System review

This is part of the [proposal GDD](./README.md), using its [design pillars](./README.md#design-pillars), [game catalogue](./game-systems.md), [ship catalogue](./ship-systems.md) and [research](./research/README.md). Proposals below are exploratory and do not amend PASM or existing delivery Plan. Networking and individual per-System AI, communication, limited knowledge and shared authority remain fixed.

## Review procedure and progress

Review in numeric order: **G001–G114, then S001–S102**. Each entry gets an inclusion recommendation, bounded scope/form, evidence, interactions, complexity/delegation, communication, pillar assessment, objections and unresolved decisions. An accepted principle does not imply every implementation detail is settled. Dependent choices can remain queued until their entry; revisit them before declaring a coherent final design.

| Position | State |
|---|---|
| G001–G114 | First ordered pass complete; selected directions and remaining details recorded below |
| S001–S102 | First ordered pass complete; selected directions and remaining details recorded below |
| Station synthesis | Reference 2/4/6/8-seat bundles and substantial Cruiser complexity choices selected; activity walkthroughs, remaining ownership details and workload validation remain |
| Deferred dependencies | Remaining equipment/thermal/resource rules and numerical tuning remain to be resolved/tested; catalogue completion is not final design validation |

## G001 — Station ownership and authored work bundles

**Inclusion recommendation: retain and develop.** Dedicated Stations give players responsibility for coherent work. Six humans is the reference crew, including Captain; 2–8 must be supported through human/AI seats and explicit human-seeking exceptions. Those are already agreed requirements. This entry chooses how work bundles are defined, not the final names or composition of all Stations.

### Evidence and relevance

Phoenix already authors Stations, owned Systems and ratings in hull data. The roster reaches clients through the existing session path; control authority and Backfill follow authored ownership. The Dynasty cruiser demonstrates regrouping existing System families into six Stations. See [Station implementation index](../../wiki/entities/station.md), [Station configuration](../../src/ship/config.rs), and [complexity design](../../pasm/spec/design/console-complexity.yaml).

The [comparable-games research](./research/comparable-games-and-falling-frontier.md#bridge-and-crew-references) records named crew responsibilities in Artemis and PULSAR, and modular presentation in Starwards. [Captain Sonar](./research/comparable-games-and-falling-frontier.md#submarines-shared-vehicles-and-other-non-space-structures) illustrates how distinct work can interlock. These support testing role clarity and complementary decisions; they do not prove Phoenix should inherit their roster or smaller-crew role-combination solutions. Configurable presentation is not evidence of safe live authority reassignment.

### Agreed form — final composition remains open

Author coherent Station bundles for the ship, stable within a mission. A bundle should connect information, ongoing operation and event decisions, with clear effects on colleagues. Vary individual depth by delegation/ratings rather than obliging players to reconstruct the crew organisation. Different authored hulls or crew layouts may offer different bundles; whether crew layouts are needed is deferred until the activity inventory and 7/8-player work are evaluated.

| Aspect | Proposed requirement or dependency |
|---|---|
| Amount | A six-human reference arrangement, plus validated 2–8 support. Exact additional seats and exceptions remain open; no busywork to manufacture extra roles. |
| Ownership | One responsible Station per System; one admitted Control Source at a time. A handover changes operator without making Captain a substitute specialist. |
| Interactions | Bundles exchange observations, requests and intentions. Shared operations make the distinctions matter. Precise information rules await G016–G026. |
| Complexity and AI | Stable responsibility can coexist with AI/simplified/detailed System control. G002/G003 decide depth and assistance; vacant seats use G004. The reactor ladder is provisional input there. |
| Communication | An officer knows what to report and whom to ask; colleagues are needed to act. Deliberate Viewscreen sharing supports discussion. No duplicate Captain map or automatic complete picture. |
| Pillars | Coherent responsibilities support expertise, conversation and individual depth. Bundle workload must also support ongoing/event activity and combat/non-combat scenarios. |
| Risks | Familiar roles can hide empty seats; custom bundles can concentrate all useful decisions; too many ownership changes can confuse requests. An authored bundle is not proof of enjoyable activity. |
| Evidence needed | Trace a bundle through quiet operation, a shared intervention and recovery. At 2, 6 and 8 humans, observe who decides, waits, communicates and delegates. Workload methods await G008/G009/G093. |

### Resolved choice: how are bundles defined?

| Option | Form | Trade-off |
|---|---|---|
| A — Designer-authored bundles | Ship content defines coherent Station roles; players select a seat and its depth. | Clear responsibilities and a testable baseline; less crew freedom to redistribute work. |
| B — Crew-configured before launch | Players assign Systems to Stations before the mission. | Flexible workloads, but setup becomes a design task and poor combinations can create waiting or overload. |
| C — Crew-reconfigured during play | Players move Systems between Station bundles in the mission. | Adaptable to circumstances, but frequent authority/information changes can undermine role clarity and coordination. |

**Selected: A.** Ship content defines the Station bundles; players choose a seat and its depth. This does not prohibit ordinary operator handover or the separately reviewed human-seeking mechanism. Final bundle composition and 7/8-player arrangements depend on later activity reviews. B/C are unselected gameplay alternatives, not promises that the current runtime supports them.

## G002 — Per-Station complexity and ratings

**Inclusion recommendation: retain and develop.** Individual complexity is essential to the agreed mixed-skill experience. Selecting less manual depth should leave worthwhile judgement and communication, rather than making the player a spectator. This entry chooses how depth is selected; G003 reviews what whole-System assistance does, and the ship review establishes each System's actual depth controls.

### Evidence and relevance

Phoenix already has designer-authored Station ratings describing AI/simplified/detailed control for owned Systems. Simplified operation uses the individual System AI to execute summary intent; direct controls must not compete with its actuator writes. Summary adapters exist only for supported kinds, so new candidates need their own valid interface. See [complexity design](../../pasm/spec/design/console-complexity.yaml) and [rating configuration](../../src/ship/config.rs).

[Elite's official manual](https://d1wv0x2frmpnh.cloudfront.net/elite/website/assets/English-PlayersGuide_v2.00-Horizons.pdf) provides a compact allocation example using three capacitor priorities. [EmptyEpsilon's Engineering guide](https://daid.github.io/EmptyEpsilon/) describes individual power and coolant allocation. These illustrate different amounts of exposed control, not interchangeable simulations or proven novice/expert modes. The [R6 references](./research/comparable-games-and-falling-frontier.md) include Rebel Galaxy and Starpoint Gemini Warlords combining manual choices with automated weapons, and Distant Worlds 2 delegating selected areas. They support selective assistance as a design direction; none establishes a Phoenix complexity interface.

### Proposed form — not yet accepted

| Aspect | Proposal and dependency |
|---|---|
| Amount | Each Station exposes meaningful depth choices rather than one global ship difficulty. Do not require identical counts of levels for every Station/System. |
| Retained activity | A low-depth operator keeps a useful goal, target, priority or operating-policy choice and remains a colleague others need to talk to. Completely AI-controlled operation serves Backfill/explicit delegation. |
| Shared model | Higher depth reveals more direct decisions over the same physical model. Lower depth does not secretly remove heat, damage, costs or knowledge limits. |
| Interaction with G001 | Changing depth does not move System ownership to another Station or redefine its work bundle. |
| Interaction with G003 | AI performs delegated execution within the existing per-System structure. The player supplies intent where simplified controls offer it. |
| Communication | Assistance may reduce manual execution; it must not supply undiscovered information or eliminate reports, resource requests and coordination. |
| Pillars | Supports individual depth and local expertise. Preserves shared causal consequences, preparation and dependence on colleagues. |
| Risks | Too many choices become configuration work; a single Station preset may force unwanted manual controls; automation may remove the interesting judgement. Detail must not hide time-critical actions. |
| Evidence needed | A novice at low depth and an expert at high depth remain useful in the same situation. Both can explain their choices and communicate changes; mode transitions preserve intent and authority. |

The proposed reactor maximum/middle/minimum/AI ladder is an example awaiting its detailed review, not a universal number of modes. Coarse views/groups must still address the same underlying Systems. The choice here does not settle coolant budgets, component repair or grouping semantics.

### Resolved choice: how does the player select depth?

| Option | Form | Trade-off |
|---|---|---|
| A — Station presets only | Choose one authored rating that sets depth/automation across the Station. | Simple and easy to teach/test; cannot tailor a single System independently. |
| B — Individual System choices only | Set each System's supported depth/automation directly. | Flexible; requires the player to configure their workload and understand the options. |
| C — Station presets with individual overrides | Start with an authored rating, then optionally change a particular System's supported depth. | Easy entry plus selective mastery; effective modes and preset/override behaviour need clear presentation. |

**Selected: A.** Complexity selection must itself be simple: players choose an authored Station preset, with no player-configured per-System overrides. A scenario can still require particular individual Systems to be brought under human control. Such requirements are scenario-authored exceptions, not a workload configuration task for the player. Preserve this distinction and the existing floor machinery; actual scenario requirements and human-seeking handling are reviewed in their relevant entries. The provisional reactor ladder remains an example awaiting detailed review.

## G003 — Whole-System AI assistants

**Inclusion: required by the agreed brief.** Assistants generally operate complete Systems. They preserve the existing individual AI structure, limited knowledge, Coordination and admitted control path. G002's authored presets determine the operator's exposed depth; choosing assistance does not require constructing an AI programme.

### Evidence and relevance

The [Phoenix AI baseline](../gdd/systems/ai-and-backfill.md) and [complexity design](../../pasm/spec/design/console-complexity.yaml) distinguish authored policy, target selection, Backfill and simplified intent control. AI and humans obey the same physical constraints; AI cannot obtain unobserved truth or bypass ownership. This structure is fixed for the redesign.

The [R6 research](./research/comparable-games-and-falling-frontier.md#capital-ships-fleet-combat-and-systemic-space) describes Rebel Galaxy's automated turret target priorities, Starpoint Gemini Warlords' manual/AI gunnery, and Distant Worlds 2's selective automation. These are references for retaining selected human decisions while delegating execution. They do not establish how Phoenix should handicap AI or prove assisted play remains engaging.

### Proposed form — competence choice pending

| Aspect | Requirement or proposal |
|---|---|
| Amount and form | One individual AI operates each delegated System through its existing authored policies and supported intent interface. No new ship-wide assistant replaces those agents. |
| Player activity | Depending on the Station preset, retain meaningful targeting, steering, priorities or operating intent while the relevant AI executes. Exact retained decisions are chosen with each System; full AI seats are covered at G004. |
| Interactions | AI may respond to its observations and legitimate Coordination, including other human/AI Systems' targets or requests. It cannot infer an uncommunicated crew plan from private state. |
| Authority | A single Control Source operates a System. Simplified controls alter intent; they do not create simultaneous human/AI actuator writes. |
| Communication | AI colleagues issue relevant reports/requests and respond through Coordination. They must not become a central information aggregator or silently replace crew discussion. |
| Pillars | Enables individual depth and small crews while preserving interdependence, local expertise and causal constraints. Assistance should remove unwanted execution burden while retaining useful decisions. |
| Risks | Too-perfect decisions make humans spectators; unreliable execution makes assistance a trap; excessive tuning makes choosing low complexity harder than playing. |
| Evidence needed | An assisted player remains useful, can understand important AI actions and can improve the crew's plan through judgement/communication. Neither novice nor expert must babysit routine execution. |
| Dependencies | G004 covers vacant seats; G006 covers stances; G007 authority; G010/G011 communication and local decisions; G101 scenario difficulty. Each ship System supplies the actual policy and control depth. |

### Resolved choice: what creates the advantage of manual operation?

| Option | Form | Trade-off |
|---|---|---|
| A — Competent execution, bounded judgement | AI executes reliably within authored knowledge, priorities and risk limits. Humans can anticipate spoken plans, recognise unusual opportunities and choose a different method. | Assistance stays useful; manual advantage must come from genuinely richer decisions and coordination. |
| B — Bounded judgement plus execution limitations | Also author slower reactions or lower execution precision so skilled manual operation has an execution advantage. | Makes manual skill visibly valuable, but can punish assisted players and turn AI supervision into work. |

**Selected: A.** AI executes competently within authored knowledge, priorities and risk limits. Human advantage comes from interpretation, anticipation, adaptable choices and coordination, rather than deliberately poor reaction or precision. AI retains ordinary resource, damage, observation and policy limits. Specific policies and NPC/scenario difficulty remain separate review topics.

## G004 — Vacant, disconnected and AFK Backfill

**Inclusion: retain ordinary Backfill, as agreed.** Supported 2–8-human play needs vacant seats to operate usefully; temporary absence must not disable the ship or force remaining humans to operate multiple manual Stations. Backfill uses the same competent individual System AI agreed at G003, not a separate simulation or all-knowing controller. **There is no unique behaviour for temporarily filling in.**

### Evidence and relevance

The [AI baseline](../gdd/systems/ai-and-backfill.md) already describes unclaimed Stations using Backfill and disconnected seats retaining their holder association for reconnect. [Crew Control](../gdd/mechanics/command-and-crew-control.md) includes explicit AFK delegation and human-seeking relocation. Preserve these foundations and the usual AI behaviour. Existing authored stances have their own persistence/invalidation rules, reviewed at G006.

[PULSAR and Wolfpack in R6](./research/comparable-games-and-falling-frontier.md) provide crew-bot references for playable small/solo crews. They support retaining empty-seat operation, but this research does not verify their handling of interrupted human intentions or reconnect. No competitor-specific takeover design is imported.

### Agreed form — ordinary AI operation

| Aspect | Requirement or proposal |
|---|---|
| Amount | Cover every ordinarily AI-operable System in an unclaimed, disconnected or AFK Station. Human-seeking exceptions remain a separate G005 review; do not silently claim every protected human choice is automatable. |
| First launch | With no prior human intent, operate from valid authored objectives, default policies and received Coordination. Humans should not have to programme each empty seat before starting. |
| Temporary absence | Use regular Backfill policies responding to Command orders, alert level, the current mission list and current situation in the usual way. No special intent-copying, frozen settings or takeover-reset policy. |
| Hysteresis | Ordinary AI behaviour should generally have hysteresis, so an agent already towing something would probably continue. This is useful regular behaviour for any AI operation, not a temporary-fill exception or a guarantee that every task continues regardless of circumstances. |
| Other humans' activity | Remaining officers continue their own Stations and communicate with AI colleagues. Authored stances at G006 can express supported changes in posture; no duplicate fine-System control panel. |
| Communication | Make takeover and important intention changes apparent. Reports/requests remain relevant to the responsible colleagues; avoid narrating every AI tick. Exact alert and report presentation belongs to later entries. |
| Return | Maintain a coherent handoff with one Control Source, current state and no stale actuator commands. Reconnect/return presentation is reviewed with the G013 player journey. |
| Pillars | Preserves dedicated roles, small-crew viability and ongoing work; sensible continuity protects shared operations and communication. |
| Risks | Policy oscillation or insufficient ordinary hysteresis can disrupt work; invisible takeover can make colleagues misjudge what will happen. Address the normal policy/communication, not a special temporary mode. |
| Evidence needed | Disconnect/AFK during a firing restraint, tow operation and power preparation; observe continuity, communication, adaptation to changed conditions and safe human return. Use the eventual selected mechanics, not assumed current support for every example. |

The previously proposed preserve-versus-reset takeover fork is superseded. Existing human/AI authority transfer still applies, but it does not imply distinct AI gameplay behaviour. Detailed ordinary policies are reviewed with their Systems.

## G005 — Human-seeking consoles and visiting tabs

**Recommendation: retain the existing complete-Station mechanism as an explicit exception**, consistent with the agreed brief. Ordinary Stations remain dedicated human/AI seats; human-seeking consoles must not become a way to pile every vacant role onto the remaining players.

The [current Crew Control design](../gdd/mechanics/command-and-crew-control.md#station-identity-hosting-and-delegation) already preserves the complete console, identity and state, automatically resolving through a finite authored list of eligible directly held Stations, then AI. It does not nest visiting Stations. Keep that form; no new acceptance dialogue, attention detector or temporary-AI variant is proposed.

Use only where the selected activity genuinely benefits from seeking a human; decide the particular consoles after their System activities are reviewed. Existing tab importance/health cues support off-screen awareness. The provisional repair and reactor ideas do not automatically qualify. Visiting work counts against its host's attention budget and must tolerate time spent on the primary Station.

The [R6 bridge references](./research/comparable-games-and-falling-frontier.md#bridge-and-crew-references) support coherent specialist roles and adaptable authored presentation, but do not verify Phoenix-style visiting-console workload. The pillar test is useful local judgement without compulsory multi-Station overload. G008/G009 establish the activity/margin criteria; G014 covers accessibility and later System reviews establish which exceptions are justified. No new mechanism decision is needed here beyond the existing constraints.

## G006 — AI stance and standing-order selection

**Recommendation: retain the existing authored Command/stance mechanism.** The user explicitly identified alert state and AI stances as valuable Captain controls, and ordinary Backfill responds to Command orders. Preserve Command as a way to express posture to the individual AI agents, not as another specialist actuator panel.

The [current stance design](../gdd/mechanics/command-and-crew-control.md) supplies an authored catalogue, objective/context-dependent choices, neutral alert fallbacks and persistence/invalidation rules. Human-operated Systems remain under their specialist's control; Command advice does not override them. Preserve these rules rather than adding a command language or new central planner.

[R6's Rebel Galaxy reference](./research/comparable-games-and-falling-frontier.md#capital-ships-fleet-combat-and-systemic-space) supplies a useful comparison for target priorities in automated weapons; it does not justify extending Captain's control over human officers. Exact stances belong to later System/activity reviews. Their amount should be small enough to understand and their effects observable. Pillar fit: overall coordination, limited direct Captain controls, competent bounded AI and continuing need for crew conversation. Test whether an order changes useful behaviour without requiring repeated micromanagement.

## G007 — System authority and shared physical rules

**Retain as a fixed foundation.** The [AI baseline](../gdd/systems/ai-and-backfill.md#authority-and-symmetry) and [System authority design](../../pasm/spec/architecture/station-system-authority.yaml) already give each System one admitted Control Source and common downstream rules. No networking, admission or individual AI replacement is proposed.

Human and AI operation incur the same costs, damage and observation limits. A Station's preset changes operation/control exposure, not the simulation; Command/Coordination express intent or advice without creating extra authority. This supports individual depth and genuine interdependence. The G003 competence decision rejects deliberate routine execution penalties; ordinary authored policy judgement still differs from human judgement. Broader fitting/research comparisons do not override these Phoenix constraints. Validate authority and parity for new Systems rather than inventing a new design fork here.

## G008 — Ongoing and event-driven activity budget

**Inclusion recommendation: adopt as a design/validation rule.** Each human Station needs worthwhile ongoing decisions and a meaningful share of event-driven interventions across a scenario. Count reasoning and effects, not button presses. A visiting console also consumes attention, even though it does not add a seat.

### Evidence and relevance

The agreed power model is an ongoing listening/support activity with rationing under pressure. [R6's Sea of Thieves reference](./research/comparable-games-and-falling-frontier.md#submarines-shared-vehicles-and-other-non-space-structures) illustrates useful settings that persist while attention shifts; Captain Sonar illustrates coupled specialist work; PlateUp! illustrates buffers and bottlenecks. These are candidate patterns, not proof that constant demand or any particular workload is enjoyable. Artemis's described fighter downtime is a warning that one exciting event can hide a long inactive role.

### Proposed form — coverage choice pending

| Aspect | Proposal or dependency |
|---|---|
| Amount | Evaluate every Station across the whole scenario and important phases. Each gets meaningful choices and contributions, including in non-combat scenarios. Exact time/count targets require playtesting, not arbitrary quotas. |
| Form | Activity cards identify changing operating decisions, event opportunities, dependencies, communication, reserves and recovery. Not every idle System needs a manual maintenance task. |
| Interaction | G001 bundles combine the selected activities; G002/G003 alter individual execution burden; G005's hosted work counts toward the same budget. |
| Communication | Listening, interpretation and coordinating a real operation are activity. Requiring extra acknowledgements to inflate participation is not. |
| Pillars | Complementary decisions, useful talk, preparation and mixed-skill operation must hold through combat, non-combat and recovery. |
| Risks | Filling every gap creates chores; accepting long passive stretches creates spectators; simultaneous interventions can overload one player. |
| Evidence needed | Observe quiet/busy periods, meaningful choices, longest unwanted inactivity, concurrent urgent demands and who influences outcomes. G093 develops measurement and G009 margins. |

### Resolved choice: must every Station always have ongoing work?

| Option | Form | Trade-off |
|---|---|---|
| A — Continuous baseline demand at every Station | Every primary Station has an operating loop relevant in every phase, with events layered on top. | Predictable engagement, but risks inventing routine work when a capability has no useful role. |
| B — Contextual loops and worthwhile work across the scenario | Ongoing demand applies where it makes sense; short quiet periods may support preparation, listening or rest. Every Station still needs meaningful intervention and influence in each scenario. | Avoids filler, but requires end-to-end pacing checks so quiet does not become exclusion. |

**Selected: B.** Power's continuous support loop remains as agreed. Other activities may be situational; no Station may be irrelevant for an entire peaceful mission or spend most of a session waiting for its one event. Short quiet periods are not a licence for dead seats. Exact bundles and acceptable pacing remain to be tested.

**Candidate added by the user:** duty-officer away assignments inspired by STO could provide interesting low-key work. This is queued for G068/G069 and S079/S080/S088 rather than being selected here. Test optional preparation/staffing and useful consequences; do not assume this means mandatory filler or a new full-time Station. See the [queued idea](./README.md#duty-officer-away-assignments-for-low-key-work).

## G009 — Reserve, grace and recoverable overload

**Inclusion recommendation: retain and develop as a cross-System design rule.** The agreed brief already requires stable operation to leave room for an intervention while ongoing demands continue. This entry determines where that room comes from and how it is communicated; exact battery, thermal, team or deadline parameters belong to later System reviews.

### Evidence and relevance

The proposed local batteries demonstrate prepared reserves, with a cost to maintaining them full. [R6's Sea of Thieves, PlateUp! and UBOAT references](./research/comparable-games-and-falling-frontier.md#submarines-shared-vehicles-and-other-non-space-structures) respectively suggest persistent useful settings, buffers/bottlenecks and constrained readiness. These support investigating margin through operating state; none supplies a universal safe response time for Phoenix.

### Proposed form — source of margin pending

| Aspect | Proposal and dependency |
|---|---|
| Amount | Enough margin from a well-prepared state to divert attention to a relevant intervention. No arbitrary universal number of seconds is adopted; validate at mixed complexity on phones. |
| Form | Visible reserves, trends and constraints let players anticipate how long an operation can continue or wait. Preparatory decisions matter; a crisis can spend the margin. |
| Interactions | Battery charge, thermal capacity, available personnel, safe geometry and actor tolerance are possible forms. They must change practical options, not duplicate meters. Detailed selection follows their catalogue entries. |
| Complexity | Lower-depth operation retains the same physical margins; competent individual AI manages delegated work normally. No separate beginner/temporary-Backfill physics or hidden immunity. |
| Communication | Operators can report a useful limit or request a delay/support. Exact estimates belong to the responsible specialist; no automatically complete Captain picture. |
| Recovery | Imperfect responses should leave meaningful degraded/recovery choices where the chosen System permits. Which consequences and recovery methods apply is deferred to damage/operations/scenario reviews. |
| Pillars | Preparation earns flexibility; ongoing and event work coexist; shared costs generate negotiation; assistants reduce execution burden without removing consequences. |
| Risks | Huge reserves make ongoing decisions irrelevant; tiny margins punish any conversation/tab change; opaque depletion makes failure feel arbitrary. |
| Evidence needed | From stable prepared operation, interrupt the operator with a reasonable task and observe warning, remaining options and recovery. Contrast prepared and depleted states without requiring perfect continuous attention. |

### Resolved choice: is response time earned through state or guaranteed?

| Option | Form | Trade-off |
|---|---|---|
| A — Prepared physical/operating margins | Available response time follows reserves, readiness and circumstances. Warnings explain that state; there is no universal extra grace timer. | Preparation and rationing matter, but margins must be tuned and legible enough for ordinary players. |
| B — A guaranteed grace stage as well | Also give critical conditions a designed response stage before their consequence, even when ordinary reserves are exhausted. | More consistent reaction opportunity, but introduces another rule and can weaken the value of preparation if applied indiscriminately. |

**Selected: A.** Get the wriggle room from prepared physical/operating state and clear warnings. Some situations may naturally have warning stages, but do not introduce a generic protective timer across all Systems. This does not decide exact empty-battery, overheating, damage or scenario deadline behaviour ahead of their reviews.

## G010 — Continuous listening and deliberate coordination

**Inclusion: required by the agreed pillars.** Communication is part of bridge play. Humans report discoveries, interpret them, listen for others' operating needs and coordinate interventions. The power operator's ongoing listening/support loop is already agreed. Do not ask again whether the crew should need to talk.

### Evidence and relevance

[R6's Captain Sonar, Sea of Thieves and Keep Talking references](./research/comparable-games-and-falling-frontier.md#submarines-shared-vehicles-and-other-non-space-structures) offer complementary information/control, listening and interpretive exchanges. Captain Sonar's compulsory move acknowledgements are not automatically a Phoenix requirement. Its role coupling and Sea of Thieves' shared operation are stronger references for useful talk than merely making players recite interface values.

Phoenix's [Coordination implementation](../../src/ship/coordination.rs) routes addressed System reports/advisories, including AI-to-human presentation; human-to-human exchanges can be spoken rather than duplicated as popups. The [AI baseline](../gdd/systems/ai-and-backfill.md#cross-system-coordination) records the shared state-derived payloads used by human/AI senders and receivers. Preserve this structure and knowledge limits. The existing AI-to-human text presentation does not settle whether the redesign should also present useful reports audibly.

### Proposed form — human communication established; AI presentation pending

| Aspect | Requirement or proposal |
|---|---|
| Amount | Talk should have useful content when something changes, a capability is needed or the plan requires coordination. Avoid constant narration and a request/approval ritual for every local action. |
| Ongoing | Listen to others' current/planned work and adapt local operation. A remark about accelerating or beginning a scan can affect power preparation without a formal request. |
| Event-driven | Report an unexpected contact, explain a constraint, request resources or coordinate a shared window. Specialist decisions remain local; Captain coordinates the overall plan. |
| Information | No automatically complete Captain picture or Captain map. Players can deliberately put information on the shared Viewscreen. Exact contact/evidence transfer rules await G016–G026. |
| AI interaction | Individual System AI sends and consumes ordinary Coordination based on its available information. It does not infer unspoken plans or generate a central perfect briefing. Communication behaviour is shared with regular Backfill. |
| Complexity | Delegating execution must preserve meaningful reporting and requests from the player's remaining work. Do not demand a new command syntax to speak to every individual agent. |
| Pillars | Local expertise becomes useful to colleagues; listening supports prepared margins; both combat and non-combat interventions can require coordination. |
| Risks | Unnoticed text can leave colleagues unaware of important changes; excessive messages/sounds become noise; automatic aggregate reports replace the need for human colleagues. |
| Evidence needed | Observe which exchanges change an action or plan, whether players can hear relevant needs while operating, and whether AI reports support a mixed crew without flooding it. Presentation/accessibility detail awaits G014/G025/G099. |

### Resolved choice: how should useful AI reports reach humans?

| Option | Form | Trade-off |
|---|---|---|
| A — Console messages | Addressed AI reports/requests appear as readable console messages and indicators. | Quiet and reviewable, but monitoring messages can compete with operating controls and offers less overheard awareness. |
| B — Audible crew reports | AI presents useful reports/requests as spoken bridge communication. | Supports listening and crew presence, but speech can overlap and be missed. |
| C — Audible reports with readable counterparts | Important AI reports/requests can be heard, with an addressed readable counterpart/history. | Supports listening and recovery of missed information; requires disciplined report priority and repeat control. |

**Selected: A, with urgency cues.** AI communication appears as console messages with levels of urgency; higher levels have notification sounds. **No AI voices**, including a synthesized or voice-acted replacement for console reports. Voice acting cost is also a stated objection. The number of urgency levels, sound priority/repeat rules and accessibility options remain for G014/G025/G099. Existing Coordination source, addressee and knowledge limits are preserved. B/C are unselected alternatives.

## G011 — Local expertise and distributed decision rights

**Retain under decisions already agreed.** Specialists control their Systems and make the operational decisions: Engineering, for example, has final authority over allocation. Captain coordinates the mission with few direct controls and no personal tactical map. The crew must report and discuss what they know and can do, with deliberate Viewscreen sharing. Do not reopen these choices or solve domination by adding approval rituals.

The [current authority and Coordination model](../gdd/systems/ai-and-backfill.md) separates a System's admitted commands from advice/requests. [R6's Captain Sonar and Keep Talking references](./research/comparable-games-and-falling-frontier.md#submarines-shared-vehicles-and-other-non-space-structures) illustrate complementary roles and knowledge, but their precise hidden-information/puzzle rules are not adopted. Phoenix's exact observation and evidence boundaries await G016–G026.

For each later activity, identify the specialist's local choice, the information they can use, how their decision affects colleagues and how they can explain a constraint or alternative. Lower complexity must leave at least some worthwhile judgement; ordinary per-System AI supplies competent execution within its limits. Risks are an experienced player dictating every method, or local authority becoming an obstruction to teamwork. Test who actually chooses methods and changes plans, not just who presses controls. No new mechanism is selected here beyond the agreed pillars/authority.

## G012 — Spectating and post-loss participation

**Inclusion recommendation: retain spectating, with a route back to useful play where a continuing scenario supports it.** A crew losing its ship while other bridges continue is different from a single-bridge mission ending for everyone. Protect against long exclusion without making loss inconsequential.

### Evidence and relevance

[Crew Control's current Spectator section](../gdd/mechanics/command-and-crew-control.md#spectator-mvp) provides public summaries, no simulation command authority, entry into eligible open Stations and cinematic wrecked-crew spectators. The [current baseline](../gdd/foundation/current-baseline.md) and catalogue distinguish this from expanded recovery/replacement acceptance. Retain the existing networking and admission boundaries.

[R6's Falling Frontier research](./research/comparable-games-and-falling-frontier.md#loss-produces-new-decisions) describes advertised simulated-personnel recovery after losses. That is a reference for consequential rescue, **not evidence of human-player respawn**. No competitor respawn rule is presumed verified by this research pass. The Phoenix decision below is a proposal judged against meaningful participation and persistent consequences.

### Selected direction — active continuation after loss

| Aspect | Proposal or dependency |
|---|---|
| Amount | Preserve spectating as an intentional option. Where the scenario continues after ship loss, provide useful active participation; loss of the mothership need not end a crew's contribution. |
| Information | Spectators use permitted public information; they do not become omniscient tactical advisers or bypass the crew's observation rules. |
| Interaction | Escape pods and duty-officer team management without mothership support are the selected continuation direction. Eligible vacant Station claims also use normal authority/Backfill handoff, but are situational and cannot be the sole route. Details depend on recovery/personnel, multi-bridge play and scenario rules. |
| Complexity and AI | Vacant-seat re-entry uses the receiving Station's authored presets and ordinary AI support. Pod/team activities must fit the same complexity and individual per-System AI foundations. No unique temporary-fill behaviour or networking redesign. |
| Communication | A returning player becomes a responsible colleague again. A spectator's presentation must not supply forbidden information to surviving crews. |
| Pillars | Restores local agency and interaction; keeps loss connected to the shared situation. Both deliberate watching and meaningful return can serve different preferences. |
| Risks | Forced watching excludes players; guaranteed instant replacement can trivialise rescue/loss; inserting a crew into already occupied Stations undermines ownership. |
| Evidence needed | Lose a ship early while another bridge continues. Measure how long its players lack useful participation, whether return has coherent costs/authority and whether surviving crews still value rescue. |

### Resolved choice: what should a continuing scenario offer a defeated crew?

| Option | Form | Trade-off |
|---|---|---|
| A — Spectating is the normal remainder | Keep current spectating/open-seat entry, with no requirement for a designed return route. | Strong elimination stakes and simple scope; an early defeat may leave a whole crew waiting. |
| B — Scenario-supported active return | Design a viable return route where play continues: eligible vacant roles or an authored recovery/replacement opportunity. Spectating remains optional. | Avoids long exclusion, but needs scenario support and consequences so losses still matter. |

**Selected: B, extended by the user.** Escape pods and duty-officer team management can support the remaining players without a mothership. Vacant seats are desirable where available, but very situational. Active survivors have their own legitimate command authority and limited observations; they do not gain Spectator omniscience. Queue pod operation, team tasks, resources, rescue, communication reach and scenario termination for their relevant recovery/personnel entries. Ship loss alone must not be assumed to end all useful play; no automatic free instant respawn is selected.

## G013 — Ready, launch, join and handover journey

**Retain the current foundation.** The [lifecycle baseline](../gdd/foundation/game-lifecycle.md#2-connecting-and-reconnecting) already supports joining without restarting, immediate ordinary Backfill on disconnection, and reconnect restoration of an available previous Station/rating without displacing its current connected holder. Fresh authoritative state replaces stale client state. Preserve easy entry, readiness and recovery, using the existing networking and Admission boundaries.

Gameplay addition: choosing an authored Station and its simple complexity preset must be enough to begin useful play. Scenario-required human Systems remain explicit. Do not put ship fitting or a long mandatory lesson before participation. Learning support is reviewed in G015; exact Station activities/preset contents follow their System reviews. Ordinary Backfill covers absence/handover under G004. This entry creates no new takeover policy or extra player approval ritual.

## G014 — Personal accessibility and input assistance

**Retain and carry forward the existing accessibility/input/comfort programme.** The [catalogue baseline](./game-systems.md) distinguishes current surfaces from planned work. Personal settings remain personal; reducing Station complexity and whole-System assistance can reduce attention/input demands but cannot substitute for accessible controls, legible information or equivalent warning cues.

Apply accessibility to every later activity: meaningful decisions must remain understandable and operable with supported assistance. G010's high-urgency sounds require readable counterparts; no AI speech is introduced. Exact warning priority/repetition belongs with the notification entries. Preserve specialist information boundaries and avoid communicating private personal settings to peers. No additional mechanic is selected here.

## G015 — Tutorials, manuals and rehearsal

**Inclusion recommendation: retain learning support and teach interacting decisions.** Phoenix's current hull tutorials/manuals provide the baseline. The [research comparison](./research/comparable-games-and-falling-frontier.md) shows why complementary roles and vehicle systems belong in the same learning problem: learning a control alone does not teach when to request power, report uncertainty or coordinate a manoeuvre.

Proposed form: optional learning, compatible with late joining and mixed experience. Teach one useful activity and its consequence before exposing deeper controls. Match guidance to the Station preset; use ordinary per-System AI as the other operators when learning alone. Rehearsal must use the normal simulation, observations and Coordination rather than training-only omniscient agents. Exact exercises depend on the later System and Station choices.

### Resolved choice: what should rehearsal teach?

| Option | Form | Trade-off |
|---|---|---|
| A — Individual Station exercises | Teach local decisions and controls; learning coordination happens in ordinary scenarios. | Smaller authoring scope, but players may know their controls without understanding their effects on colleagues. |
| B — Individual exercises plus short shared problems | Keep local practice and offer optional crew exercises such as preparing, coordinating and recovering from a shared operation; ordinary AI fills missing roles. | Teaches interactions as well as controls, but needs authored exercises that adapt to crew size and chosen complexity. |

**Selected: B.** Optional individual exercises plus short shared problems, with ordinary AI filling missing roles. Adapt to crew size and chosen Station complexity. Teach interdependence without prescribing constant communication or one correct script. Joining live play requires no mandatory tutorial. Exact exercise content follows the later activity reviews.

## G016 — Contact discovery and radar projection

**Inclusion recommendation: retain radar, with useful uncertainty and distinct specialist work.** The [current Sensors baseline](../gdd/mechanics/sensors-epistemics.md#radar-projection) projects contacts differently for Tactical, Helm, Navigation and Sensors, with separate Combat Lock, Science Target and chart-selection authority. Current scan readings and the planned knowledge ladder are distinct from basic radar visibility; the redesign must preserve that distinction rather than label the entire future model Current.

[R6's NEBULOUS and Falling Frontier analysis](./research/comparable-games-and-falling-frontier.md) motivates separating discovery from actionable engagement intelligence. HighFleet adds a useful reference for incomplete bearings and emissions. Those references do not settle which Phoenix Stations should receive a basic return automatically; the choice below is a Phoenix proposal. Exact signatures, coverage, active/passive sensors and confidence belong to later catalogue entries.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Radar should support movement, investigation, combat and rescue. Distinguish an observed return from a known identity or usable firing solution. Avoid a stream of fully identified world entities. |
| Ongoing activity | Notice changes and choose which uncertainty matters. Stable coverage can persist while the operator listens; discovery must not require repeatedly clicking every return. |
| Active intervention | Investigate a significant return, request a manoeuvre or resource, and report what it might mean. Detailed identification and assessment await G017–G021. |
| Interactions | Helm's movement changes what can be observed; Sensors interprets evidence; Tactical decides engagement; Comms checks claims. Captain receives crew reports and deliberate Viewscreen sharing, with no personal map. |
| Complexity and AI | Lower complexity delegates sensor execution to ordinary per-System AI while retaining useful investigation priorities. Exact presets and sensor controls await the ship-System review. AI uses permitted observations and Coordination. |
| Communication | Reports should supply interpretation, uncertainty and implications, rather than merely reciting a blip that every colleague already sees. Basic-return distribution remains the question below. |
| Pillars and risks | Supports complementary expertise and changing combat/non-combat problems. Too much automatic detail removes information work; routing every basic contact through a human can create transcription chores, overload and waiting. |
| Evidence needed | Compare a mixed-experience crew navigating, investigating and fighting: who notices, interprets and acts, and which spoken reports change a decision? Test with Sensors delegated as well as human-controlled. |

### Resolved choice: who receives a newly detected basic return?

| Option | Form | Trade-off |
|---|---|---|
| A — Basic returns on relevant operational radars | Helm, Navigation and Tactical can see permitted basic returns on their appropriate views. Sensors supplies deeper interpretation and assessment; no automatic complete dossier or Captain map. | Supports immediate local action, but Sensors needs valuable uncertainty-resolution work beyond announcing arrivals. |
| B — Sensors distributes contacts | Newly discovered contacts initially appear to Sensors; deliberate designation/sharing supplies other Stations' contact views. | Creates stronger information handoffs, but can make Sensors a bottleneck and routine contact forwarding a chore. |

**Selected: A.** Relevant operational radars show permitted basic returns; Sensors provides deeper interpretation and assessment. Seeing a return does not automatically reveal identity, intentions, capabilities or a firing solution. Captain has no personal map. Detailed evidence-sharing rules remain for G017–G026.

## G017 — Detection, identification, classification and assessment

**Inclusion recommendation: retain these distinctions, but make investigation serve a decision.** The [baseline Sensors design](../gdd/mechanics/sensors-epistemics.md#plan--c2c5-knowledge-ladder) plans detection, identification, classification, tactical assessment and later intrusive analysis. Current scans provide range/fidelity-dependent readings of published infrastructure; the whole planned ladder is not implemented. [R6's NEBULOUS and Falling Frontier research](./research/comparable-games-and-falling-frontier.md) supports treating discovery and useful intelligence as distinct operating problems. The enquiry structure below is a Phoenix proposal, not a verified competitor feature.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Broad unobfuscated information normally progresses through identification/classification/assessment without manually processing every contact. Retain the distinction between observations and claims. Optional deeper enquiries support one-on-one and slower situations; do not require four routine progress bars for every contact. |
| Ongoing activity | Maintain priorities as the situation changes. A known vessel's condition or emissions can change; completing identification should not permanently finish information work. |
| Active intervention | Broad scanning supports fleet-scale awareness. When warranted, penetrate deeper to resolve uncertainty relevant to approaching a distressed vessel, assessing a threat, choosing a tow method or investigating a suspicious claim. Information can be incomplete or unavailable. |
| Interactions | Sensors interprets observations; Comms supplies claims and responses; Helm changes viewing geometry; Engineering supports sensing effort; Tactical decides whether the evidence supports engagement. Sensors cannot directly read intentions or hidden world truth. |
| Complexity and AI | At low complexity, the human selects an investigation priority and ordinary System AI handles supported execution. Greater depth can expose investigation methods and competing demands; exact presets follow the ship-System review. |
| Communication | Report an implication with its evidence and limits, such as an observed power loss that affects a rescue plan. Avoid compulsory narration of every intermediate scan step. Exact distribution of findings remains for later entries. |
| Pillars and risks | Creates local interpretation and combat/non-combat decisions. Risks include repetitive scan queues, information that never changes action, excessive micromanagement, and an AI that solves all interpretation. |
| Dependencies | G018 confidence; G019 tracking; G020–G021 sensing/effort; later evidence, deception and intrusive-analysis rules. No scan hardware, resource cost or exact confidence progression is adopted here. |

### Resolved choice: what does the operator ask the scan system to do?

| Option | Form | Trade-off |
|---|---|---|
| A — Progressively complete a contact | Prioritise contacts; scan effort reveals increasingly detailed identification/classification/assessment in a standard progression. | Easy to learn, but risks turning Sensors into a queue for completing dossiers regardless of the current problem. |
| B — Investigate a relevant question | Prioritise a subject and an enquiry, such as checking claimed identity, assessing observable combat capability or determining damage relevant to rescue. Gather what supported observations can establish. | Connects effort to decisions, but needs a small readable set of enquiries and useful answers without guaranteed certainty. |

**Selected: generally A, with B for deeper detail.** Broad data that has not been obfuscated should be acquired through routine progressive assessment, allowing fleet battles without micromanaging each sensor contact. The operator can penetrate deeper for targeted enquiries in one-on-one and slower situations. This supersedes the initial recommendation to organise all investigation around enquiries. Broad assessment still depends on supported observations; it does not expose intentions or protected internals automatically. Exact broad/deep data boundaries, scan effort allocation and costs remain open for subsequent entries. No manual decoding puzzle is selected.

## G018 — Per-observer evidence and confidence

**Inclusion recommendation: retain observer-limited knowledge and readable confidence.** The [planned baseline](../gdd/mechanics/sensors-epistemics.md#plan--c2-confidence-and-per-ship-evidence) and [PASM design](../../pasm/spec/design/fields-epistemics.yaml) provide per-ship observation/evidence, provenance and named confidence tiers. This is design intent, not a claim that the complete model is Current. Preserve individual AI's permitted observations and Coordination; no global dossier grants every crew or System hidden knowledge.

Under G017, routine broad assessment updates automatically where supported, while deeper work resolves worthwhile uncertainty. Confidence must not introduce a manual confirmation task for every return. Good observations may be dependable; poor conditions, obfuscation and limited evidence must remain visible. [R6's HighFleet, Radio Commander and Falling Frontier references](./research/comparable-games-and-falling-frontier.md) motivate distinguishing incomplete observations from claims and inference; the precise confidence representation below is a Phoenix choice.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Provide concise confidence/source cues where they affect decisions, with detail available on inspection. Do not require reading a statistical dashboard during a fleet battle. |
| Activity | Broad confidence updates are routine. Human attention goes to consequential uncertainty, contradictions and whether better evidence is worth the effort. |
| Interactions | Sensors explains what is known and uncertain; Helm and Tactical act on permitted observations; Comms supplies attributed claims. Communicating information does not make it more certain. Exact distribution remains for later entries. |
| Complexity and AI | Presets change how much work is delegated, not the quality of hidden information granted. AI uses the same supported knowledge; lower complexity can summarise uncertainty without falsely presenting certainty. |
| Communication and pillars | Reports explain limitations that change a plan. Prepared observation and focused work can improve options; neither requires routine verbal confirmation of every contact. |
| Risks | One vague rating hides important differences; many badges create clutter; constant unreliability makes every action a guessing game. |
| Dependencies | G019 handles age/prediction; later evidence and deception entries handle contradictory claims. Exact tier labels, progress rules and display design remain open. |

### Resolved choice: is confidence a single contact rating or attached to individual findings?

| Option | Form | Trade-off |
|---|---|---|
| A — One contact confidence | A concise overall confidence tier summarises how well the contact is known. | Simple, but a confirmed identity can make uncertain position or damage estimates appear equally dependable. |
| B — Confidence by finding, summarised | Position, identity and condition can have different confidence; the normal view highlights consequential uncertainty, with details on inspection. Updates remain automatic. | More honest and useful, but needs disciplined presentation to avoid a badge-heavy radar. |

**Selected: B.** Confidence attaches to individual findings, with concise summaries and automatic updates. A clearly identified ship can still have an uncertain condition or location. Preserve that distinction without making the operator manage individual confidence values or inspect every contact. Exact display, tier names and progression remain open.

## G019 — Contact persistence, ageing and prediction

**Inclusion recommendation: retain useful contact history and distinguish observation from prediction.** The [Sensors baseline](../gdd/mechanics/sensors-epistemics.md) preserves timestamped scan readings/evidence and plans observation confidence. The [radar implementation index](../../wiki/concepts/radar-projection.md) distinguishes published projections from saved target memory; neither establishes the complete proposed ageing/prediction model. This entry is a gameplay proposal, not a networking or snapshot redesign.

[R6's HighFleet, Wolfpack and Radio Commander analysis](./research/comparable-games-and-falling-frontier.md#submarines-shared-vehicles-and-other-non-space-structures) motivates using incomplete observations, motion and last-known reports. Automatic prediction below is a Phoenix inference, not a claim that those references implement this exact mechanism.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | A lost return must not remain a deceptively live contact. Keep useful last-observed information, its age and source; decide below whether the normal radar also projects motion. |
| Ongoing activity | Tracking and ageing happen automatically across contacts. Sensors monitors consequential losses and changes instead of manually tending every track during fleet battles. |
| Active intervention | Choose whether to reacquire a priority contact, search a likely route, reposition for coverage or accept uncertainty. Ordinary System AI performs delegated tracking/search execution within supported knowledge. |
| Interactions | Helm changes coverage; Sensors explains what was observed versus inferred; Tactical adapts engagement and Comms can request an update. Exact target-lock/guidance effects belong to combat entries. |
| Confidence | Age affects changing facts such as location or condition, not every historical fact identically. Losing sight does not erase a previously established identity. G018's confidence-by-finding remains applicable. |
| Complexity and communication | All presets preserve honest stale/predicted cues. More depth may expose search priorities and history, not require manual plotting. Reports should explain consequential uncertainty and likely alternatives. |
| Pillars and risks | Occlusion and evasion can create changing decisions. Risks include exact-looking predictions, clutter from old tracks, excessive contact babysitting and AI knowing a target's hidden manoeuvres. |
| Dependencies | Reacquisition/association, clutter cleanup, expiry and prediction bounds remain open; G020/G021 govern sensing, and G106/G109 distinguish inferred motion from received actor reports. |

### Resolved choice: what does radar show after observation is lost?

| Option | Form | Trade-off |
|---|---|---|
| A — Last-known marker | Show the last observed position and its age, without automatically moving the marker. | Clearly historical and simple, but players must estimate movement when planning a search or manoeuvre. |
| B — Predicted track with growing uncertainty | Project from observed motion, clearly mark it as predicted and broaden its possible position as time passes. Retain the last observation for inspection. | Supports useful search decisions without manual plotting, but must never look like a live precise return or follow unseen manoeuvres. |

**Selected: B.** Automatically predicted tracks with growing positional uncertainty, clearly distinguished from live observations. Prediction uses only observed motion, with no secret updates from the target's actual position or unseen manoeuvres. Retain the last actual observation for inspection. Exact horizon, uncertainty shape, reacquisition and cleanup rules remain open.

## G020 — Wide, focused and intrusive scan trade-offs

**Selected direction: independent broad and focused sensor Systems, each drawing separate power.** G017 establishes broad assessment without fleet-scale contact micromanagement, with optional deeper enquiries. The [current scan baseline](../gdd/mechanics/sensors-epistemics.md#current-scan-mechanic) uses range, fidelity, power requirements and interference. The [existing planned wide/focused design](../../pasm/spec/design/fields-epistemics.yaml) pauses other scan progress while focusing one subject; the redesign supersedes this exclusivity rather than changing the existing Plan in place.

[R6's Falling Frontier research](./research/comparable-games-and-falling-frontier.md#information-is-an-operation) describes advertised coverage/directional-search trade-offs. NEBULOUS links sensing to engagement information. These motivate an opportunity cost for concentration; they do not determine Phoenix's exact allocation rule. Intrusive analysis of protected internals remains a candidate for deeper work, with sensing limits and possible exposure/reactions rather than guaranteed access. Exact intrusiveness, emissions and diplomacy depend on later entries.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Broad operation supplies routine assessment; focused effort resolves important detail. Neither should require repeated scan-button presses on every contact. |
| Ongoing activity | Maintain coverage and investigation priorities as demands change. Ordinary AI executes delegated sensing through normal System authority and limited observations. |
| Active intervention | Use the focused System on a threat, rescue subject or suspicious claim while broad sensing continues independently. Each draws separate power; the opportunity cost is shared ship power, not a shared scanner capacity pool. |
| Interactions | Engineering supports sensor demand; Helm changes range/geometry; Tactical and Comms explain what detail matters and when. Resource requests matter when additional effort competes with other needs, without an approval ritual for every scan. |
| Complexity | Authored Station presets can offer simple priorities or deeper allocation controls. Exact controls, concurrent subject counts and sensor hardware remain open for the ship-System review. |
| Pillars and risks | Supports shared capacity decisions and combat/non-combat investigation. Risks are free focused upgrades, abandoning all awareness for one enquiry, and a detailed allocation interface required of beginners. |
| Evidence needed | During fleet combat, investigate one important contact while others manoeuvre. Compare situational awareness, worthwhile trade-offs and attention demands; repeat for a slower rescue/investigation. |

### Resolved choice: does focused work stop broad assessment?

| Option | Form | Trade-off |
|---|---|---|
| A — Exclusive modes | Wide assessment or focused investigation; focusing pauses broad scan progress as in the existing Plan. | Strong, simple trade-off, but investigating one contact can let the wider picture age rapidly. |
| B — Shared capacity | Broad assessment continues while focused work takes part of the sensor capacity; more depth means slower or poorer updates elsewhere. | Supports fleet awareness alongside deeper work, but requires a readable capacity trade-off and simple preset/AI allocation. |

**Selected: user alternative, replacing A/B.** Broad and focused sensing are two separate Systems, neither directly detracting from the other, with separate power draws. They can run concurrently; Engineering can ration their demands through shared ship capacity. Preserve individual AI per System. This replaces the proposed shared sensor-capacity trade-off. Exact power requirements, supported focus targets, controls and whether intrusive sensing is an operation of the focused System remain for later review. S057/S058 record this as Proposal, not Current.

## G021 — Emissions, signatures and concealment

**Inclusion recommendation: include legible signatures and consequential concealment.** The [baseline](../gdd/mechanics/sensors-epistemics.md#plan--c2c5-knowledge-ladder) plans detectable intrusive scanning and Jam/Spoof/Harden; broader signature play is Plan and cloak remains unscheduled. [R6's HighFleet and Falling Frontier research](./research/comparable-games-and-falling-frontier.md) describes sensing that can expose its user, passive signatures and concealment/exposure decisions. The operating-signature model below is a Phoenix proposal, not a wholesale import.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Make detectable activity follow consistent causes and observable limits. Concealment changes what others can learn; it does not provide a free invisibility toggle or grant Sensors hidden truth. |
| Ongoing activity | Monitor changing evidence of activity and the ship's exposure. Routine signature calculation/observation updates are automatic; human attention goes to interpretation and operating priorities. |
| Active intervention | Choose whether a useful action warrants exposure, alter operation to reduce detectability or seek better confirmation. Specific active/passive scan controls await the sensor-System review. |
| Interactions | Sensor emissions, movement, power demand and combat activity are candidate signature causes; decide their scope below. Engineering supports operating priorities, Helm chooses geometry, Sensors interprets observations and Tactical weighs engagement. |
| Complexity and AI | Simple presets delegate execution while retaining consequential intent/priority choices. Individual System AI follows supported orders and knowledge; it cannot coordinate concealment through an omniscient replacement planner. |
| Communication and pillars | Exposure creates reasons to discuss approach, investigation and engagement across Stations in both combat and non-combat situations. Avoid a mandatory report for every fluctuation. |
| Risks | Too many hidden modifiers, continuous stealth micromanagement, concealment that eliminates interaction, or signatures that disclose every internal state exactly. |
| Dependencies | Exact emissions, passive detection, thresholds, countermeasures and cloaking remain for later ship-System entries. Signatures can supply evidence without proving identity or intent. |

### Resolved choice: how broadly should ordinary operation affect detectability?

| Option | Form | Trade-off |
|---|---|---|
| A — Selected emitting actions | Detectability mainly follows explicit active sensing, jamming, firing and dedicated concealment equipment. | Easier to understand and tune, but fewer ongoing links between ordinary operation and exposure. |
| B — Operating signatures | Several major Systems contribute observable signatures when operating; reducing exposure can constrain propulsion, sensing or other capability. | Creates cross-Station choices and emergent evidence, but needs a small readable set of causes rather than dozens of hidden modifiers. |

**Selected: B.** Several major Systems contribute operating signatures alongside explicit emitting actions. Reducing exposure can constrain capability. Keep the causes readable, updates automatic and stable operating priorities possible; create planning decisions rather than continuous adjustment chores. Exact contributors, detection effects and concealment controls remain open for their System reviews.

## G022 — Information sharing, provenance and Comms transfer

**Inclusion recommendation: retain attributed information transfer without routine transcription.** The [Sensors baseline](../gdd/mechanics/sensors-epistemics.md#evidence-and-dossiers) records finding source/time and preserves contradictions; current designation advises Tactical without changing its Combat Lock. Planned per-ship evidence crosses ships through explicit Comms transfer. Preserve existing networking, Admission and individual AI Coordination: this entry selects gameplay sharing policy, not a new transport.

G016 already supplies basic returns to relevant operational radars. Captain still has no personal map; human reporting and deliberate Viewscreen presentation remain established. Do not reopen those decisions. Sharing should preserve G018's confidence per finding and G019's observed/predicted distinction. A claim received from another crew is attributed information, not independently verified observation.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain source, observation time, confidence and whether information is observed, predicted or claimed. Ordinary receipt/recording is automatic once a permitted transfer occurs; no copying every fact by hand. |
| Activity | Choose whether to share Navigation, Sensors or Tactical radar with the faction. Receiving ships decide what to do with the information. Across crews, discuss significance and plans through text/audio/video Comms. |
| Interactions | Sensors supplies findings, Comms manages external exchange, and receiving specialists interpret/use them within their authority. Information transfer does not set another Station's targets or orders. |
| Complexity and AI | Simple presets allow useful high-level sharing choices; ordinary System AI executes supported transfers under authored policy and available knowledge. It cannot send unknown facts or infer unspoken intent. |
| Communication and pillars | Human exchanges convey significance, requests and plans. External data transfer can support coordination without replacing interpretation or revealing a complete world picture. |
| Risks | Repeated manual sends become a chore; unrestricted automatic sharing removes knowledge boundaries; receiving a report silently upgrades it to certainty; data sharing becomes a substitute for bridge conversation. |
| Dependencies | Faction membership, receiver presentation, exact feed contents and loss of sharing remain to be specified. G024 and later Comms Systems address disruption/security; costs and range remain open. Human media and Backfill dialogue detail follow the Comms/System reviews. |

### Resolved choice: how should allied crews exchange continuing sensor information?

| Option | Form | Trade-off |
|---|---|---|
| A — Deliberate report batches | Comms sends selected contacts/findings as timestamped reports; later updates require another send. | Clear disclosure decisions, but sustained fleet coordination can demand repetitive updates. |
| B — Reports plus scoped continuing sharing | Keep deliberate reports, and allow Comms to establish a chosen sharing scope with another crew; permitted updates then flow automatically until stopped or disrupted. | Supports fleet-scale coordination, but needs clear scope, link state and honest information limits. |

**Selected: user-defined faction feeds.** Choose to share Navigation, Sensors or Tactical radar with the faction; receivers decide how to use the information. Do not convert sharing into route, target or order control. Preserve origin, age, confidence and observed/predicted distinctions, with no Captain map or hidden truth. Text/audio/video Comms connects player-crewed ships. Backfill ships provide Comms dialogue for suggesting courses of action, with responses/execution through their ordinary AI, limited knowledge and shared physical rules. Human audio/video does not overturn G010's prohibition on AI voices. Exact feed content, presentation, faction access, media UX, action suggestions and responses remain for later entries. S065/S069 record the additional Comms directions as Proposal; current networking remains the foundation.

## G023 — Knowledge libraries and Intelligence correlation

**Inclusion recommendation: retain a usable in-game reference and correlation capability.** The [C6 Intelligence baseline](../gdd/mechanics/comms-commitments.md#plan--c6-intelligence-c7-personnelsurrender) organises known entities, registry codes, dossiers, reports, pursuit estimates and mission history, with Comms as its primary search/correlation surface. This remains Plan, not a complete Current feature. Final Station composition follows the activity reviews.

The [research register's original gap analysis](./research/feature-gap-analysis-master.md) identifies library/database and intelligence-report candidates. [R6's Falling Frontier reference](./research/comparable-games-and-falling-frontier.md#information-is-an-operation) motivates information constrained by source knowledge; it does not verify the automatic reference matching proposed below. Do not require external wiki homework or give an AI a database of hidden world facts.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Make relevant known reference material accessible in-game, separate from current observation. A class's usual capabilities are not proof of this vessel's current loadout or condition. |
| Activity | Compare observed signatures, claims and history; investigate an unusual mismatch or missing record. Routine recognition should fit G017's fleet-scale broad assessment. |
| Interactions | Sensors supplies observations, Comms contributes identities/claims/history, and specialists interpret implications. Report actionable discrepancies or alternatives rather than reading database text aloud. |
| Complexity and AI | Ordinary assistance retrieves permitted records and supports comparisons; it must expose sources/limits and leave worthwhile priorities and interpretation. Deeper presets can expose richer search tools; exact controls remain open. |
| Pillars and risks | Supports complementary expertise and non-combat investigation. Risks include homework, repeated manual lookups, perfect automatic conclusions and reference defaults mistaken for live facts. |
| Dependencies | G017–G019 govern observations/confidence/age. Later deception and actor-report entries govern contradictions. Exact reference contents, discovery and persistence remain open. |

### Resolved choice: how should routine reference matching work?

| Option | Form | Trade-off |
|---|---|---|
| A — Automatic references, deliberate investigation | Attach relevant known references to observed identities/signatures automatically; humans investigate unusual matches, discrepancies and implications. | Avoids routine lookup chores, but matching must show uncertainty and never silently decide intent. |
| B — Deliberate lookup | Operators query the library to connect observations with records, including routine matches. | Makes research an explicit activity, but can burden fleet-scale play with repetitive searching. |

**Selected: A, with an unusual-record exception.** Routine matching/attachment is automatic. If a record is marked unusual, the player finds it and explicitly flags it for attachment to an entity. The exception is an explicit record property, not a requirement to manually inspect every ambiguous match. Source and confidence remain intact; attaching reference material does not verify the vessel's current state or solve the mission. Exact marking, search and attachment controls remain open.

### G022 refinement — generic Comms applies to every ship

Every ship offers generic requests contextual to its relationship with the caller and current objectives, independently of any plot Comms from that ship. This expands the earlier Backfill-only framing. When a human controls the receiving Comms System, the generic channel becomes text chat; the controlling actor can therefore change without creating an entirely different communication concept. AI recipients interpret requests through ordinary behaviour, knowledge and physical limits. Player-ship audio/video remains available; AI voices remain excluded. Detailed request vocabulary, eligibility, response rules and simultaneous plot/generic presentation await G041 and S065/S069.

## G024 — Interception, decryption and signal deception

**Inclusion recommendation: explore interception and decoding as information work alongside electronic interference/deception.** The [catalogue](./game-systems.md) separates research interception/decryption from planned Jam/Spoof/Harden. [R6's HighFleet research](./research/comparable-games-and-falling-frontier.md#submarines-shared-vehicles-and-other-non-space-structures) provides a reference for intercepted transmissions and exposure-sensitive electronic activity. It does not establish Phoenix's exact transmission or decoding rules.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Interception can reveal attributed, incomplete information; decoding can require capability, effort or access. Electronic disruption/deception changes observations or communication through consistent rules, not omniscient remote control. |
| Ongoing activity | Ordinary System AI can maintain selected reception/search coverage. The player prioritises consequential traffic and compares findings with observations, objectives and claims. |
| Active intervention | Decide what merits interception/decoding effort, seek corroboration, or attempt disruption/deception when worth the cost and exposure. Exact actions remain for the ship-System review. |
| Interactions | Comms interprets messages, Sensors compares evidence, Engineering supports operating demands and other specialists act on implications. G021's signatures can make electronic activity consequential. |
| Complexity | Delegate acquisition/execution at lower complexity; deeper controls must offer meaningful choices. No compulsory frequency/cipher minigame is adopted here. |
| Communication and pillars | Findings should change a route, threat assessment, rescue or negotiation. Keep uncertainty and source visible; intercepted claims do not become verified truth. |
| Risks | Random clue vending, mandatory puzzle chores, overwhelming traffic, universal deception and information that exposes an actor's private state without a plausible signal. |
| Dependencies | S066/S067 cover reception/decoding; later electronic warfare Systems cover disruption. Enemy text interception in team PvP is selected; exact reach/security/decoding and audio/video interception remain open. Preserve networking and individual AI knowledge boundaries. |

### Resolved choice: what generates interceptable information?

| Option | Form | Trade-off |
|---|---|---|
| A — Authored intercept opportunities | Scenarios explicitly provide transmissions/clues at chosen moments; interception chiefly serves designed encounters. | Predictable pacing and smaller scope, but limited emergent information work between those moments. |
| B — Systemic traffic plus authored opportunities | Actors' supported communication can produce interceptable traffic as they pursue objectives; scenarios can also author particular transmissions. | Connects interception to changing situations, but needs selective, legible traffic and actor-specific knowledge rather than constant noise. |

**Selected: B, with shared human/AI channels.** AI and players use the same Comms channels. Fleet ships communicate targets and chat as players do; that supported traffic provides systemic interception opportunities alongside authored transmissions. In team PvP, enemy text chat can be intercepted. AI communications derive from its real observations, objectives and ordinary policies, not invented knowledge or a second central planner. AI remains textual under G010; human audio/video remains available under G022. Exact interception reach, access, security, decoding, disruption and audio/video interception remain open. S066 records the direction as Proposal.

## G025 — Shared Viewscreen, alerts and attention hierarchy

**Inclusion recommendation: retain deliberate Viewscreen sharing and disciplined alerts.** Shared presentation, no personal Captain map, console AI reports, urgency levels and sounds for higher urgency are already agreed. Do not reopen them. The [accessibility baseline](../gdd/foundation/onboarding-accessibility.md) requires reviewable textual/visual equivalents for important sounds and time-critical information; those are retained requirements, not AI speech.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Players deliberately put useful information on the Viewscreen. Console alerts identify source, urgency and consequence without automatically aggregating every specialist finding into a complete bridge picture. |
| Activity | Specialists notice and interpret relevant changes, report implications and call attention when colleagues need to act. Presentation supports coordinated decisions without creating a full-time display-operation job. |
| Interactions | Sensor uncertainty, resource margins, failures and requests compete for attention. Group repeated updates where useful; preserve reviewable history without requiring mandatory acknowledgement of every notification. |
| Complexity and AI | Delegated Systems still issue useful reports through ordinary Coordination. Personal assistance changes operation depth, not alert truth or AI knowledge. No AI voices. |
| Communication and pillars | Most specialist information should support human reporting rather than replace it. The unresolved choice below concerns immediate danger, not automatic tactical briefings or another Captain display. |
| Risks | Global notification noise, missed local warnings, constant interruption, compulsory acknowledgement chores and automatic presentation replacing conversation. |
| Dependencies | Exact urgency levels, repeat/escalation rules, display control/competition and personal accessibility settings remain for presentation/System review. No networking changes. |

### Resolved choice: how should immediate ship-wide danger be announced?

| Option | Form | Trade-off |
|---|---|---|
| A — Local warning, officer reports | Even urgent specialist warnings first reach the responsible Station; its human/AI operator communicates the danger to colleagues. | Strong specialist responsibility, but a distracted operator can delay awareness of an immediate shared threat. |
| B — Local by default, critical bridge warning | Most warnings remain local; a small defined class of immediate ship-wide dangers also produces a bridge-wide warning with sound and readable cue. | Gives everyone a chance to react, but thresholds must be strict to avoid noise or replacing specialist interpretation. |

**Selected: player observation and Captain-set alert condition, replacing the recommendation.** Players use their usual tools to spot danger, interpret it and communicate. Captain can set the ship's alert condition. Do not add an automatic bridge-wide danger announcement that substitutes for this process, or an automatic alert-condition transition. G010's ordinary addressed reports and urgency notification sounds remain; they are not a new omniscient warning service. Exact alert-condition effects and levels await the relevant System review.

### G024 refinement — encryption and public chatter

Comms has encryption levels and low-power public channels. Public traffic supplies a chatter feed for Comms to inspect, using the shared human/AI channel model and actual supported traffic. This adds ongoing listening and opportunities to notice useful activity without requiring decryption of every conversation. Intercepting protected traffic does not automatically expose its plaintext. Exact encryption levels, decoding requirements, power curves, range, routing and feed filtering remain for S065–S067 and related reviews; audio/video interception is still undecided.

## G026 — Mission logs and recap

**Inclusion recommendation: retain reviewable factual history and useful recaps.** The [campaign baseline](../gdd/foundation/campaign-continuity.md) distinguishes current action/history tools from planned narrative continuity/debrief presentation. The [catalogue](./game-systems.md) marks broader mission log/recap as Plan. [R6's Radio Commander analysis](./research/comparable-games-and-falling-frontier.md#submarines-shared-vehicles-and-other-non-space-structures) motivates timestamped reports without manual transcription; it does not establish a Phoenix automatic narrative generator.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Automatically preserve relevant permitted observations, reports, commitments and actions with source/time. A recap helps a returning player recover context without exposing hidden world events or every specialist's conclusions. |
| Activity | Review what happened or what was actually reported when a claim, commitment or plan needs checking. No mandatory prose entry during a crisis. |
| Interactions | Logs support evidence comparison, handover, post-loss participation and debrief. They do not replace deliberate reports or grant Captain a map/automatic complete situation picture. |
| Complexity and AI | Routine recording is automatic. Any assistance summarises supported facts and preserves uncertainty; it must not invent the crew's intent or an actor's motives. |
| Communication and pillars | Shared permitted factual history reduces repetitive catch-up while preserving the need to explain interpretation and plans in conversation. Player-authored log annotations are not selected. |
| Risks | Logging becomes typing homework, unbounded history is unreadable, or recap silently exposes hidden truth and inferred intentions. |
| Dependencies | Exact visibility, filtering, retention and campaign persistence remain open; later debrief/continuity entries determine what carries between sessions. |

### Resolved choice: should players be able to annotate the factual log?

| Option | Form | Trade-off |
|---|---|---|
| A — Factual history only | Automatically record supported events/reports; reasoning remains in ordinary conversation. | Minimal input burden, but later review may not explain why the crew made a choice. |
| B — Factual history plus optional notes | Keep automatic history and allow players to attach a short explanation, interpretation or decision note. Never require it for progress. | Preserves useful context, but needs a quick, optional interface and clear attribution. |

**Selected: A.** Automatic factual history only. Do not add optional player annotations as a mission-log mechanic. Preserve supported event/report sources, time and information limits; no invented intentions, mandatory prose or voice transcription. Exact recap presentation and retention remain for later review.

## G027 — Spatial geometry, movement and collision

**Inclusion recommendation: retain consequential position and physical limits.** The [movement baseline](../gdd/mechanics/movement.md) already separates authored motion axes/capabilities and Helm authority from Navigation destinations and Tactical arc advice. The [navigation baseline](../gdd/mechanics/navigation-relative-motion.md) includes accepted 3D extensions. This review concerns the gameplay importance of space, not replacing the simulation/network architecture or selecting an exact flight model.

[R6's NEBULOUS, Homeworld and Rebel Galaxy references](./research/comparable-games-and-falling-frontier.md) motivate geometry, formations and firing aspects as possible sources of decisions. Their exact control models are not adopted. Terrain/occlusion and relative holds are reviewed in subsequent entries.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Capital ships use 2D movement with limited up/down for ship-to-ship collision avoidance. Small craft, including shuttles, fighters and torpedoes, use full 3D. Position/orientation should change feasible actions, with legible displays rather than difficult input for its own sake. |
| Activity | Maintain useful positioning while listening, then commit to manoeuvres for sensing, rescue, protection or engagement. Ordinary System AI can execute delegated axes/capabilities using the same physical limits. |
| Interactions | Navigation proposes destinations, Helm chooses motion, Sensors explains coverage and Tactical requests useful aspect. Engineering supports demands; no specialist gains remote Helm control. |
| Complexity | Presets can delegate execution while keeping meaningful route, positioning or risk choices. Lower complexity must use the same physical space as deeper play. Exact controls remain for ship-System review. |
| Communication and pillars | Geometry creates reasons to explain a desired viewing/firing/working position and reconcile it with competing needs. Avoid requiring constant bearing transcription. |
| Risks | Unreadable vertical relationships, dexterity burden, arbitrary collisions and a dimensional model that only adds controls without worthwhile options. |
| Dependencies | G028–G032 handle terrain, fields, bodies and relative motion. Weapon arcs and manoeuvre methods follow their later entries. |

### Resolved choice: how central should three-dimensional positioning be?

| Option | Form | Trade-off |
|---|---|---|
| A — Mostly planar operations | Design ordinary encounters around a common plane; height is used for selected situations rather than a routine tactical dimension. | Easier radar reading and spoken coordination, but fewer ordinary vertical positioning choices. |
| B — Meaningful 3D operations | Height and approach direction routinely offer useful options; simple encounters can still be largely planar and presets/AI can delegate execution. | Richer positioning and coverage decisions, but requires clear displays and assistance rather than demanding manual control of every axis. |

**Selected: A, with an explicit craft split.** Retain 2D capital-ship movement, allowing limited up/down for ship-to-ship collision avoidance. Small craft such as shuttles, fighters and torpedoes use full 3D movement. This replaces the recommendation for routinely meaningful capital-ship vertical positioning. Physical capability is shared by humans/AI and does not vary with a player's complexity preset. Exact avoidance limits, small-craft controls and interactions with terrain/weapon geometry remain for later entries.

## G028 — Asteroid population and destructible terrain

**Inclusion recommendation: retain terrain that changes useful options, with destruction scope decided below.** The [world baseline](../gdd/systems/world-environmental-systems.md#asteroids-and-fields) describes seeded spatial populations, collision hazards and targetable/destructible asteroids. Currently destroyed population asteroids return when their cell is left and re-entered. This differs from persistent named scenario objects. Tactical occlusion must be credited only where supported; a visual rock is not automatically sensor or weapon cover.

[R6's NEBULOUS reference](./research/comparable-games-and-falling-frontier.md) motivates terrain affecting detection and engagement; it does not verify Phoenix occlusion or select a destructibility rule. Mining/scavenging and planetary bodies are later entries.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Terrain should create readable routes, constrained approaches or useful exposure choices. G027's capital-ship movement remains planar; small craft can use their full 3D capability. |
| Activity | Navigation/Helm select a route and position; Sensors evaluates supported coverage; Tactical may clear an obstacle if destruction has worthwhile effects. Avoid endless rock shooting as filler. |
| Interactions | Clearing can consume time, power/ammunition and attention. If cover or occlusion applies, opening a route may also expose the crew. Exact occlusion and resource costs remain unresolved. |
| Complexity and AI | Ordinary System AI can execute an approved route or attack within its limits. Humans choose whether clearing or going around better serves the plan. No extra terrain-control Station. |
| Communication and pillars | Terrain can connect travel, sensing, rescue and combat choices. Useful conversation concerns routes, clearance, exposure and the resources required. |
| Risks | Dense unavoidable collisions, decorative cover, unlimited destruction administration and cleared obstacles returning before their removal can matter. |
| Dependencies | Named versus population objects, persistence within a scenario, supported occlusion and later resources/scavenging. No permanent galaxy terrain state or networking redesign is selected. |

### Resolved choice: how much should deliberate terrain clearing matter?

| Option | Form | Trade-off |
|---|---|---|
| A — Mainly navigational obstacles | Retain destructible rocks, but encounters chiefly reward finding routes around terrain; destruction is incidental/local. | Smaller persistence/design burden, but fewer coordinated reasons to clear a path. |
| B — Selective meaningful clearing | Design some obstacles/routes where destruction changes access or exposure, and preserve that clearance long enough to matter in the scenario. | Adds coordinated rescue/combat/travel choices, but needs clear consequences and scoped persistence. |

**Selected: B.** Use selected consequential obstacles/routes where clearing changes access or exposure, and preserve clearance long enough to matter in the scenario. Do not make every rock a mission objective. Exact lifecycle/persistence, supported occlusion and resource costs remain open.

## G029 — Region hazards, interference and fog

**Inclusion recommendation: retain environmental effects as operating constraints, with potential tactical use decided below.** The [world baseline](../gdd/systems/world-environmental-systems.md#regions-current-implementation) describes existing region effects including damage, movement restrictions and sensor/Comms impairment. Graded moving fields are G030, not assumed Current here. [R6's Stormworks and HighFleet analysis](./research/comparable-games-and-falling-frontier.md) motivates conditions changing methods and imperfect sensing changing choices; it does not establish Phoenix's exact hazard rules.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Effects should have discoverable causes, intelligible consequences and recovery paths. A dangerous area changes feasible routes and operation; it should not be an arbitrary unavoidable punishment. |
| Ongoing activity | Monitor exposure, operating margins and impaired information while choosing whether to remain, traverse or leave. Broad sensing and delegated operation stay automatic within their actual capabilities. |
| Active intervention | Prepare reserves, select a route, adjust support or coordinate a task under degraded conditions. Prepared margins under G009 should buy response time rather than eliminate the environment's effect. |
| Interactions | Sensors assesses supported evidence; Helm/Navigation choose position and timing; Engineering supports endurance; Comms handles impaired contact; Tactical adapts to exposure/information constraints. Exact mitigation belongs to relevant Systems. |
| Complexity and AI | Presets can delegate execution and monitoring while retaining useful priorities. Human/AI ships experience the same physical effects; AI does not see through interference merely to keep a scenario moving. |
| Communication and pillars | Share an operational limit and its implications, rather than narrating every hazard tick. Environmental constraints support rescue/investigation and combat without requiring a GM. |
| Risks | Random punishment, stacked ongoing chores, unreadable boundaries and environmental effects that inconvenience humans while AI ignores them. |
| Dependencies | G030 covers evolution/falloff/forecasting; later Systems cover mitigation and signature/sensing effects. Exact boundaries, intensity, safe exposure and recovery rules remain open. |

### Resolved choice: should hazardous regions also offer useful opportunities?

| Option | Form | Trade-off |
|---|---|---|
| A — Primarily obstacles | Regions constrain travel and operations; crews avoid or endure them to reach a goal. | Clear danger model, but usually favours the safest bypass when available. |
| B — Costs and opportunities | Some regions offer concealment, protected approaches or useful operating positions while imposing real costs on sensing, Comms, movement or endurance. | Creates contextual choices, but effects must remain coherent and symmetrical rather than becoming free bonuses. |

**Selected: B.** Some hazardous regions offer useful opportunities alongside real costs, such as concealment with impaired own-ship observation/communication. Not every hazard needs an upside. Opportunities must follow coherent supported effects shared by human/AI ships; exact effects and mitigation remain open.

## G030 — Moving fields, falloff and physical emissions

**Inclusion recommendation: retain evolving, observable fields rather than only fixed penalty areas.** The [world baseline](../gdd/systems/world-environmental-systems.md#plan--c2-environmental-fields-c3-physical-sources) and [PASM design](../../pasm/spec/design/fields-epistemics.yaml) plan intensity, falloff, motion/growth envelopes and physical source emissions. This extends existing Regions, not a second environmental simulation. Keep the existing analytic/parametric direction and network architecture; no fluid simulation or full environmental physics is proposed.

The [Sensors baseline](../gdd/mechanics/sensors-epistemics.md#plan--c2c3-fields-and-hazard-knowledge) plans observation and forecasting, with AI using the same supported knowledge. [R6's Stormworks and Into the Breach analysis](./research/comparable-games-and-falling-frontier.md) motivates conditions affecting methods and actionable warning; neither verifies Phoenix's exact prediction or field-source model.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Moving fronts, changing intensity and bounded source effects can change routes, operation windows and available cover. Static regions remain useful; every environment need not evolve. |
| Ongoing activity | Broad sensing monitors supported trends automatically. Sensors interprets a consequential change, prioritises deeper observation and communicates a forecast with its limits. |
| Active intervention | Choose a crossing/working window, reposition, prepare reserves or act on a source if supported. No surprise unavoidable collapse merely because a hidden timer expired. |
| Interactions | Navigation compares routes/timing; Helm executes; Engineering supports endurance; Comms/Tactical adapt to changing reach/visibility. Captain receives reports and coordinates, without a personal forecast map. |
| Complexity and AI | Simple presets delegate acquisition/execution, retaining useful timing/priorities. Forecasts cannot assume unobserved source state or grant AI cleaner evidence. Exact forecast interface remains open. |
| Communication and pillars | Explain what is changing, what is likely and what operation it affects. Predictability means useful evidence and bounded behaviour, not omniscient certainty about future actor interventions. |
| Risks | Invisible causal chains, forecast micromanagement, random punishment and an expensive environmental simulation with little bridge activity. |
| Dependencies | G029 defines operational consequences; later infrastructure/emission Systems define supported sources and interventions. Exact source types, falloff, time scales and observation requirements remain open. |

### Resolved choice: can activity change environmental fields?

| Option | Form | Trade-off |
|---|---|---|
| A — Mainly authored evolution | Fields move/grow/fade according to scenario-authored envelopes; crews mainly predict, traverse or exploit them. | Readable and bounded, but fewer ways to alter the situation through action. |
| B — Authored evolution plus bounded causal changes | Keep authored envelopes, and let selected sources/events change fields—for example, disabling an interference source or a wreck creating a debris region. | Adds emergent intervention and consequences, but needs observable causes and a small supported rule set. |

**Selected: B.** Authored evolution plus selected causal field changes from actions/events. Use observable causes and a bounded rule set within the existing parametric field direction, without making every emission a new region or requiring full environmental physics. Exact links, source types, time scales and forecasting remain open for relevant reviews.

**Architecture refinement — [AD69](./architecture/index.html#spatial-fields):** environmental fields and suitable equipment emissions share supported geometry, source transforms and falloff/sampling. Separate effect processors preserve their different consequences. Extend the Regions/parametric foundation; each emission need not become a Region entity. True field state is not directly available to controllers. [AD70](./architecture/index.html#field-signal-paths) selects signal-path attenuation for applicable fields, alongside supported local effects, even when both endpoints are outside the field. It affects in-world sensing/Comms equally for human and AI equipment, without changing transport or revealing hidden field state. [AD71](./architecture/index.html#solid-terrain-signals) also selects signal obstruction/attenuation by planets, moons and suitable large asteroids through authored physical bounds and signal properties. Exact eligible bodies and channel rules remain open; S101 relay routes can carry permitted messages around obstructions, without granting direct sensor visibility.

## G031 — Physical bodies, on-rails orbits and reference frames

**Selected direction: static bodies and coherent frames; reject orbital motion.** The [world baseline](../gdd/systems/world-environmental-systems.md#celestial-bodies-and-terrain) describes current authored bodies and planned on-rails orbits. The redesign retains static bodies and rejects that orbital extension. The [Navigation baseline](../gdd/mechanics/navigation-relative-motion.md) separates stable chart coordinates, shared destinations and Helm execution. G027 constrains capital ships to 2D with limited collision-avoidance height.

[R6's Falling Frontier research](./research/comparable-games-and-falling-frontier.md#information-is-an-operation) describes celestial-body occlusion as an advertised information constraint. It does not verify an orbital-window mechanic. The operation-window proposal below is a Phoenix inference and comparison with its existing Plan.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Planets, moons, satellites and asteroid fields remain static. Bodies establish collision terrain, position and supported occlusion. Coherent reference frames keep charts, radar and spoken directions compatible. |
| Ongoing activity | Navigation reads fixed terrain alongside ordinary moving ships/objects, with supported position/prediction cues. No orbital calculation or orbital windows. |
| Active intervention | Choose an approach around fixed bodies or intercept an ordinarily moving object. Small craft use full 3D; capital ships retain planar operation. |
| Interactions | Navigation explains routes, Sensors assesses supported coverage, Helm chooses execution and Comms/Tactical coordinate. Relative operations between moving ships follow in G032. |
| Complexity and AI | Presets delegate supported execution; all players/AI share the same static-body rule and physical limits. |
| Communication and pillars | Fixed terrain and moving actors still create useful approach, rescue and engagement choices. Avoid coordinate transcription and orbital homework. |
| Risks | Decorative terrain, unreadable charts, or accidentally reintroducing orbital motion through a moving satellite/body. |
| Dependencies | Frame presentation and exact occlusion geometry/channel rules remain open; AD71 selects substantial-terrain signal occlusion. Orbiting is rejected; ordinary actor/field motion remains under its own entries. |

### Resolved choice: should body motion create gameplay windows?

| Option | Form | Trade-off |
|---|---|---|
| A — Mostly fixed operational geometry | Bodies provide terrain and place; orbital motion is absent or chiefly background presentation. | Simpler encounter reading, with less timing-based variety. |
| B — Selective on-rails operation windows | Some bodies/stations follow predictable authored paths that change useful access, coverage or rendezvous opportunities during a scenario. | Adds planning and coordination, but needs readable timing and worthwhile alternatives to waiting. |

**Selected: static bodies, rejecting orbital motion.** Orbital bodies/planets/moons/satellites/asteroid fields are static. If an object moves, it is not orbiting anything. This replaces the recommendation for selective orbital windows and the existing orbital Plan as a redesign direction. Preserve coherent charts/frames and ordinary moving-object operations; do not introduce orbiting under another label.

## G032 — Relative motion and constrained holds

**Inclusion recommendation: support useful relative operations within normal motion limits.** The [external-operation baseline](../gdd/mechanics/external-operations.md) has tractor coupling/stabilisation, formation-keep, docking and range-constrained team repair. Planned relative-motion work extends this foundation, but those existing constraints do not prove a universal Helm hold mode. G031 rejects orbiting; this entry concerns ordinary moving ships, not orbital mechanics.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Helm can enable Match Speed for its selected target. Match scalar speed within actual propulsion/observation limits; do not automatically match heading, relative position, range or aspect. |
| Activity | Speed matching persists until throttle input or another click on Match Speed. The player steers and manages working geometry. Steering does not cancel matching; matching can become infeasible. |
| Interactions | Helm owns manoeuvring; Navigation suggests destination/relative intent; Operations or Tactical explains working range/aspect; Engineering supports propulsion/operation demand. Suggestions do not take Helm authority. |
| Complexity and communication | Presets expose useful intent or deeper execution controls. Explain the requested geometry and constraints, rather than require continual numerical corrections from another officer. |
| Pillars and risks | Supports meaningful combat/non-combat coordination. Risks include trivialising all positioning, impossible perfect holds, repeated adjustment chores and magical coupling beyond the selected hardware. |
| Dependencies | Exact hold modes, authority/UI, error tolerances, target-observation requirements and break conditions await propulsion/external-operation Systems. Capital ships remain planar under G027. |

### Resolved choice: what should an assisted relative hold maintain?

| Option | Form | Trade-off |
|---|---|---|
| A — Match motion | Match a target's observed movement; the operator still corrects range and relative position. | Smaller assistance scope, but extended operations can require repetitive corrections. |
| B — Selected working geometry | Attempt to maintain a chosen relative position/range/aspect within physical limits; the operator chooses or revises the relationship. | Supports longer shared operations, but must expose limitations and leave meaningful positioning choices. |

**Selected: user-defined Match Speed, replacing the position-hold recommendation.** With a Helm target selected, click Match Speed to enable persistent matching of its speed. It ends when the player touches the throttle or clicks Match Speed again. Steering remains with the player; no automatic heading/position/range/aspect maintenance is selected. Matching can become infeasible. Exact target-loss behaviour, infeasibility feedback, acceleration limits and preset integration remain for S048; do not silently add extra cancellation rules. S048 records the distinction from the existing broader Plan.

## G033 — Civilian traffic, routes and orders

**Inclusion recommendation: retain moving civilian actors and meaningful requests.** The [world baseline](../gdd/systems/world-environmental-systems.md#traffic-and-routes) has authored routes, legs, speeds/holds and current standing orders/compliance. The [Navigation baseline](../gdd/mechanics/navigation-relative-motion.md#civilian-traffic-and-orders) separates requesting action from directly controlling another actor. G022 now provides generic relationship/objective-contextual requests for every ship, separate from plot Comms; a human receiving operator uses text chat.

[R6's X4, Foxhole and Falling Frontier analysis](./research/comparable-games-and-falling-frontier.md) motivates actors pursuing work and deliveries whose interruption matters. Their scale is not adopted; fleet/economy/logistics details await later entries. Current route following does not establish the complete objective-driven traffic model below.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Traffic should be legible and selective, with routes, needs and responses that can matter to the crew. Avoid an unbounded list of indistinguishable vessels. |
| Activity | Monitor meaningful changes, negotiate/request a hold or diversion, plan passage or provide assistance. Routine actor movement does not need player attention each tick. |
| Interactions | Navigation/Helm assess routes and safety; Comms handles generic requests and claims; Sensors supplies observations; Engineering/Operations can help if supported. An accepted request changes ordinary actor behaviour, not direct remote actuators. |
| Complexity and AI | Actors use normal individual System AI and supported objectives/knowledge. The crew delegates routine execution while retaining consequential requests and priorities. No new omniscient traffic planner. |
| Communication and pillars | Actors can explain a need or limitation and report action through shared Comms channels. Their work supports non-combat activity and consequences beyond combat targets. |
| Risks | Busywork traffic policing, identical recurring requests, invisible consequences and AI actors that ignore their own needs or knowledge merely to comply. |
| Dependencies | G034 reviews priority/congestion; later economy, commitments and deliveries define broader effects. Exact actor counts, needs, compliance and failure rules remain open. |

### Resolved choice: what drives ordinary civilian traffic?

| Option | Form | Trade-off |
|---|---|---|
| A — Authored routes and encounters | Traffic follows prescribed routes; meaningful needs/interventions mainly come from explicit scenario events. | Predictable and easy to pace, but ordinary traffic can remain scenery between encounters. |
| B — Bounded purposeful traffic | Selected vessels pursue actual scenario-relevant work, such as deliveries or evacuation, and adapt supported behaviour to delays, danger and requests. Authored encounters remain available. | Creates changing needs and consequences, but requires a readable, bounded set of objectives and responses. |

**Selected: B.** A manageable number of vessels pursue actual scenario-relevant objectives and respond to supported circumstances/requests. Their outcomes matter; authored encounters and scenario scope/pacing remain available. Preserve ordinary AI and knowledge limits, without adopting a galaxy economy or constant traffic-management chore. Exact objectives, counts and responses remain open.

## G034 — Traffic priority, congestion, panic and emergency authority

**Inclusion recommendation: include consequential access/priority problems where the situation supports them.** The [Navigation baseline](../gdd/mechanics/navigation-relative-motion.md#civilian-traffic-and-orders) already distinguishes accepted, refused and violated requests from transport failure. The catalogue's broader priority/congestion/panic work remains Plan. G022 establishes relationship/objective-contextual generic requests, and G033 establishes purposeful traffic; neither means the crew automatically controls every civilian.

[R6's Foxhole and Falling Frontier logistics analysis](./research/comparable-games-and-falling-frontier.md) motivates scarce service and route capacity affecting useful deliveries. Phoenix's priority/authority model is a proposal rather than a verified competitor rule.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Use visible conflicts over meaningful access, such as a service berth or safe passage. Priority choices should alter feasible work, not merely reorder identical request popups. |
| Activity | Discover a blockage or urgent need, compare consequences, negotiate/request an exception or provide another route/service. Routine movement continues through normal actor AI. |
| Interactions | Navigation assesses routes/access, Comms handles relationship and requests, Sensors checks supported conditions and other specialists assess what assistance is feasible. Overall coordination need not grant Captain direct traffic actuators. |
| Authority and panic | Requests reflect relationship, objectives, willingness and capability. Emergency authority is not magical compliance; urgency/panic may affect supported behaviour, with intelligible reasons and effects. Exact rules remain open. |
| Complexity and AI | Delegate routine execution and monitoring; retain consequential prioritisation. No extra temporary AI mode or omniscient traffic planner. |
| Communication and pillars | Explain whom a priority helps, whom it delays and what alternative is possible. Constraints can create rescue/logistics decisions without relying on a GM. |
| Risks | Repetitive queue tending, arbitrary refusal, identical crises, permanent specialist overload and unexplained panic causing unavoidable loss. |
| Dependencies | G035/G036 cover capacity/dependencies; later commitments and actor relationships cover terms/compliance. Exact queue, emergency authority and panic mechanics remain open. |

### Resolved choice: how routinely should the crew manage traffic priority?

| Option | Form | Trade-off |
|---|---|---|
| A — Routine flow elsewhere, crew handles exceptions | Local actors/services manage ordinary flow; the crew intervenes in consequential congestion, requests and emergencies. A traffic-focused scenario can give them a larger role. | Fits varied scenarios without constant queue tending, but traffic control is mainly event-driven. |
| B — Regular onboard traffic control | Where traffic is present, the crew routinely manages access/priority alongside exceptional interventions. | Provides an ongoing coordination activity, but risks recurring administration and distracting from other work. |

**Selected: A.** Local actors/services manage routine flow; the crew handles consequential exceptions and emergencies. Traffic-focused scenarios can expand its sustained role. Do not make routine queue tending mandatory wherever traffic exists. Exact priority, congestion, panic and authority rules remain open.

## G035 — Infrastructure condition, capacity and thresholds

**Inclusion recommendation: retain real external condition/capacity and consequential service changes.** The [world baseline](../gdd/systems/world-environmental-systems.md) and [external operations](../gdd/mechanics/external-operations.md) describe authoritative condition/capacity, target-side thresholds and real effects from repair/transfer. Published condition, stored resources and operating throughput are distinct; current thresholds do not establish a universal rule that every service's throughput scales with health.

[R6's Falling Frontier and Foxhole logistics analysis](./research/comparable-games-and-falling-frontier.md) motivates service availability depending on actual resources and deliveries. Phoenix's damage-to-service rule below is a proposal. G036 reviews dependencies and later entries review resources/components. The shared shield/System/component damage direction is now selected for facilities; detailed component statuses, repair-team work and parts rules remain provisional for later review.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Facilities use shields, Systems and components for damage, like ships. Damage to the relevant machinery changes capability and failure/recovery states. Repair or supply must change actual feasible work, not merely complete an animation. |
| Activity | Notice a consequential service problem, diagnose supported causes, estimate what can still be done and choose repair, supply, rationing or an alternative. Local actors handle ordinary operation. |
| Interactions | Sensors observes condition/capability, Comms learns claims/needs and negotiates access, Navigation/Helm reaches a useful position, Engineering/Operations supplies or repairs within supported limits. |
| Complexity and AI | Delegate execution and routine status handling; retain prioritisation and method decisions. All agents use permitted evidence, real resources and ordinary System authority. |
| Communication and pillars | Explain the remaining capability and the effect of an intervention. Partial service can create choices about whom to help and what can wait. |
| Risks | Generic health bars unrelated to services, arbitrary threshold surprises, invisible capacity changes and forced repair to full condition before anything useful happens. |
| Dependencies | Exact resource versus throughput vocabulary, degradation causes, threshold/recovery rules and component granularity remain for dependency/repair/resource reviews. No universal linear health multiplier is selected. |

### Resolved choice: how should damage affect service before complete failure?

| Option | Form | Trade-off |
|---|---|---|
| A — Mainly service thresholds | Service mostly works normally until a meaningful threshold disables or changes it. | Easy to read, but fewer choices involving reduced service before failure. |
| B — Degraded capability plus critical thresholds | Relevant damage can reduce useful service, with thresholds for distinct failures/recovery. Partial repair can restore worthwhile capability. | Creates rationing and staged recovery choices, but effects need clear causal rules rather than an unexplained percentage penalty. |

**Selected: B, with facilities treated like ships.** Facilities have shields, Systems and components for damage. Degradation/failure/recovery follows the affected machinery and can permit useful partial service/repair, rather than using a separate generic facility health penalty. Shared rules do not require identical ship/facility loadouts. Exact component statuses, damage routing, repair methods and parts remain for later entries; the user's detailed repair proposal is still queued.

## G036 — Infrastructure dependency and service networks

**Inclusion recommendation: include bounded, visible dependencies between useful services.** The [existing PASM direction](../../pasm/spec/design/fields-epistemics.yaml) uses producer/conduit/buffer/consumer vocabulary for external scenario infrastructure with capacity/dependency resolution; broader ship integration is Plan. This is a physical service/resource dependency, not a redesign of networking. G035 now gives facilities the shared shield/System/component damage direction.

[R6's Falling Frontier and Foxhole logistics analysis](./research/comparable-games-and-falling-frontier.md) motivates services depending on available supply and deliveries. Their full economic scale is not selected. Exact continuous flows and graph implementation remain outside this decision.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | A service can depend on a supported source, connection, reserve or upstream capability. Consequences ripple automatically through established dependencies, rather than requiring individually scripted downstream failures. |
| Activity | Find the relevant bottleneck, estimate what remains available and choose repair, supply, rationing or another route/source where supported. Local actors continue routine operation. |
| Interactions | Sensors observes supported condition/links; Comms obtains claims, access and priorities; Engineering/Operations assesses interventions; Navigation/Helm reaches the relevant site. All information respects source/confidence limits. |
| Complexity and AI | Ordinary AI executes supported work and requests within its knowledge. Presets simplify handling, not erase shared physical dependencies or require humans to manage a second engineering panel for every facility. |
| Communication and pillars | Explain the practical consequence of the bottleneck and compare intervention costs. Reserves can create time to act under G009; component damage can explain service loss under G035. |
| Risks | Invisible cascades, chains too large to understand, routine infrastructure administration and duplicated resource truths. |
| Dependencies | Exact link types, capacity/reserve behaviour, diagnosis and rerouting mechanisms remain for relevant System/resource reviews. No unbounded galaxy grid is selected. |

### Resolved choice: should service failure propagate through actual dependencies?

| Option | Form | Trade-off |
|---|---|---|
| A — Mainly local service failures | Damage primarily affects that facility's own services; broader consequences are separately authored scenario events. | Easier to bound, but connected services may feel like independent scripted problems. |
| B — Bounded causal service chains | Selected services depend on upstream capability, links or reserves; failures and interventions change downstream availability through those dependencies. | Creates diagnosis, prioritisation and alternative-supply choices, but requires readable causes and manageable chains. |

**Selected: B, explicitly systemic.** Propagation ripples automatically through actual dependencies. Scenarios establish the relevant relationships/state, not a separate script for every consequence. The earlier recommendation to start with manageable chains concerns legibility and scope, not an exception to automatic propagation. Reserves, supported alternatives and restored capability influence the ripple through their own rules; knowledge of downstream changes still depends on observations/Comms. Exact dependency types and recovery rules remain open.

## G037 — Workforce, strike and disposition

**Inclusion recommendation: retain workforce availability/willingness as distinct from faction hostility.** The [current world baseline](../gdd/systems/world-environmental-systems.md#infrastructure) records workforce, disposition and strike flags, but does not provide a universal automatic strike-to-rate modifier. Under G036, any selected workforce/service dependency should have a real rule and propagate its consequences automatically; do not claim the complete redesign is Current.

[R6's UBOAT analysis](./research/comparable-games-and-falling-frontier.md) provides a reference for personnel and resource choices affecting operation, alongside the risk of individual maintenance chores. It does not verify Phoenix's facility workforce or labour mechanics. Named duty officers and away teams remain for their later reviews.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Facility workforce uses the same underlying team system as player-ship repair/security teams. Availability/willingness can affect supported work, with intelligible causes; a strike does not automatically make the facility hostile. |
| Activity | Discover why service is unavailable, compare the affected work and negotiate or change practical conditions where supported. Local operation is normally autonomous rather than another player administration panel. |
| Interactions | Comms discusses needs/terms, Sensors checks observable conditions and Engineering/Operations may restore safety or provide resources. Service changes then follow actual dependencies, not a dialogue-only cosmetic flag. |
| Complexity and AI | Normal System AI executes according to supported availability/objectives/knowledge. Personal assistance does not bypass willingness or grant hidden personnel truth. |
| Communication and pillars | People provide motives and constraints that machinery alone cannot explain. Interventions can connect diplomacy, supply and repair in non-combat situations. |
| Risks | Arbitrary refusal, mood bars without causes, universal hostility from disagreement and endless population micromanagement. |
| Dependencies | Exact needs, negotiation, workforce/service links and autonomy remain open; G038/G040/G041 and later personnel entries supply related rules. Detailed individual repair-team/duty-officer design is not selected here. |

### Resolved choice: what granularity should ordinary facility workforce use?

| Option | Form | Trade-off |
|---|---|---|
| A — Work groups/roles | Track availability and willingness at a useful group/role level; named representatives can explain needs. | Supports service consequences without administering each person; less individual personnel detail. |
| B — Individual personnel | Track ordinary staff individually, with personal availability/needs contributing to service. | Allows detailed personnel stories and assignments, but increases simulation/presentation scope and risks maintenance chores. |

**Selected: A, explicitly shared with player-ship teams.** Use the same underlying team system as repair/security teams aboard player ships, rather than a separate workforce abstraction or simulation of each ordinary worker. The [team baseline](../gdd/mechanics/duty-teams-and-operations.md) distinguishes existing repair-team work from planned broader Duty Teams; the full shared facility model remains Proposal. Exact team roles, availability/willingness, service effects and personnel rules await their reviews. Shared teams preserve systemic consequences and individual AI/knowledge foundations.

## G038 — Actor needs, stocks and event-driven economy

**Inclusion recommendation: retain bounded actor needs and real resource stocks.** The [C9 baseline](../gdd/future/roadmap-baseline.md) plans event-driven actors/economy and discrete cargo lots, rather than establishing them as fully Current. G033 already selects purposeful traffic, G035 shared facility damage, G036 automatic dependency propagation and G037 shared teams. Do not ask again whether those consequences should be systemic.

[R6's X4 and Distant Worlds 2 references](./research/comparable-games-and-falling-frontier.md) motivate independent production/transport and need fulfilment; Drox Operative motivates interventions in a changing situation. Their empire/galaxy scale is not adopted. Market pricing below is a separate scope choice, not a prerequisite for systemic stocks and shortages.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Relevant actors consume, hold or obtain real supported resources. Shortages and available capacity can motivate work, requests and offers without a script for every downstream consequence. |
| Activity | Discover a need, assess what the crew can contribute and choose an intervention, delivery or exchange. Routine production/transport is autonomous within actor capability/knowledge; no compulsory resource-grinding loop. |
| Interactions | Comms discusses needs/terms, Sensors checks supported facts, Navigation/Helm plans delivery and Engineering/Operations assesses stocks, parts or service capability. Actual transport/arrival matters; deeper delivery rules follow later entries. |
| Complexity and AI | Delegate routine execution and stock handling while preserving commitments/priorities. No omniscient economic planner replaces the existing individual System AI. |
| Communication and pillars | Competing needs give useful reasons to ask what is feasible and compare consequences. Economic changes should generate bridge decisions in combat and non-combat situations. |
| Risks | Invisible background outcomes, endless shortages, obligatory trading administration and production labour that creates no worthwhile intervention. |
| Dependencies | Goods, consumption, production, contracts, cargo and delivery remain for later reviews. Sector scale/persistence and actor policies follow their catalogue entries. |

### Resolved choice: how much market pricing should the economy include?

| Option | Form | Trade-off |
|---|---|---|
| A — Needs, stocks and contextual exchanges | Use real supplies, deliveries, allocations and negotiated offers; no general supply/demand price simulation is required. | Keeps attention on practical needs and relationships, with less trading-system scope. |
| B — Add bounded market pricing | Selected goods also have prices/offers that change with supported supply and demand; trading becomes another intervention. | Creates commercial choices and alternatives, but adds pricing/balance work and can distract from shared operations. |

**Selected: A.** Real stocks, shortages, deliveries, allocations and contextual exchanges, without a general supply/demand market-price simulation. Payments and negotiated/authored offers remain possible. Physical needs and consequences remain systemic; exact goods, production/consumption and terms await later reviews.

## G039 — NPC doctrine and autonomous reactions

**Inclusion recommendation: retain authored intent and observable autonomous behaviour through existing individual System AI.** The [AI baseline](../gdd/systems/ai-and-backfill.md#npc-doctrine) describes doctrine, objectives, faction relationships and fine-System policies, with some transitional host decision kernels still present. Preserve the user's fixed AI structure and ordinary Backfill behaviour; this review is about gameplay differences expressed through those policies, not a replacement central planner or deliberate execution incompetence.

[R6's X4 and Drox Operative analysis](./research/comparable-games-and-falling-frontier.md) motivates actors pursuing objectives and creating opportunities outside player control. Individual variation below is a Phoenix proposal, not a verified competitor personality model.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Actors should pursue understandable objectives and respond to supported observations, requests, resources and risk. Meaningful differences should change decisions rather than only labels or hidden stat bonuses. |
| Activity | Observe a pattern, compare it with claims and adapt a route, offer or engagement. Humans can influence circumstances without directly controlling NPC actuators. |
| Interactions | Sensors observes behaviour; Comms asks, suggests and negotiates through shared channels; other specialists assess feasible responses. AI cannot react to hidden human intentions or information it never received. |
| Complexity and AI | Competent execution remains the baseline. Authored priorities/reserve/risk limits can vary; shared physics, knowledge boundaries and regular hysteresis remain. No unique temporary-fill behaviour. |
| Communication and pillars | Behaviour and actual Comms reports can reveal useful evidence of intent/limits. Fleet target communication and chatter stay grounded in supported knowledge under G024. |
| Risks | Arbitrary personality dice, factions with a bespoke rulebook each, perfectly predictable exploits, secret bonuses and AI decisions disconnected from its stated needs. |
| Dependencies | G040 reviews relationships; G041 terms/commitments; later ship/asymmetry and scenario entries handle hardware, force composition and pacing. Exact policy values/profiles remain open. |

### Resolved choice: where should behavioural variation live?

| Option | Form | Trade-off |
|---|---|---|
| A — Primarily faction doctrine | Most ships of a faction share recognisable operating priorities; differences mainly come from role, equipment and current objectives. | Easier to learn, but individual encounters can become predictable. |
| B — Doctrine plus limited actor variation | Keep recognisable doctrine, with selected actors differing in priorities, risk or reserve policies; behaviour and Comms can supply evidence of those differences. | Supports investigation/negotiation, but variation must be coherent and readable rather than random. |

**Selected: B.** Recognisable doctrine plus limited consistent actor differences in priorities, risk or reserve policies, expressed through existing individual System AI. Behaviour and Comms can supply evidence; no elaborate personality simulation, hidden bonuses or automatic trait revelation. Exact profiles and evidence remain open.

## G040 — Factions, standing and reputation

**Inclusion recommendation: retain relationships that change feasible cooperation and conflict.** The [campaign baseline](../gdd/foundation/campaign-continuity.md) preserves authored party standing and workforce context, explicitly distinguishing it from a universal morality score. [Comms](../gdd/mechanics/comms-commitments.md) already records promises and their settlement. Broader layered standing is Plan C9, not established Current. G022 requires generic requests contextual to relationship/objectives; G039 permits consistent individual differences.

[R6's Drox Operative, X4 and Rebel Galaxy references](./research/comparable-games-and-falling-frontier.md) motivate faction interactions and consequential exchanges. The precise trust model below is a Phoenix proposal rather than a verified import.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Relationships affect willingness, access, cooperation and hostility through understandable rules. Do not reduce every action to a universal good/evil score or infer a market-price engine rejected in G038. |
| Activity | Understand whose cooperation is needed, decide which obligations to accept and weigh practical consequences of help, refusal or conflict. |
| Interactions | Comms negotiates and interprets responses; other specialists assess feasibility and follow through. Actor resources/objectives still constrain cooperation even when relationships are favourable. |
| Complexity and AI | Routine responses follow supported policy and knowledge. Human/AI recipients use the same Comms concept; trust does not grant another ship's direct controls. |
| Communication and pillars | Different interests can make local interpretation and coordination useful. Acknowledged past actions/promises provide context for future requests, without requiring a GM. |
| Risks | Arbitrary numerical penalties, perfect knowledge of unseen actions, repetitive reputation grinding and faction labels dictating every individual's response. |
| Dependencies | G041 handles terms/commitments; later continuity and actor-report entries govern persistence and propagation. Exact measures, decay, disclosure and hostility rules remain open. |

### Resolved choice: should trust be shared faction-wide or also belong to individual actors?

| Option | Form | Trade-off |
|---|---|---|
| A — Primarily shared faction standing | A faction-level relationship drives most responses; local exceptions are authored. | Easier to read, but helping one actor can make every faction member react alike. |
| B — Faction relationships plus actor trust | Faction allegiance establishes context; individual ships/organisations can also remember relevant known interactions and commitments. | Supports contextual cooperation and consequences, but needs concise presentation and honest knowledge limits. |

**Selected: B.** Faction relationships establish context, with individual ship/organisation trust based on known interactions and commitments. An unfamiliar actor need not share another actor's experience. Exact measures, presentation, information spread and persistence remain open; do not introduce an omniscient reputation update or universal morality score.

## G041 — Diplomacy, dialogue and commitments

**Inclusion recommendation: retain consequential dialogue and track useful obligations.** The [current Comms baseline](../gdd/mechanics/comms-commitments.md#commitments) has an authored commitment ledger: scenarios record and settle promises, while the ledger itself does not judge fulfilment. G022 now separates all-ship generic requests from plot Comms, using text chat for human receiving operators; G040 gives known commitments relationship consequences. Preserve existing System authority and knowledge limits.

[R6's Rebel Galaxy and Radio Commander analysis](./research/comparable-games-and-falling-frontier.md) motivates consequential dialogue, requests and follow-up. Reusable automatically tracked commitments below are a Phoenix proposal, not a verified competitor mechanism.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Dialogue should change practical options, cooperation or obligations. Ordinary talk need not create a formal contract; plot dialogue remains separately authored. |
| Activity | Compare a requested action with available capability, resources and competing priorities; make, revise or refuse terms, then follow through. Avoid long dialogue monopolising a specialist while other duties collapse. |
| Interactions | Comms conducts exchanges, specialists explain feasibility and the crew coordinates execution. Promising a repair does not reserve teams or actuate Systems magically. |
| Complexity and AI | Ordinary AI handles supported routine exchanges under its policies; consequential choices remain subject to the agreed Station/scenario human-control rules. AI cannot promise based on hidden capacity or unspoken plans. |
| Communication and pillars | A promise gives colleagues a concrete reason to coordinate work. Recipient knowledge still governs relationship responses; physically completing work does not automatically inform every faction member. |
| Risks | Every message becomes contract administration, promises judged by opaque rules, invented interpretation of human chat and automatic negotiation removing meaningful human decisions. |
| Dependencies | Exact terms, acceptance/recording UX, fulfilment evidence, failure/renegotiation and actor knowledge remain open. G026 factual history records supported events without inventing intent. |

### Resolved choice: can generic exchanges create automatically tracked commitments?

| Option | Form | Trade-off |
|---|---|---|
| A — Formal promises remain authored | Generic requests affect ordinary behaviour; scenarios explicitly create and settle formal obligations. | Smaller rule set, but ordinary negotiations have less reusable follow-through outside authored plots. |
| B — Reusable commitments plus authored plots | Supported generic offers can become explicit obligations with clear criteria, tracked from actual outcomes; bespoke plot promises remain available. | Connects systemic negotiation to consequences, but needs readable terms and deliberate acceptance rather than guessing intent from chat. |

**Selected: A, with PCG side-mission support.** Formal commitments remain tied to authored content; generated commitments can arise through PCG side missions, not a universal obligation layer for generic requests. This replaces the recommendation for reusable commitments in ordinary exchanges. Generic Comms still affects supported behaviour/relationships, separately from plot or generated mission contracts. Exact PCG terms, acceptance and settlement await mission-generation review.

## G042 — Surrender, compliance and coercion

**Inclusion recommendation: include surrender as a consequential interaction, with persistent actors and observable compliance.** The [C7 baseline](../gdd/mechanics/comms-commitments.md#plan--c6-intelligence-c7-personnelsurrender) plans acceptance/refusal/counteroffer/feigned compliance and changes to objectives, hostility and vessel orders without automatic despawn or ownership transfer. This is Plan, not fully Current. [R6's Artemis reference](./research/comparable-games-and-falling-frontier.md) establishes surrender messaging as a bridge-game comparison, not this proposal's exact rules.

G022 supplies contextual generic requests and human text channels; G039/G040 supply bounded policies and known relationships. G041 limits formal commitments to authored/PCG side-mission content; surrender dialogue need not create a universal formal contract.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | A surrender offer/demand can change an encounter's objectives and behaviour, leaving real ships, crews and resources in the world. Do not turn surrender into automatic despawning or a free replacement asset. |
| Activity | Assess leverage and risk, negotiate supported terms, observe the response and decide what safeguards or follow-up are feasible. |
| Interactions | Comms exchanges terms; Sensors/Tactical observe supported weapons/movement; Helm positions safely; other specialists assess rescue, escort or security needs. No specialist gains remote actuators merely by sending a demand. |
| Complexity and AI | AI responds through ordinary policy, knowledge and physical limits. Human receiving Comms uses the same channel concept; behaviour must remain legible and consequences coherent. |
| Communication and pillars | Terms create useful coordinated interventions beyond shooting. Observed conduct, claims and confidence remain distinct. |
| Risks | Universal fake surrender, arbitrary instant capture, removal of useful player participation and a dialogue option bypassing physical capability. |
| Dependencies | Exact offer/demand types, threat assessment, capture/security, deception and consequences remain for later entries. G012's useful post-loss participation constraint still applies. |

### Resolved choice: is accepted surrender enforced by the game or verified through behaviour?

| Option | Form | Trade-off |
|---|---|---|
| A — Enforced compliance | Acceptance mechanically prevents selected violations, such as firing or leaving the agreed area. | Clear and reliable, but turns agreement into restrictions independent of physical control. |
| B — Behavioural compliance | Acceptance changes supported orders/intent; conduct remains physically possible and can be observed, challenged or violated. | Creates monitoring and safeguard decisions, but violations need plausible policies/consequences rather than routine random betrayal. |

**Selected: B.** Acceptance changes supported orders/intent, with observable behavioural compliance and possible violations. No magical actuator lock, automatic despawn or ownership transfer. Violations need coherent causes/consequences rather than routine random betrayal. Exact terms, follow-up and capture mechanics remain for later entries.

## G043 — Allied ship orders and support

**Retain under the already agreed Comms/AI model.** G022 supplies contextual generic requests for every ship, with human receiving Comms as text chat. G024 supplies shared fleet channels, targets and chatter. G039/G040 supply ordinary policy differences, relationships and actor knowledge. Allied support therefore uses requests for supported actions, not an added RTS surface or direct control of another ship's actuators.

The [C7 baseline](../gdd/future/roadmap-baseline.md) plans autonomous allies/Comms orders. [R6's Starpoint Gemini Warlords and Homeworld analysis](./research/comparable-games-and-falling-frontier.md) motivates useful allied capabilities while warning against fleet micromanagement consuming the bridge. Requesting support is meaningful when it changes reconnaissance, protection, recovery or supply options; the exact action set depends on later ship-System/activity reviews.

Receiving AI uses ordinary individual System policies, objectives, knowledge and physical limits. Human recipients decide through the same communication concept. Report progress, refusal or difficulty through supported Comms rather than exposing hidden truth. Difficulty or loss of contact does not introduce unique temporary-fill behaviour. Preserve local specialist control and useful work for the requesting crew; no approval ritual for each allied actuator action. Exact action vocabulary, task persistence, fleet status presentation and compliance consequences remain open. No further choice is introduced here beyond prior decisions.

## G044 — Pursuit and inter-system pressure

**Inclusion recommendation: support consequential pursuit without a universal hurry timer.** The [C7 roadmap baseline](../gdd/future/roadmap-baseline.md) includes Pursuit; it is Plan, not fully Current. G019 supplies honest prediction, G021 signatures, G024 interception and G039 knowledge-limited actor behaviour. Those can support discovering, avoiding or countering a pursuer without giving it the crew's hidden destination.

[R6's HighFleet and Radio Commander analysis](./research/comparable-games-and-falling-frontier.md) motivates exposure, reports and last-known information. Persistence across inter-system travel below is a Phoenix scope proposal, not a claim those references implement the same pursuit rule.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Pursuit should come from actors with objectives, knowledge and capability. Estimates of threat/lead are information, not an omniscient countdown that guarantees an enemy arrival regardless of events. |
| Activity | Assess whether time spent repairing, rescuing, hiding or diverting is worthwhile; act on evidence of pursuit rather than maintain constant maximum speed. |
| Interactions | Sensors/Comms infer supported pursuit evidence; Navigation compares destinations/routes; Helm executes; Engineering prepares endurance and other specialists assess interventions. |
| Complexity and AI | Normal individual System AI handles supported travel/operations, with the same knowledge limits. Complexity presets do not grant perfect lead or destination knowledge. |
| Communication and pillars | Pursuit can create coordinated choices about preparedness, competing work and misleading/exposing information. It need not be active in every scenario or eliminate quieter periods. |
| Risks | Compulsory hurry, invisible pursuer teleportation, unavoidable catch-up and a permanent backlog of threats with no effective counterplay. |
| Dependencies | Later travel/scenario/continuity entries determine transitions, scale and persistence. Exact tracking, loss/reacquisition, off-screen representation and countermeasures remain open; no new network architecture. |

### Resolved choice: can pursuit continue across inter-system travel?

| Option | Form | Trade-off |
|---|---|---|
| A — Local pursuit | Pursuit primarily operates within an encounter; leaving ends it unless an authored continuation specifically restores it. | Clear scope and respite, but travel can trivially remove a persistent threat. |
| B — Knowledge-limited continuing pursuit | In supported scenarios, pursuers can continue across travel when their information, objectives and capability permit; breaking the trail can matter. | Creates route/time/deception choices, but must provide counterplay and avoid an omniscient chase. |

**Selected: objective-dependent behaviour, including B for hunters.** Foes have different objectives: defending a base can justify breaking off early, while hunting the players can justify continuing pursuit across travel. Apply ordinary policies, knowledge and capability; neither every foe pursues indefinitely nor every departure resets pursuit. Exact break-off/continuation criteria, tracking and off-screen representation remain open. No new AI planner or full galaxy simulation is selected.

## G045 — Anomalies, black holes and wormholes

**Inclusion recommendation: consider a selective set of unusual phenomena that creates useful investigation and operation choices.** The [catalogue](./game-systems.md) separates research black-hole/wormhole ideas from planned generated anomalies/terrain. The [original research register](./research/feature-gap-analysis-master.md) includes wormhole pairs as an EmptyEpsilon-inspired candidate; it does not establish Phoenix portal mechanics as Current or committed Plan.

G029/G030 provide coherent hazards, observable changing fields and interventions. G031 keeps celestial bodies static and rejects orbiting; anomaly content must not silently restore orbital capital-ship manoeuvring. Specific exotic physics, instant-kill hazards and teleportation rules are not selected here.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | An unusual site should have discoverable effects and worthwhile approaches, not just a scan progress bar or an arbitrary punishment. Reuse supported sensing, fields, resources and operation rules where useful. |
| Activity | Broad sensing notices evidence, focused sensing resolves important detail, and the crew chooses investigation, exploitation or avoidance. Normal AI handles delegated execution within observation limits. |
| Interactions | Sensors explains evidence/uncertainty, Navigation/Helm considers access and routes, Engineering assesses support/endurance and Comms/Operations considers affected actors or work. Exact tasks depend on the phenomenon. |
| Complexity and communication | Keep useful intent choices at low complexity; deeper interpretation can supply reasons to discuss method, preparation and risk. No mandatory decoding puzzle or exact hidden explanation is required. |
| Pillars and risks | Can support non-combat discovery and changed combat/logistics options. Risks include unexplained randomness, one-player scan-and-collect chores and a bespoke rulebook for every phenomenon. |
| Dependencies | G046 handles acquisition sites; later travel, observation and scenario-generation entries govern route changes/content. Exact phenomenon types, costs, discovery and persistence remain open. |

### Resolved choice: can anomalies change travel options, or mainly provide investigation/hazard work?

| Option | Form | Trade-off |
|---|---|---|
| A — Investigation and local effects | Sites reveal evidence/resources or impose local hazards; ordinary travel connections remain unchanged. | Smaller rule set, but less opportunity for discoveries to alter strategic routes. |
| B — Include selective route-changing phenomena | Alongside investigation/local effects, some discoveries can provide or change usable routes, such as a wormhole connection. | Creates navigation, preparation and access decisions, but requires clear supported behaviour and consequences. |

**Selected: B.** Include selective route-changing phenomena alongside investigation/resources/local effects, where discoveries create worthwhile crew choices. Exact types, transit, costs and persistence remain open; no universal portal implementation, instant-kill black-hole rule or mandatory anomaly in every scenario.

## G046 — Salvage, mining and discovery sites

**Inclusion recommendation: include acquisition jobs with situational choices and useful outcomes.** The [catalogue](./game-systems.md) labels general salvage/mining as Research/Proposal, not fully Current. [Existing external operations](../gdd/mechanics/external-operations.md) provide real towing/transfer/repair foundations but do not establish generic loot extraction or recycling. [R6's Avorion and EVE references](./research/comparable-games-and-falling-frontier.md) establish mining/scavenging/trade as comparison activities; Phoenix should avoid copying MMO harvesting labour.

The user's provisional parts/inventory/scavenging idea reaches this entry. Select the acquisition direction here; exact repair parts, desired-stock automation and component status rules remain queued for their entries. G035 shares facility/ship damage, and G038 uses real stocks without a general market-price engine.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | A site should offer resources, recovery or evidence that affects useful work. Acquisition draws from supported source state and changes stocks; no repeated scan/collect chore solely to fill downtime. |
| Activity | Choose whether the return is worth the time, exposure, capacity and competing needs; select a supported method and decide when to stop or leave. Ordinary System AI performs delegated extraction/recovery execution. |
| Interactions | Sensors assesses supported contents/condition/hazards, Navigation/Helm positions, Engineering/Operations assesses needs/equipment and Comms checks claims/access. Tactical protects an exposed operation where relevant. |
| Complexity | Presets can delegate whole acquisition Systems while retaining target, priority and risk decisions. Do not require repeated manual harvesting inputs or an extra dedicated Station for every tool. |
| Communication and pillars | Explain what is needed, what can be recovered and whether time/resources should go elsewhere. Recovery can connect combat consequences to non-combat work without a GM. |
| Risks | Compulsory grinding, anonymous loot unrelated to the source, inventory overload and one officer doing all work while colleagues wait. |
| Dependencies | G065/G066 and S089–S093 cover stores/acquisition equipment; damage/repair entries cover component conditions and parts. Exact yields, compatibility, extraction/recycling and persistence remain open. |

### Resolved choice: what should wreck salvage recover?

| Option | Form | Trade-off |
|---|---|---|
| A — Standard resource yields | Wrecks provide a bounded material/parts yield based on their type and condition; contents are mainly acquisition resources. | Simple inventory and balancing, but recovery is less connected to what survived the encounter. |
| B — Surviving cargo/components plus materials | Supported recovery can take actual remaining cargo or useful components, alongside raw materials where appropriate. | Links salvage to damage/stocks and creates recovery priorities, but needs manageable compatibility and condition rules. |

**Selected: B.** Recover actual surviving cargo and useful components alongside raw materials where appropriate. Recovery reflects supported remaining state, not anonymous loot independent of the source. Exact compatibility, condition, component/part conversion, inventory representation and methods remain for later review; do not require every component to become an individual inventory item or adopt unrestricted ship construction here.

## G047 — Coarse Orbital Mode and planetary points of interest

**Inclusion recommendation: retain useful planetary operations; review the separate map mode rather than infer it from the setting.** The [C8 baseline](../gdd/future/roadmap-baseline.md) plans a coarse wrapping planetary surface with points of interest. The [existing detail](../gdd/future/planned-not-scheduled.md) uses normal planar controls and an authored outward-thrust escape hold, without atmospheric flight or terrain collision. Despite that document's historical filename, the roadmap places this feature in C8.

This mode is different from actual body orbiting, which G031 rejects. Planets remain static; capital ships retain G027's 2D movement and small craft full 3D. Planetary scanning, Comms, team assignments or small-craft work can also exist without the wrapping map. The choice below concerns the amount of gameplay machinery required, not permission to restore orbital motion.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Surface-related sites should provide useful investigation, assistance, negotiation or operations. Their activity must matter to the crew rather than merely add a destination marker. |
| Activity | Discover a site's need/evidence, choose an approach or assignment and coordinate supported teams/craft while the bridge retains worthwhile work. Exact personnel/craft activities follow later entries. |
| Interactions | Sensors investigates, Comms contacts actors, Navigation/Helm selects a working position and Operations/Engineering supports delivery or personnel. No automatic need for a separate planetary Station. |
| Complexity and AI | Presets delegate supported execution under the same physical/knowledge rules. No mandatory on-foot play, terrain-following flight or individual survival chores are adopted. |
| Communication and pillars | Planetary sites can support combat/non-combat shared decisions and optional GM play, without requiring one to create all activity. |
| Risks | A second travel mode adds controls/transitions but few decisions, stranded bridge players, and a mode name silently reviving rejected orbital mechanics. |
| Dependencies | Later teams, craft, travel and generated-site entries define exact activity. Landing/access/range, views and any mode transitions remain unresolved. |

### Resolved choice: do planetary activities need the separate wrapping map mode?

| Option | Form | Trade-off |
|---|---|---|
| A — Sites through ordinary operations | Represent planetary points of interest through existing space/Comms/sensing and supported team/craft operations; no separate wrapping planetary travel mode. | Reuses coherent bridge tools, but provides less direct global-surface navigation. |
| B — Retain coarse planetary mode | Use C8's separate wrapping surface map with normal planar controls and site interactions, while the planet itself stays static. | Supports distinct global-surface routes, but adds presentation/transitions and needs worthwhile choices to justify them. |

**Selected: B.** Retain the coarse wrapping planetary map mode with normal planar capital-ship controls and a static planet. No orbital body motion is restored. The user names Dropfleet Commander as inspiration for orbital activities; [R7 initial notes](./research/dropfleet-commander.md) record primary sources, candidate transfers and the remaining detailed rules review. Exact sites, deployment, transit and transitions remain open, rather than importing a tabletop ruleset wholesale.

## G048 — Target acquisition, Combat Lock and Science designation

**Inclusion recommendation: retain distinct acquisition, investigation and engagement authority; review offensive target concurrency.** The [current weapons baseline](../gdd/mechanics/targeting-weapons.md#combat-lock-authority) uses one authoritative Combat Lock per ship, shared by human/AI. The [Sensors baseline](../gdd/mechanics/sensors-epistemics.md#science-target-and-designation) keeps Science Target separate and sends advisory designation without writing Tactical's lock. This is the current comparison point, not a requirement to retain one offensive target in the redesign.

[R6's NEBULOUS research](./research/comparable-games-and-falling-frontier.md) describes weapon groups engaging different targets. G017 supports fleet-scale broad assessment; the whole-System assistant model supports routine firing against selected targets. A richer targeting model must preserve local authority and readable acquisition limits, rather than become fleet micromanagement or grant AI hidden tracks.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Distinguish radar/UI selection, a designation/advisory, an acquired engagement target and actual weapon readiness. An uncertain or known contact does not guarantee range, arc or a usable solution. |
| Activity | Prioritise threats and opportunities, select supported engagements and observe results. Ordinary System AI can execute firing/maintenance under those priorities. |
| Interactions | Sensors supplies evidence/designation, Tactical chooses engagement and Helm decides positioning. Power/ammunition/defence demands follow later entries. A designation is advice, not remote control. |
| Complexity | Keep basic target choice useful at low complexity; deeper presets can expose richer assignments only where worthwhile. No need to click every emitter every tick. |
| Communication and pillars | Target priorities can require reports, resource negotiation and geometry requests. More targets must create meaningful choices rather than merely more controls. |
| Risks | A single lock blocks useful concurrency; multiple assignments overwhelm novices; AI independently attacks without authorised intent; invisible differences between selected/acquired targets. |
| Dependencies | G049–G053 cover readiness, solutions, defence and engagement; exact target authority/UI and weapon-System ownership follow later review. No network redesign. |

### Resolved choice: how many offensive targets can the ship engage?

| Option | Form | Trade-off |
|---|---|---|
| A — One offensive Combat Lock | Keep the common target for offensive weapons; defensive engagement is reviewed separately. | Simple coordination, but can prevent useful simultaneous attacks by different weapon Systems. |
| B — Primary target plus System assignments | Keep a simple main target, with supported individual weapon Systems able to receive separate target assignments/priorities. Ordinary AI handles routine execution. | Supports layered fleet combat, but needs clear authority and presets that avoid emitter-by-emitter administration. |

**Selected: B.** Keep a primary target and allow supported individual weapon Systems separate target assignments/priorities, with ordinary AI executing delegated engagements. Simple presets retain main-target play. Exact assignment scope, AI selection policies and acquisition limits remain open; Science designation stays advisory. No emitter-by-emitter administration or network redesign is adopted.

## G049 — Arcs, range, lead and exposure

**Inclusion recommendation: retain consequential engagement geometry and readable weapon-dependent performance.** The [current baseline](../gdd/mechanics/targeting-weapons.md) already combines range/arc/readiness gates with sustained beams, predicted non-homing blaster volleys and guided torpedoes. Tactical's bearing request is advice to Helm, not control of the throttle. The redesign must preserve that local agency and G027's movement constraints.

[R6](./research/comparable-games-and-falling-frontier.md) provides two useful comparisons: Rebel Galaxy makes capital-ship aspect and automated turret priorities matter; NEBULOUS separates contact knowledge from usable fire-control and illumination, with terrain affecting those opportunities. These support coordination over position and exposure, without importing their complete combat models. Occlusion and manoeuvre-induced faults have their own later reviews at G105/G110.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain authored arcs, physical reach, readiness and weapon-dependent lead/guidance. Review whether favourable geometry merely permits firing or also materially improves expected effectiveness within reach. |
| Activity | Tactical judges useful attack opportunities; Helm chooses a pass, turn, distance or escape; Sensors assesses supported contact/solution evidence. Engineering prepares the power margin needed for the chosen operation. |
| Complexity and AI | A simple preset exposes a useful target and concise engagement advice. Deeper presets can expose causes and expected performance. Ordinary System AI handles aiming/firing execution under the same information and physical limits. No compulsory manual reticle aiming. |
| Communication and pillars | Requests should explain an opportunity and its cost: a steadier or closer pass may improve an attack while exposing the ship or delaying another objective. Helm decides how to meet or decline that request. |
| Risks | Binary range dancing, opaque misses, an always-best distance, excessive prediction work and Tactical prescribing every steering input. Avoid adding a generic accuracy penalty to every weapon regardless of how it works. |
| Dependencies | G048 target assignments, G051 salvo defence, G052/G053 engagement geometry, G105 occlusion/fire-control, G110 readiness and later weapon-System reviews. Exact falloff, lead, tracking and guidance rules remain open. |

### Resolved choice: how much should geometry matter inside a weapon's firing envelope?

| Option | Form | Trade-off |
|---|---|---|
| A — Mainly firing opportunities | Range, arcs and readiness primarily determine whether weapons can fire; retain existing projectile lead/guidance effects without making favourable geometry a broader performance layer. | Easier to read, but positioning can become repeatedly crossing a firing boundary. |
| B — Graduated, weapon-dependent effectiveness | Within physical limits, distance, relative motion or supported solution quality can materially affect effectiveness where appropriate to the weapon. | Creates worthwhile passes and exposure choices, but requires clear causes and avoids an arbitrary universal hit-chance roll. |

**Selected: B.** Let distance, relative motion or supported solution quality materially affect effectiveness where appropriate to the weapon. Positioning and information can improve an attack, with clear causes and ordinary AI handling routine execution. Exact falloff, tracking, guidance and information dependencies remain open. No universal random accuracy roll, compulsory manual aiming or terrain collision rule is adopted here.

## G050 — Shield frequency, penetration and counterplay

**Inclusion recommendation: retain readable defensive counterplay; decide whether frequency/modulation earns its own mechanic.** The [weapons baseline](../gdd/mechanics/targeting-weapons.md) already includes frequency behaviour and authored shield pierce. [Shields](../gdd/mechanics/shields.md) currently provides directional protection, focus, collapse and recovery; deeper frequency/modulation controls are **Plan — C4**, not a fully implemented control surface.

[R1's historical feature analysis](./research/feature-gap-analysis-master.md) proposes Science discovering a shield weakness and Tactical matching beam frequency. That is a useful coordination example, but its proposed scan-and-match loop is not evidence that repeated number copying remains interesting. The redesign should test the decision after the weakness is known, not merely count the report as communication.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Preserve distinctions between shield protection, penetration and exposed machinery. Frequency/modulation could be removed, or become a resistance trade-off against observed attacks rather than a single correct number. |
| Activity | Assess what the enemy is using, decide which threats to resist or exploit, and anticipate changes. Routine execution and solved tuning can be delegated to the owning System AI. |
| Interactions | Sensors supplies evidence, Tactical chooses an attack, Shields chooses protection and Helm controls exposed facings. Engineering's resource constraints can affect feasible operation, but exact tuning costs are not adopted yet. |
| Complexity | Simple presets delegate the complete tuning System. Deeper presets expose consequential choices and their effects, without requiring every player to understand a frequency chart. |
| Communication and pillars | Reports matter when they change a decision about attack or protection. Mixed threats can require choosing whose fire to withstand; sharing an already-solved setting should not be the entire job. |
| Risks | Copy-the-number chores, constant retuning races, automatic best-setting buttons that remove the decision, opaque resistance maths and mandatory specialist work during every attack. |
| Dependencies | G049 geometry, G051 solution evidence, G052/G053 engagement/defence, S005 weapon tuning and S021–S026 shielding. Exact spectrum, resistance profiles, retune costs/times and penetration rules remain open. |

### Resolved direction: should frequency/modulation remain a distinct gameplay mechanic?

| Option | Form | Trade-off |
|---|---|---|
| A — Remove frequency matching | Build counterplay from weapon properties, penetration, facing, resources and exposed components; no separate frequency-tuning layer. | Fewer variables and less rote tuning, but loses one possible information-to-defence decision. |
| B — Keep modulation as a trade-off | Tuning can improve resistance to some attacks while weakening protection against others, with meaningful constraints on switching; offensive tuning is reviewed against the same principle. | Creates threat prioritisation and useful reports, but needs legible effects and competent whole-System automation. |

**Selected: user alternative — optional, equipment-dependent shield probing.** A crew can dedicate time to probing shield weaknesses if it has suitable scanners; foes can do the same. This normally provides a small advantage, but special shields or penetrating phasers can make it much more effective. Neither A nor B above was selected as written. In particular, universal resistance trade-offs and switching costs remain proposals, not agreed rules.

The activity is choosing whether a particular investigation is worth the scanner time, then deciding how to use the evidence. It is not mandatory preparation for every attack. Sensors and Tactical have a reason to discuss a consequential finding, while routine execution can be delegated to the relevant individual System AI. Exact scanner eligibility, probing duration, shield/weapon combinations, persistence and defensive response remain open. The preceding activity-card alternatives should be read as candidates subject to this selected direction.

### Resolved dependency: how long is a discovered weakness usable?

| Option | Form | Trade-off |
|---|---|---|
| A — Finding persists for the observed configuration | Complete a probe and retain the finding; an actual shield-configuration change can make it stale. No continuous scan is needed merely to preserve the benefit. | Creates a bounded investigation and frees the scanner for other work, while leaving genuine changes relevant. |
| B — Exploitation needs ongoing probing | Keep suitable scanning active to maintain the exploitable weakness or current solution. | Makes exploitation compete continuously with other scanner work, but can turn a useful discovery into an attention/resource tether. |

**Selected: generally A, with equipment-specific exceptions.** Findings persist for the observed configuration without continuous probing. Some special enemy equipment may rotate shield or phaser frequencies, making renewed assessment useful. This does not require all foes to rotate or all attacks to depend on active scanning. Rotation timing, detection of changes and possible player equipment remain open.

## G051 — Incoming salvo, interception and defence saturation

**Inclusion recommendation: include bounded interception as a candidate combat layer, without requiring a missile-first engagement sequence.** [R1 #38](./research/feature-gap-analysis-master.md) proposes point defence against torpedoes/drones using manual grid aiming. The redesign's whole-System assistant principle instead delegates routine tracking and firing; worthwhile human decisions concern protection, capacity and priorities. [R6's NEBULOUS comparison](./research/comparable-games-and-falling-frontier.md) connects missiles, guidance, electronic warfare and contact information, while HighFleet illustrates that electronic protection can also expose its emitter.

The [current torpedo baseline](../gdd/mechanics/targeting-weapons.md) already provides guided projectiles, tube readiness and in-flight state. It does not establish that the proposed PDC/interception layer is implemented. G027 retains full 3D for torpedoes and small craft alongside planar capital-ship movement.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Incoming attacks have observable travel and supported threat evidence. Interceptors have finite capacity and useful coverage, so saturation follows actual resources/geometry rather than a scripted guaranteed penetration. Exact interceptor weapons and limits remain candidates. |
| Activity | Prepare coverage; decide which threatened asset or incoming threat deserves limited capacity; choose whether to spend reserves, reposition or use another supported defence. Ordinary System AI handles routine tracking/interception. |
| Interactions | Sensors assesses detected threats, Helm changes coverage/escape geometry, Engineering supplies resources and Tactical/defence chooses priorities. Comms can negotiate protection of an ally or civilian. Roles are activities, not an adopted Station roster. |
| Complexity | Simple presets can use an intelligible protection policy; deeper presets expose priorities and reserve/coverage choices. No compulsory manual aiming at every missile. The same AI knowledge and physical limits apply at every preset. |
| Communication and pillars | Defence should enable an operation or protect something the crew values. If coverage can extend to others, escort position and resource requests become useful shared decisions. |
| Risks | Passive perfect protection, incoming spam, opaque saturation, mandatory twitch aiming and a fixed missile→PDC→broadside ladder. Ensure the crew can understand why defence succeeded or failed. |
| Dependencies | G048 assignments, G049 geometry, G052 broadside, G053 engagement choice, G105 information/occlusion and S008/S018–S020 defensive equipment. Exact capacity, target classes, ammunition, costs and ownership remain open. |

### Resolved choice: can interception protect other ships?

| Option | Form | Trade-off |
|---|---|---|
| A — Primarily self-defence | Interception automatically focuses on attacks threatening the firing ship; escorts help mainly by attacking or distracting the source. | Simple protection policy, but fewer direct defensive choices around allies and civilian operations. |
| B — Protection within actual coverage | Interceptors can engage supported incoming threats to other assets when detection, range and geometry permit. Protection priorities can compete with the ship's own safety. | Enables escort and fleet cooperation, but requires legible coverage and priorities when capacity is insufficient. |

**Selected: B.** Interceptors can engage supported incoming threats to other assets when detection, range and geometry permit. Limited capacity makes protection priorities compete with own-ship safety; ordinary System AI handles routine tracking/interception. Exact equipment, capacity, costs and authority remain open. No separate mandatory defence Station, remote shield protection or unlimited shared sensor knowledge is adopted.

## G052 — Broadside / close-pass combat geometry

**Inclusion recommendation: support consequential aspect-dependent attacks; decide how widely broadside play defines ships.** The [current weapons baseline](../gdd/mechanics/targeting-weapons.md) already has authored bank/tube arcs and bearing requests to Helm. [R6's Rebel Galaxy comparison](./research/comparable-games-and-falling-frontier.md) combines capital-ship broadsides with automated turrets, suggesting that human positioning and delegated gunnery can coexist. It does not by itself establish an engaging six-player bridge loop.

G049 already adopts graduated weapon-dependent effectiveness. Broadside design should add a distinctive opportunity and exposure choice where equipment supports it, rather than duplicate that decision under another name. G027's capital-ship movement remains planar; missile→PDC→broadside remains an optional hypothesis, not a prescribed sequence.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Support side-facing batteries and useful close passes through authored equipment/geometry. Review whether this is a common reference combat pattern or one hull/loadout style among several. |
| Activity | Choose a pass, time a useful commitment, judge the exposed facing and retain an escape option. Ordinary weapon-System AI handles routine aiming and firing against authorised targets. |
| Interactions | Tactical explains the firing opportunity; Helm owns the course; Shields assesses the facing; Engineering prepares supported demand; Sensors supplies relevant evidence. No Captain approval is needed for each shot. |
| Complexity | Simple presets retain target and positioning decisions with delegated gunnery. Deeper presets can expose supported bank priorities or timing; no compulsory manual broadside aiming or separate charging player. |
| Communication and pillars | Coordinate an opportunity whose attack advantage has a cost in protection, resources or another objective. Preserve Helm's ability to choose a different approach. |
| Risks | Every hull becoming the same broadside ship, circling as a solved manoeuvre, fixed engagement phases and a firing window that occupies only Helm/Tactical while others wait. |
| Dependencies | G048 targeting, G049 effectiveness, G051 interception, G053 range transitions, shields/power and S007 batteries. Exact weapon statistics, arcs, charge and firing controls remain open. |

### Resolved choice: how central should broadside passes be across ship designs?

| Option | Form | Trade-off |
|---|---|---|
| A — Common reference combat pattern | Most combat hulls have important side-facing batteries, making coordinated broadside passes a common combat expectation; specialised hulls vary from it. | Gives crews a recognisable shared geometry, but narrows ship diversity and can make encounters converge. |
| B — Equipment-dependent combat style | Broadside passes are important for suitably equipped hulls, alongside forward-fire, stand-off, carrier, support or other coherent styles. | Supports varied encounters and crew plans, but requires each hull's opportunities to be readable. |

**Selected: B.** Broadside passes are important for suitably equipped hulls, while other designs favour different positions and tactics. Ordinary weapon-System AI handles routine gunnery; Helm owns the manoeuvre. Exact batteries, arcs and firing controls remain open. Specific alternative hull classes remain candidates, not adopted feature commitments.

## G053 — Layered engagement ranges and transitions

**Inclusion recommendation: retain emergent range choices, without prescribing phases.** G049–G052 already establish weapon-dependent effectiveness, interception and equipment-dependent broadside opportunities. A crew can prefer different distances for different reasons; the rejected mandatory missile→PDC→broadside sequence does not need another vote.

The [movement baseline](../gdd/mechanics/movement.md) provides authored acceleration/speed/turn response, tactical boost and committed impulse transit. Power, damaged machinery and hazards affect achievable movement. G044 already makes pursuit depend on an actor's objective, knowledge and capability rather than a universal chase leash. These are the starting point for range control, not an abstract combat-mode switch.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Let equipment, movement, terrain, resources and objectives create reasons to remain at range, close, retreat or protect a position. Review whether specialised equipment can also interfere directly with disengagement. |
| Activity | Compare current advantage with the cost of changing distance; prepare resources/route; decide whether to pursue, break contact or disrupt an opponent's range control. Ordinary System AI executes delegated equipment actions within actual knowledge/authority. |
| Interactions | Sensors reports capability and observed intent, Tactical assesses attack opportunities, Helm chooses movement, Engineering supports the manoeuvre and Comms identifies objectives or alternatives. Captain coordinates overall priorities through the existing command role. |
| Complexity | Simple presets keep course/target/priority choices readable; deeper presets expose relevant readiness and equipment options. No compulsory multi-control combo to cross an arbitrary engagement boundary. |
| Communication and pillars | A range change should have reasons and costs other players can affect: escape route, remaining reserves, protection of a civilian or an opportunity to disable pursuit. |
| Risks | Endless kiting, automatic escape denial, hidden combat locks, every fight converging on one range and objectives that make every withdrawal a failure. |
| Dependencies | G027 movement, G044 pursuit, G049–G052 combat layers, later propulsion/tractor/EWAR Systems and travel entries. Specific equipment, reach, effects, counters and transit compatibility remain open. |

### Resolved choice: what can prevent a ship from disengaging?

| Option | Form | Trade-off |
|---|---|---|
| A — Movement, damage and objectives | Relative movement, terrain, ordinary machinery damage and the cost of abandoning an objective determine disengagement; no dedicated interdiction effect. | Coherent basic rules and fewer special counters, but a faster ship can dominate distance where objectives do not constrain it. |
| B — Also equipment-based interdiction | Suitable equipment can interfere with propulsion or transit under explicit operating limits, with ways to evade, disrupt or disable it. | Creates coordinated pursuit/escape decisions, but needs readable counterplay and must not become automatic combat imprisonment. |

**Selected: B.** Suitable specialised equipment can interfere with propulsion or transit under explicit operating limits, with ways to evade, disrupt or disable it. This supplements movement, damage and objective constraints and applies the same physical/knowledge rules to both sides. Specific equipment, effects, costs and counters remain open. No universal combat lock, particular tractor effect or automatic transit cancellation is selected.

## G054 — Finite ammunition, launch commitment and fabrication

**Inclusion recommendation: retain finite stocks and launch commitments; review onboard replenishment as an activity rather than assume the existing Plan is adopted.** The [current weapons baseline](../gdd/mechanics/targeting-weapons.md) uses shared torpedo magazines and individual tube reload state. A projectile's damage/pierce state is captured at launch. **Plan — C5** adds fabrication and payload sidegrades through Tactical calibration, with available payloads loaded into tubes and behaviour frozen at launch. The [roadmap baseline](../gdd/future/roadmap-baseline.md) records this intended scope.

This gives a useful before/after comparison: current magazine endurance versus a possible production loop that prepares a different ammunition mix during the mission. Neither finite ammunition nor fabrication should require repetitive reload clicks. R6's [Battlestar Galactica Deadlock comparison](./research/comparable-games-and-falling-frontier.md) supports preparation/commitment/observation as a design rhythm, without copying turn-based or paused play.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Finite ammunition makes launches consequential. Where supported, payload variants should offer distinct effects rather than a universal heavier-is-better upgrade. Decide whether onboard production is a substantive scenario activity or replenishment remains mainly external. |
| Activity | Choose a launch worth its stock cost, prepare a useful payload mix and forecast endurance. If fabrication is included, choose production priorities and when limited throughput/resources are worth spending. |
| Interactions | Sensors' evidence can change useful payloads, Tactical judges expenditure, Engineering supports power demand and stores/acquisition activities supply material. Exact Station ownership follows the later coherent-bundle review. |
| Complexity and AI | Whole-System AI can load/reload routinely and fabricate to desired stocks/priorities if that capability is selected. Humans need not click every round or queue entry. Detailed payload/production decisions are exposed only where useful at the chosen preset. |
| Communication and pillars | Reports can change the next batch rather than magically alter an in-flight attack. Requests for replenishment/resources should have a real alternative or competing use. |
| Risks | Infinite missile spam, production that erases scarcity, repetitive reloads, a mandatory pre-mission spreadsheet and material accounting that becomes the main job of every Station. |
| Dependencies | G051 interception, G055 power, G065/G066 stores/acquisition, S004 torpedoes, S010–S014 payloads/fabrication. Exact ammunition types, recipes, throughput, stock capacities and shared material pools remain open. |

### Resolved choice: how should ammunition be replenished during a scenario?

| Option | Form | Trade-off |
|---|---|---|
| A — Mainly carried stocks and external resupply | Load finite magazines and replenish through supported docking, transfer or recovery; onboard fabrication is omitted or secondary. | Clear endurance and supply objectives, but fewer ways to adapt the ammunition mix while underway. |
| B — Also meaningful onboard fabrication | Suitable ships can manufacture supported ammunition during the scenario, consuming finite material, power and production time. | Adds preparation and resource choices, but production must not eliminate scarcity or become queue maintenance. |

**Selected: B, plus starbase restocking.** Suitable ships can manufacture supported ammunition during a scenario using finite materials, power and production time; ammunition can also be restocked at starbases. Ordinary System AI can produce to desired stock priorities. Exact recipes, payload catalogue, throughput, capacities and whether materials compete directly with repair remain unresolved. Starbase stock/access/transfer limits follow later service/logistics reviews.

**Architecture refinement — [AD62](./architecture/index.html#recipe-output-blockage):** fabrication waits before a production step whose required stored outputs cannot all fit. A full output store does not automatically discard products. AD63 separately discards unfinished work when deliberately switching recipes, retains unused inputs and completed products where compatible storage permits, and disposes of excess that cannot be retained. Moving retained stock still requires ordinary physical logistics; temporary output blockage does not trigger disposal.

## G055 — Power / battery / coolant / heat economy

**Inclusion: already selected as a central ongoing activity; resolve remaining dependencies without reopening the agreed power model.** The [current baseline](../gdd/mechanics/power-resource-network.md) uses three commanded groups and a ship-wide battery; **Plan — C4** adds component supply networks, local heat, cooling and graceful local degradation. The redesign instead already selects flow into each System's local battery, fullness-dependent passive drain, and heat from charging and depletion counted separately with System thermal modifiers. Stable preparation provides crisis margin; Engineering owns allocation and listens to other operators' changing needs.

The earlier [research comparison](./README.md#research-comparison) distinguishes EmptyEpsilon's limited simultaneous coolant allocation from Elite Dangerous's grouped recharge capacitors. Neither establishes Phoenix's fullness-dependent drain or heat rule. Retain those as explicit proposal decisions, not borrowed mechanics.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Continuous competing flow/cooling needs with readable stored margin and thermal limits. Full local batteries are inefficient; preparing them in advance can nevertheless be worthwhile. Healthy ships cannot sustain every System at full output indefinitely. |
| Activity | Listen, anticipate demand, prepare reserves, rebalance sustainable operation and ration during high demand. Stable operation must allow enough unattended margin to handle a crisis elsewhere. |
| Interactions | Other operators request capability or describe their intentions; Engineering chooses allocations and communicates limits. Increased flow is preparation/support, not remote control of the consuming System. |
| Complexity | The user's queued ladder remains provisional: maximum dozens of individually allocated groups; middle several allocated groups; minimum a group priority list plus no/low/high coolant usage with AI distribution; AI manages all. Exact grouping, interfaces and internal allocation need further review against the same shared physical model and individual System AI. |
| Communication and pillars | Ongoing listening supports other players; scarcity makes requests and alternatives consequential. No Captain allocation dashboard or requirement that every adjustment receive approval. |
| Risks | Perfect-balance chores, full reserves as an always-best setting, crisis overload, hidden consumers, and simplified controls quietly granting different physical capacity. |
| Dependencies | Coolant model, thermal failure/recovery, local empty-battery effects, supply topology and S031–S039. Exact coefficients, passive-drain thermal treatment and control grouping remain unresolved. |

### Resolved choice: is normal cooling limited by throughput or by consumable stock?

| Option | Form | Trade-off |
|---|---|---|
| A — Reusable circulation | Normal cooling reuses coolant; circulation and heat rejection have finite capacity. Leaks or exceptional equipment may lose coolant if later adopted. | Supports an ongoing allocation balance without automatic stock depletion; heat still has to be rejected. |
| B — Routine consumption | Cooling normally spends finite coolant stock, requiring production or restocking as well as allocation. | Adds endurance and logistics decisions, but can make ordinary operation a recurring supply chore. |

**Selected: B — consumable coolant.** Normal cooling spends finite coolant stock. The user additionally proposes, provisionally, a System that recycles used coolant at a loss. This could extend endurance while retaining net consumption; it is not accepted as a definite capability. Exact stock/replenishment, collection of used coolant, recovery yield, throughput, power demand and thermal failure rules remain open for S031–S039 and stores review. No fluid simulation or radiator hardware is adopted. The queued reactor complexity ladder remains provisional for the detailed System review.

## G056 — Combat load / flux / local energy endurance

**Inclusion recommendation: retain actual energy/thermal endurance; avoid an additional abstract meter unless it creates a distinct decision.** G055's selected model already couples allocation, charging/depletion heat, local reserves, inefficient full batteries and consumable coolant. It can make sustained pressure and a prepared burst different choices without a further ship-wide combat resource.

[R6's Starsector comparison](./research/comparable-games-and-falling-frontier.md) identifies overload linking offence/defence and recovery as an inspiration, but the official historical manual was only partially verified; exact current flux rules are not established here. The earlier Elite Dangerous manual comparison provides grouped energy capacitors as another endurance approach. These are alternatives to compare against Phoenix's selected physical resources, not a reason to accumulate all their meters.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Use observable energy, heat and coolant state to explain sustainable operation, reserve expenditure and recovery. Decide whether any distinct abstract combat-load meter is necessary. |
| Activity | Forecast how long pressure can continue, choose a burst or recovery interval and coordinate demands. Routine execution is delegated to the owning System AI; the human chooses priorities and accepted risk. |
| Interactions | Tactical/defence/Helm can compete for actual supply and cooling, with Engineering setting allocation. Coupling should follow explicit consumers and constraints rather than a hidden generic penalty for being in combat. |
| Complexity | Concise endurance forecasts may summarise several actual constraints without introducing a new resource. Deeper presets can inspect causes in the same physical simulation. |
| Communication and pillars | Reports about reserves and recovery should change another player's plan. An additional meter must justify a distinct decision, not merely add a report or emergency button. |
| Risks | Duplicate resources, unclear overload causes, one universal attack/vent rhythm and forcing another mandatory monitoring task onto Engineering. |
| Dependencies | G055, S032/S034/S036/S037 and later thermal failure/recovery controls. No exact venting, overload or lockout mechanic is adopted here. |

### Resolved choice: does the ship need a separate combat-load/flux meter?

| Option | Form | Trade-off |
|---|---|---|
| A — Existing physical resources only | Local batteries, heat and coolant determine endurance; summaries/forecasts expose those constraints without an extra resource. | Fewer overlapping variables and clear causal explanations, though resource tuning must create useful attack/recovery choices. |
| B — Add a distinct combat-load resource | A further meter couples specified offensive/defensive activity to overload and recovery independently of actual battery/heat/coolant state. | Can impose a clear shared combat rhythm, but risks duplicating the selected endurance model. |

**Selected: A.** Local batteries, heat and coolant carry endurance and recovery decisions, with forecasts/summaries exposing those actual constraints. No separate abstract combat-load/flux meter is included. Exact thermal failure/recovery behaviour remains for the relevant System reviews; no universal vent/overload rhythm is adopted.

## G057 — Granular damage and capability loss

**Inclusion recommendation: retain consequential capability loss; review the user's component-condition proposal as a replacement for the present material model.** The [current damage baseline](../gdd/mechanics/damage-repair.md) distributes leaked damage to fine System hull, with percentage/threshold-based degradation, disablement and destruction. Operators know their own machinery; Engineering's remote detail depends on team diagnosis. The proposed redesign changes what is damaged, not networking or System AI knowledge limits.

The user's [queued proposal](./README.md#component-conditions-repair-parts-and-inventory) instead gives Systems components with conditions: reactor links exemplify separate components, with worn links losing efficiency, damaged links supplying intermittently, EMP temporarily stopping supply without repair, and destroyed links supplying nothing. These examples remain provisional until selected below. [R6's NEBULOUS comparison](./research/comparable-games-and-falling-frontier.md) supports component damage and repair teams as a research direction; it does not establish these particular conditions or a percentage-free model.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Damage should change a specific capability or operating choice. Compare present System hull/tiers with component conditions that describe distinct failures rather than uniformly reducing performance. |
| Activity | Recognise what operation changed, choose a workaround or reduced operating mode, report useful evidence and decide which restoration matters first. Team/parts execution is reviewed at G058 rather than adopted automatically here. |
| Interactions | Local operators explain symptoms/capability; Engineering prioritises investigation/restoration; other officers alter plans to fit the remaining ship. G035 already requires facilities to share the ship shield/System/component damage direction. |
| Complexity and AI | Simple presets can present actionable aggregate capability while ordinary System AI handles delegated operation. Deeper presets can expose known component conditions. Simplification must not grant repair knowledge or change the physical damage model. |
| Communication and pillars | Distinct failures give reports operational meaning: intermittent supply differs from efficiency loss or temporary EMP. Avoid forcing transcription of every component merely to request help. |
| Risks | Too many tiny faults, unclear intermittent behaviour, component lists becoming the whole game, and replacing one percentage bar with several redundant bars. |
| Dependencies | G058 diagnosis/repair, G059 propagation, G060 maintenance, parts/stores, S021–S030 protection and S070–S078 diagnosis/restoration. Condition combinations, hit routing, severity, recovery and defeat criteria remain unresolved. |

### Resolved choice: what should be the primary damage representation?

| Option | Form | Trade-off |
|---|---|---|
| A — System hull percentages and tiers | Keep numerical System damage, with authored thresholds producing degraded, disabled or destroyed capability; add specific effects sparingly. | Familiar and compact, but different failures can collapse into the same repair-the-bar decision. |
| B — Component conditions | Adopt the proposed component-level failure model, with meaningful conditions replacing the primary System percentage-damage model. | Supports distinct workarounds and restoration choices, but needs a manageable component topology and clear condition effects. |

**Selected: B.** Component conditions replace the primary System percentage-damage model. Failures change a specific capability and create different workarounds/restoration priorities; individual reactor links illustrate component granularity. Exact topology, condition effects/severity and combinations, damage routing, repair requirements and defeat criteria remain unresolved. This selects the model direction without adopting every queued maintenance or parts detail.

**Architecture refinements — AD44, AD47 and AD49–50:** compartments own structural HP; individual damage events apply status effects to components through authored rules using deposited damage amount/type and component susceptibility. Small hits need not cause faults; crossing structural HP thresholds is not the primary trigger. Components retain individual conditions and repair targets without separate HP pools. This does not restore System percentage-damage bars. AD49 adds a distinct compartment-sourced disabling condition at zero HP; restoring the compartment removes it while unrelated component conditions remain. AD50 selects recovery at a modest authored HP fraction above zero and below full HP; after recovery, the condition reapplies only at zero HP. Exact effect weights, structural work rules and threshold tuning remain open. See the [damage ownership model](./architecture/index.html#compartment-hp).

## G058 — Diagnosis, triage, team travel and restoration

**Inclusion recommendation: retain team-based diagnosis/triage and component restoration; decide whether restoration needs typed parts.** The [current baseline](../gdd/mechanics/damage-repair.md) gives local operators knowledge of their machinery while Engineering needs an on-site team for exact remote detail. Teams travel, repair and return; Engineering can prioritise revealed internal Systems. **Plan — C4** adds symptom-to-order diagnosis and material depth. G057 now selects component conditions, so diagnosis and repair must address the actual failed components rather than merely refill a System health bar.

[R6](./research/comparable-games-and-falling-frontier.md) offers complementary references: Guns of Icarus links restoration to colleagues' operational choices; Barotrauma illustrates symptoms with several possible causes; Keep Talking and Nobody Explodes makes differing information consequential. These support interpretation and triage, not mandatory repair hammering, copied codes or a puzzle for every fault.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Use the shared team framework selected at G037; teams investigate and restore affected components. Review parts as a restoration constraint, with diagnosis visibility and detailed orders resolved separately. |
| Activity | Judge urgency, choose investigation/repair priorities, route limited teams and decide whether restoration is worth its time/resources or a workaround is sufficient. |
| Interactions | Local operators report useful symptoms and operational needs; Engineering chooses team work; stores/acquisition can supply necessary parts if selected. Other officers adapt operations while work continues. |
| Complexity and AI | Ordinary System AI executes delegated diagnosis, dispatch and repair within the same knowledge and stock limits. Simple presets can delegate complete Systems; deeper presets expose consequential priorities without compulsory manual repair gestures. |
| Communication and pillars | Reports should distinguish a harmless temporary outage from a failure worth diverting a team. Resource scarcity can create alternatives such as restoring sensing before a weapon, rather than making every request immediately satisfiable. |
| Risks | Engineering becoming a universal exact-damage dashboard, idle team travel, recipe memorisation, excessive part types and unavoidable dead ends after the first missing spare. |
| Dependencies | G057 conditions, G059 containment, G060 maintenance, G065/G066 stores/acquisition and S075–S078. Exact parts, substitution, repair/replacement, diagnosis visibility, team travel and recovery from destroyed components remain open. |

### Resolved choice: should component repairs consume typed parts?

| Option | Form | Trade-off |
|---|---|---|
| A — Mainly team time | Restoration primarily consumes available team time; omit typed repair parts as a central constraint. | Keeps triage compact, but fewer restoration choices connect to stores and recovery. |
| B — Teams plus appropriate parts | Material-requiring repairs consume suitable parts, with several useful types; teams perform the work and AI can handle delegated execution. | Connects repair priorities to preparation and acquisition, but needs manageable categories and alternatives when stock is short. |

**Selected: B.** Teams restore individual components, with material-requiring repairs consuming appropriate typed parts. Ordinary System AI executes delegated work within actual knowledge, team and stock constraints. Exact categories, recipes/quantities, substitution and repair-versus-replacement remain unresolved. Temporary EMP recovery does not itself establish a repair task. This adopts restoration/parts direction without automatically adopting a standalone inventory System or maintenance rules.

### Resolved dependency: what does an operator know before diagnosis?

The current baseline gives a local Station exact detail for its own Systems, while Engineering needs a team on site for exact remote detail. A component-condition model could retain that boundary, or distinguish operating evidence from a confirmed internal fault. The research comparison with Barotrauma supports symptoms with several possible causes; Keep Talking and Nobody Explodes supports complementary knowledge, but neither requires a memorised diagnosis puzzle in Phoenix.

| Option | Form | Trade-off |
|---|---|---|
| A — Exact local fault information | Local operators see exact component conditions in their own machinery; Engineering uses their reports or on-site teams to learn remote detail. | Keeps strong local expertise and quick reports, but diagnosis is mainly an information-access step. |
| B — Local evidence, diagnosis where needed | Operators see relevant readings, symptoms and known faults; ambiguous causes require supported tests or on-site investigation before becoming confirmed conditions. | Adds interpretation and meaningful investigation choices, but must not make every obvious fault a delay or compulsory puzzle. |

**Selected: B.** Operators see relevant readings, symptoms and reliably detected faults; ambiguous causes require supported tests or on-site investigation before becoming confirmed component conditions. Obvious faults need not be hidden or artificially delayed. Ordinary System AI can execute delegated diagnosis using the same evidence. Exact test controls and reporting/information sharing remain unresolved; no compulsory diagnosis puzzle is adopted.

## G059 — Fault propagation, isolation and containment

**Inclusion recommendation: use causal dependencies with meaningful containment; decide whether faults can cause additional material damage.** G036 already selects automatic dependency ripples, G055 connects energy/cooling to operation, G057 gives failures component-level effects and G058 makes diagnosis evidence-led. Reopening whether consequences are systemic would repeat an agreed decision. The remaining question is how far an unresolved fault can escalate.

The [C4 resource-network baseline](../gdd/mechanics/power-resource-network.md) provides producers, conduits, buffers and consumers as intended foundations, not every possible cascade or containment control. [R6's Barotrauma/Stormworks comparisons](./research/comparable-games-and-falling-frontier.md) suggest preserving partial operation through causal interventions; they do not require walkable interiors, fluid simulation or copied wiring controls.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Supported dependencies propagate consequences automatically. Isolation or a bypass can preserve useful operation where the authored topology allows it; exact machinery/controls remain candidates. Decide whether escalation can create new component damage rather than only interrupt service. |
| Activity | Identify what is observed/suspected, choose whether to isolate now or accept risk to keep operating, route teams and restore a safe path. A prepared reserve should provide intervention time where the failure permits it. |
| Interactions | Local operators report symptoms and changing capability; Engineering weighs supply/containment against ongoing demand; other officers adapt plans while teams investigate. Isolation has a cost in unavailable service, not merely a free clear-fault button. |
| Complexity and AI | Ordinary System AI can execute delegated emergency/containment policies using permitted evidence. Presets simplify decisions and presentation without secretly preventing physical consequences. Cross-System action follows existing authority/communication, not a new omniscient repair AI. |
| Communication and pillars | A containment request can conflict with a needed weapon, scan or escape manoeuvre. Players should understand what continuing operation risks and what isolation sacrifices. |
| Risks | Arbitrary chain reactions, instant whole-ship loss, one overloaded Engineer, auto-isolation that removes every human decision and incidents whose causes cannot be investigated. |
| Dependencies | G055 resources, G057 conditions, G058 evidence/teams, G060 maintenance and S028/S029/S034/S075–S078/S102. Exact topology, containment authority, warnings, time margins and bypass effects remain open. |

### Resolved choice: can an unresolved fault damage further components?

| Option | Form | Trade-off |
|---|---|---|
| A — Mainly service loss | Dependencies propagate reduced or lost capability, but faults do not generally create further material damage. | Easier to bound and recover, but containment has fewer reasons beyond restoring service. |
| B — Causal damage escalation | Specific authored causes can damage further components if left unresolved; supported isolation/containment can prevent or limit that escalation. | Creates consequential crisis choices, but requires readable evidence, intervention opportunities and recovery paths. |

**Selected: B, dependent on the damage/debuff.** Some conditions can escalate or spread through supported causal rules; others remain stable and are safe to leave untreated. Supported isolation/containment can prevent or limit relevant escalation. Exact conditions, causal paths, warnings, intervention margins and controls remain open. No universal contagion or arbitrary cascade budget is adopted; consequences follow the underlying supported causes.

## G060 — Preventive maintenance and degradation

**Inclusion recommendation: support prevention only where it creates a worthwhile operating choice; decide what creates wear.** The user's component proposal includes worn components losing efficiency and requiring maintenance, but G057's adoption of component conditions did not adopt routine ageing rules. G059 now distinguishes escalating/spreading conditions from stable ones, allowing sensible deferral rather than requiring every debuff to be cleared immediately.

[R1 #33](./research/feature-gap-analysis-master.md) records time-based degradation as a historical research candidate attributed to Starship Simulator. That is not a fresh verification of its current implemented mechanics. The useful Phoenix comparison is whether a maintenance task changes a decision about risk, output, scarce teams or parts, rather than repeatedly resetting a timer.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Compare no routine wear, operating-condition-dependent wear and routine time-based ageing. Maintain the same underlying wear model across presets, with ordinary System AI able to execute delegated maintenance. |
| Activity | Assess a developing loss/risk, choose whether to reduce stress or schedule work, and decide whether continued operation is worth deferring maintenance. |
| Interactions | Local operators report capability/usage; Engineering weighs teams, restoration and requested output; stores supplies relevant parts where required. Routine healthy cruise need not create work simply to occupy a seat. |
| Complexity and AI | Simple presets delegate maintenance execution and present useful consequences/priorities. Deeper presets can expose observed causes and deferral choices. No compulsory repeated reset inputs. |
| Communication and pillars | A request for a quieter operating interval or reduced output should have a reason another operator can understand and weigh against their objective. |
| Risks | Chore clocks, maintaining every component equally, hidden wear from ordinary inputs and overload from maintenance competing with diagnosis, power and repair. |
| Dependencies | G055 thermal/resource model, G057 conditions, G058 teams/parts, G059 escalation, G110 manoeuvre-related strain and S078. Exact thresholds, ageing rates, maintenance methods and parts costs remain open. |

### Resolved choice: what should create maintenance needs?

| Option | Form | Trade-off |
|---|---|---|
| A — Damage only | No routine wear model; preventive work concerns known damage and its possible escalation. | Keeps workload focused, but removes an operating-cost choice before damage occurs. |
| B — Stress-dependent wear | Hard use, overheating or particular adverse conditions can create wear; healthy ordinary operation does not inevitably produce a maintenance timer. | Connects prevention to chosen risk and operating conditions, but requires legible causes and sensible rates. |
| C — Routine ageing too | Components also wear with ordinary use/time, producing recurring scheduled maintenance alongside stress/damage. | Supports a broader upkeep/endurance loop, but risks predictable chores in quiet periods. |

**Selected: B.** Hard use, overheating or particular adverse conditions can create wear. Healthy ordinary operation does not inevitably produce a recurring maintenance timer. Crews can anticipate/influence wear through operating choices and delegate routine maintenance execution to ordinary System AI. Exact stress sources, thresholds, rates, methods and parts requirements remain open for System review.

## G061 — Protected external operations under threat

**Inclusion recommendation: retain physical external work as a purpose for coordinated protection; review withdrawal risk.** The [current external-operations baseline](../gdd/mechanics/external-operations.md) uses crew-owned tractor, dock, umbilical and repair Systems. Coupling, actual transferred resources and repaired target state determine outcomes; there is no separate generic progress simulation. Interruptions belong to those physical Systems, and resources already transferred remain transferred.

[R6's Stormworks/Foxhole comparisons](./research/comparable-games-and-falling-frontier.md) suggest rescue conditions and threatened deliveries can change the method or feasible plan. G051 already allows interception protecting another asset; G032 retains manual steering alongside Match Speed. External work can therefore give combat, sensing, Comms and resource decisions a shared non-combat purpose without requiring a GM.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain tow, stabilise, transfer and supported field repair as chains of ordinary Systems. Protection can concern hazards as well as enemies. Review whether emergency break-off is usually safe or can carry equipment/state-specific risks. |
| Activity | Assess need and conditions, choose a method/working position, prepare capacity and protection, sustain the operation or decide to suspend/withdraw. Ordinary System AI handles delegated execution. |
| Interactions | Sensors assesses threats/target condition, Comms establishes needs/access, Helm owns positioning, Engineering supplies power/cooling and machinery, repair teams commit real capacity and defence protects supported assets. Exact Station bundles remain open. |
| Complexity | Simple presets delegate complete external-work Systems while retaining target/route/priorities. Deeper presets expose meaningful constraints and withdrawal preparation, without repeated keep-alive inputs. |
| Communication and pillars | Operators report how long a position or resource commitment is needed, what can safely stop and what is exposed. A colleague can propose a different method rather than merely wait for completion. |
| Risks | One player performs the whole operation, everyone else waits, generic progress resets, automatic perfect coupling and danger used only to punish interruption. |
| Dependencies | G032 relative speed, G035–G037 targets/teams, G051 protection, G055 resources, G058 repair, later teams/craft and S049–S054/S083–S093. Exact interfaces, release procedures and physical hazards remain open. |

### Resolved choice: how risky can emergency break-off be?

| Option | Form | Trade-off |
|---|---|---|
| A — Usually a safe pause | Breaking off normally stops supported work safely; urgency comes mainly from delay and the target's independent situation. | Forgiving and easy to understand, but fewer preparation/withdrawal decisions. |
| B — Equipment/state-dependent withdrawal risk | Some operations need preparation to withdraw safely; hurried break-off can have a supported physical consequence, while others stop safely. | Adds consequential commitment and crew coordination, but hazards must be readable and must not reset completed work arbitrarily. |

**Selected: B, principally through deployed crew/equipment recovery.** Operations that commit crew or equipment normally need to recover them; hurried withdrawal can leave a recovery problem. A tractor beam deploys neither and is safe to release in this respect. Its target may still face independent danger, but no generic recovery penalty is added to beam release. Exact recovery methods, abandonment consequences and material commitments remain for personnel/craft/equipment review. Actual transferred/repaired state persists; no universal interruption penalty is adopted.

## G062 — Asymmetric hulls, factions and doctrines

**Retained under already agreed constraints; no repeat question.** The [ship design baseline](../gdd/systems/ships-and-systems.md) authors distinct equipment, handling and AI policies under common System/control rules. G039 already selects recognisable doctrine plus bounded actor variation; G044 makes pursuit objective-dependent; G050 allows special shield/weapon interactions; G052 selects equipment-dependent positional styles. Together these answer the catalogue's core inclusion question: variants should change the crew's choices through supported capability and behaviour, not only numerical durability/damage.

Sensors/Comms interprets evidence, Tactical/Helm chooses an approach and Engineering/defence supports the resulting demands. Presets and ordinary per-System AI remain on the same physical model, with no hidden stat bonuses or omniscient faction planner. Specific hulls, faction equipment availability and coherent Station bundles remain for later reviews. No new faction-exclusive mechanics or fixed class roster is adopted here.

## G063 — Multi-bridge cooperation and competition

**Inclusion recommendation: retain independently active bridges and the existing networking; review how objectives create useful interaction.** The catalogue records current escort/elimination and planned multi-bridge acceptance. G022/G024 already select optional faction radar sharing, shared human/AI Comms channels and contextual requests, without remote command of another human crew. G051 supports protection within actual interception coverage; G061 supplies operations worth protecting.

[R6's Carrier Command 2/Foxhole comparisons](./research/comparable-games-and-falling-frontier.md) provide research transfers around remote operations, supply routes and delivery consequences. The Phoenix proposal is to use bounded objective dependencies to make cooperation worthwhile, not copy an RTS commander or persistent logistics workload.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Each bridge needs worthwhile ongoing and active decisions. Review whether allied bridges mainly repeat a common task or also receive complementary objectives that interact. PvP can use opposed goals under the same rules. |
| Activity | Decide division of effort, exchange relevant evidence, request protection/resources and revise plans when another bridge's situation changes. Preserve independent local priorities and actuator authority. |
| Complexity and AI | Individual System AI supports each ship at its selected presets and responds through ordinary orders/objectives/Comms. No central omniscient fleet AI or new transport architecture. |
| Communication and pillars | Cooperation should have a purpose: one operation changes what another can do. Sharing a feed remains optional, and useful discussion should not require a universal fleet dashboard. |
| Risks | A support bridge waiting for another crew, rigid roles that become inactive after one task, one Captain directing all ships, and cooperative content requiring several human bridges to function. |
| Dependencies | G022/G024 sharing/Comms, G037 teams, G051 protection, G061 external work and later scenario/PCG/fleet-operation reviews. Exact objectives, role assignment and scaling remain open. |

### Resolved choice: how should cooperative objectives distribute work across bridges?

| Option | Form | Trade-off |
|---|---|---|
| A — Mainly common parallel objectives | Allied bridges pursue largely the same task, choosing their own division of effort; special dependencies are occasional. | Flexible and easy to substitute ships, but cooperation may amount mostly to adding firepower. |
| B — Also complementary interacting objectives | Provide substantive objectives that help or constrain one another, while every bridge retains useful work and room to change its approach. | Creates purposeful communication and different decisions, but requires avoiding bottlenecks and inactive support crews. |

**Selected: B.** Include complementary interacting objectives alongside shared tasks, with each bridge retaining worthwhile work, local authority and freedom to change its approach. Outcomes can help or constrain another bridge's options. Ordinary individual System AI supports missing human crews; exact objectives and scaling remain open. No fixed mandatory support role, central fleet command or network redesign is adopted.

## G064 — Disabled ships, rescue and loss continuation

**Inclusion recommendation: retain recovery as useful external work and active participation after loss; resolve field restoration limits.** G012 already selects escape pods and managing teams without a mothership, with vacant seats another situational option. G046 selects actual surviving salvage; G058 selects teams/typed parts; G061 requires recovery of deployed crew/equipment. These decisions should not be reopened merely because a ship is disabled or lost.

The [current damage baseline](../gdd/mechanics/damage-repair.md) does not restore destroyed/non-repairable targets unless authored rules explicitly allow it. The [external-operations baseline](../gdd/mechanics/external-operations.md) commits real teams to field repair, while towing and transfers remain ordinary physical Systems. [R6's Falling Frontier research](./research/comparable-games-and-falling-frontier.md) advertises escape-pod rescue/capture, but that developing game's description is not playtest evidence or a requirement to copy its loss rules.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Disabled vessels can need protection, supplies, towing or component restoration. Distinguish a recoverable vessel from actual ship loss; exact defeat/recoverability criteria remain open. Review whether suitable destroyed components can be replaced in the field. |
| Activity | Assess whether recovery is feasible/worthwhile, choose field work versus towing/evacuation, commit teams/equipment/parts and protect their recovery. Displaced players retain G012's legitimate survivor/team participation. |
| Interactions | Sensors assesses supported condition/threat evidence, Comms establishes needs/access, Helm positions/tows and Engineering/teams manage feasible restoration. Other ships can provide G051 protection or G063 complementary work. |
| Complexity and AI | Ordinary System AI executes delegated rescue/restoration within actual knowledge, tools, parts and authority. No hidden completion shortcut or automatic omniscient distress response. |
| Communication and pillars | Rescue should produce useful decisions for both assisting and affected crews; neither bridge becomes a passenger while one team timer runs. |
| Risks | Early loss excluding players, universal instant resurrection, repair dead ends from missing a unique part, towing as prolonged inactivity and salvage silently becoming recovery of an irretrievably destroyed vessel. |
| Dependencies | G012 participation, G046 salvage, G057 defeat/conditions, G058 diagnosis/parts, G061 deployed-resource recovery and later personnel/craft/stores/service Systems. Replacement topology, effort and tools remain open. |

### Resolved choice: can destroyed components be replaced in the field?

| Option | Form | Trade-off |
|---|---|---|
| A — Supported field replacement | Suitable components can be replaced when the crew has appropriate parts, equipment and team access; other work can still require a facility. | Creates rescue/restoration choices and useful salvage, but needs readable limits and meaningful effort. |
| B — Facility replacement | Field work restores repairable conditions; destroyed-component replacement requires an appropriate starbase/facility. | Gives towing and facilities a stronger role, but more failures prevent restoration away from services. |

**Selected: A for most components, with catastrophic exceptions.** Appropriate parts, equipment and team access allow most components to be replaced in the field. Some larger Systems can be damaged beyond repair by catastrophic damage; ordinary restoration must not automatically undo that state. Exact catastrophic criteria, replacement availability/effort and defeat/recoverability remain open. This does not make every destroyed vessel recoverable or revoke G012's active participation after loss.

**Architecture refinement — AD52:** physical ship destruction requires a supported catastrophe; compartment HP depletion alone does not destroy the vessel. Crew can abandon before catastrophic loss, and loss of engines and power can establish disablement. Physical destruction, recoverable disablement, abandonment and an observer's assessment that the ship is out of action remain distinct. Individual AI uses permitted observations, reports and Comms to reach a clear operational assessment and responds through ordinary objectives/doctrine, without waiting for an explosion or reading hidden machinery state. Later observed recovery or hostility remains relevant. Intentionally engineless entities and local battery reserves require appropriate capability interpretation; exact predicates and confidence/persistence remain open. AD54 selects abandonment initiation authority. See the [ship-loss distinction](./architecture/index.html#ship-destruction).

**Architecture refinement — AD53:** the normal abandonment procedure issues non-hostile cease-fire/standby orders, retaining supported evacuation, life support and distress functions where possible. Ordinary command channels and individual System AI execute authorised operations within physical constraints and transition timing; human-held Systems retain their authority. This is not a global shutdown or proof that the vessel is safe. See [abandonment orders](./architecture/index.html#abandonment-orders).

**Architecture refinement — AD54:** a human initiates abandonment when human players are aboard. AI may report danger and recommend evacuation, then execute authorised steps for delegated Systems. Fully AI-crewed ships can initiate through ordinary individual System AI policies, permitted knowledge and objectives. Control assignment remains data-driven; no vote, mandatory captain approval, central controller or special Backfill behaviour is added. Actual evacuation remains distinct from seat vacancy/disconnection. See [abandonment initiation](./architecture/index.html#abandonment-initiation).

**Architecture refinement — AD55:** authorised crew can return an abandoned ship to service during the same scenario through supported personnel transfer and restoration. Preserve the same ship, Systems, Consoles, damage, supplies and other persistent state. Abandonment alone requires no port recovery; particular repairs still follow their facility/access/material rules. Normal authority, individual AI and physical requirements govern resumed operation. Returning aboard does not automatically restart hostile orders, repair machinery or clear an observer's assessment. Hostile capture remains separate. See [return to service](./architecture/index.html#abandoned-ship-recovery).

## G065 — Resource capacities, transfer and replenishment

**Inclusion recommendation: retain real stocks/capacities and physical replenishment; review inventory management as its own delegable System.** The [current external baseline](../gdd/mechanics/external-operations.md) transfers between actual capacity ledgers and retains already transferred resources. The [C4 resource baseline](../gdd/mechanics/power-resource-network.md) plans named bulk propellant/material stocks, capacities, transfer and consumption. G054 selects fabrication plus starbase ammunition restocking; G055 selects consumable coolant; G058 selects typed repair parts; G046/G064 provide recovery and restoration reasons to care about stocks.

[R6's Foxhole/Carrier Command 2 comparisons](./research/comparable-games-and-falling-frontier.md) connect delivery and protected routes to feasible operations. The transfer for Phoenix is meaningful stock choices and arrival consequences, not MMO production labour. Located delivery details remain at G108; trade/hazardous lots follow at G066.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Resource amount, usable storage and transfer constraints should be real and readable. Review the user's queued idea of a separate inventory-management System with desired stock levels, rather than distribute every stock task among consumers. |
| Activity | Forecast needs, choose a stock mix/reserve, identify a useful replenishment source and decide what to transfer or preserve. Scarcity should change a plan rather than only add a warning icon. |
| Interactions | Consuming Systems report demand, stores assesses available supply/capacity and Comms/Helm supports access and physical delivery. Engineering continues to own power allocation. |
| Complexity and AI | A whole inventory System could manage routine stock handling/replenishment within authorised priorities, availability and physical limits. Humans retain meaningful priorities when resources conflict. No mandatory dedicated inventory Station is adopted. |
| Communication and pillars | Demand reports and limited stocks connect weapon preparation, repairs, cooling and external work. One operator cannot silently promise resources that do not exist. |
| Risks | Spreadsheet chores, excessive categories, free instant transfers, stock targets that cannot all fit and inventory AI becoming an omniscient logistics planner. |
| Dependencies | G038 exchange, G046 salvage, G054–G055 consumption, G058 parts, G064 replacement, G066 cargo, G108 delivery and S041/S042/S089–S093. Exact categories, shared/dedicated capacity and replenishment policies remain open. |

### Resolved choice: should inventory management be its own System?

| Option | Form | Trade-off |
|---|---|---|
| A — Stock controls within consuming Systems | Weapons, repair and other consumers manage their relevant stocks through their own controls/AI. | Keeps local tasks together, but cross-resource priorities and replenishment can become fragmented. |
| B — Dedicated inventory-management System | Give stock management its own individual System AI, with desired stock levels/priorities and routine handling within actual limits. | Supports whole-System delegation and coherent replenishment, but needs useful human choices and clear requests from consumers. |

**Selected: B, with main hold stores and local System stocks.** Local Systems maintain stock levels, while the hold contains the main stores. Stock targets are set manually; replenishment then runs automatically and takes time. Manual movements remain available but should not be necessary for routine operation. Preserve individual System AI and existing authority/knowledge limits; do not add a central omniscient planner. Exact capacities, throughput, scarcity priorities and Station ownership remain open. A stock target is not guaranteed supply or authority to make a protected allocation.

## G066 — Cargo lots, trade and hazardous goods

**Inclusion recommendation: retain cargo with operational consequences; review when distinct consignments are worthwhile.** [C4/C9's roadmap baseline](../gdd/future/roadmap-baseline.md) plans hazardous intervention and later discrete cargo lots/economy. G038 already selects contextual exchange grounded in real stocks/needs without a general market-pricing simulation. G065 gives the hold main stores and timed automatic local replenishment; it does not yet determine whether transported cargo is only quantities or distinguishable consignments.

[R6's Foxhole/X4/Falling Frontier comparisons](./research/comparable-games-and-falling-frontier.md) provide logistics and cargo research directions, with Falling Frontier's details advertised rather than playtested. The Phoenix activity opportunity is assessing a particular shipment's need, risk or claim, rather than maintaining a slot grid or trading spreadsheet.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Keep routine supplies readable as stocks. Distinct cargo lots could preserve relevant ownership, origin or condition where these change an operation; exact attributes remain candidates. |
| Activity | Assess a consignment, choose whether/how to carry, transfer or dispose of it, and weigh cargo against useful stores/capacity or safe operating conditions. |
| Interactions | Sensors investigates supported hazards/condition; Comms handles actors/claims/access; stores manages capacity and transfer; Engineering supports containment where applicable. Formal commitments remain authored/PCG mission work under G041, not universal contracts for every transfer. |
| Complexity and AI | Ordinary System AI handles routine manifests, stock accounting and delegated transfer. Human decisions concern consequential cargo choices; simple presets should not demand handling every unit. |
| Communication and pillars | Information about a shipment should change a route, recipient, handling method or priority. Claims and hazard evidence can create non-combat work without a GM inventing every incident. |
| Risks | Inventory busywork, arbitrary hazard surprises, duplicated manifests, market trading detached from bridge work and metadata nobody uses. |
| Dependencies | G023 records, G038 exchange, G041 commitments, G046 salvage, G059 containment, G061 recovery, G065 stocks, G067 services and S089/S092. Exact metadata, capacity, hazard rules and inspection remain open. |

### Resolved choice: should cargo include distinguishable consignments?

| Option | Form | Trade-off |
|---|---|---|
| A — Mainly bulk quantities | Track amounts by cargo/resource type; individual shipment identity is chiefly authored mission context. | Compact accounting, but fewer systemic differences between otherwise similar shipments. |
| B — Meaningful cargo lots | Keep routine stocks aggregated, but distinguish consignments when origin, ownership, condition or another property affects decisions. | Supports investigation, claims and handling choices, but needs useful metadata and automatic bookkeeping. |

**Selected: B.** Keep routine supplies aggregated, while meaningful consignments preserve relevant properties such as ownership, origin or condition where these affect decisions. Exact attributes, hazard interactions, inspection and capacity remain open. This does not require every spare part to be individually managed or introduce a universal contract/market-price layer.

## G067 — Docked repair, rearm and service permission

**Inclusion recommendation: retain docked services as real capabilities and resource flows; review routine resupply initiation.** The [external-operations baseline](../gdd/mechanics/external-operations.md) already supports physical docking and capacity-backed transfer; [C3/C4/C6's roadmap](../gdd/future/roadmap-baseline.md) extends service access, restoration and replenishment. G054 explicitly selects starbase ammunition restocking. G035/G036/G038 already make services depend on actual machinery, stocks and systemic consequences; these do not need another inclusion vote.

G065's automatic replenishment currently describes hold-to-local-System stocks, not automatic purchase or external allocation at every starbase. [R6's Falling Frontier logistics reference](./research/comparable-games-and-falling-frontier.md) remains advertised research, while Foxhole offers a delivery-to-availability comparison. Neither determines Phoenix's resupply permission policy.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Services follow actual available stocks, facility capability, access and physical transfer/work limits. Decide whether routine authorised replenishment follows stock targets automatically or requires a new order each visit. |
| Activity | Choose a service destination, assess what it can provide, prioritise shortages/repairs and decide whether the time/access/cost is worth it. Routine execution should not consume every operator's attention. |
| Interactions | Comms establishes access or resolves exceptional terms; stores knows demand; Helm docks/undocks; repair and supply Systems perform supported work. Their individual authority/knowledge remains intact. |
| Complexity and AI | Ordinary System AI can execute delegated service actions within legitimate permission and limits. No free instant stock creation, central service planner or automatic protected beneficiary choice. |
| Communication and pillars | Reports about shortages, service availability or competing needs should affect the stop's purpose and duration. Other useful work can continue where physically feasible; do not require every crew member to watch a timer. |
| Risks | Per-item shopping chores, spending without an authorised policy, universal full-refill stations, idle whole-bridge downtime and service promises unrelated to real facility stocks/capability. |
| Dependencies | G022 contextual requests, G035–G038 services/stocks, G054 ammo, G058/G064 repair, G065 targets, G066 cargo, G108 delivered stocks and S049/S077/S084/S089. Exact service rates, prices, priorities, access and work compatibility remain open. |

### Resolved choice: how should routine docked replenishment start?

| Option | Form | Trade-off |
|---|---|---|
| A — Stock targets within an authorised service policy | Eligible routine resupply follows desired stock levels and approved limits; exceptions such as unavailable stock, unusual terms or priorities require decisions. | Extends automation and keeps visits focused on consequential choices, but requires clear permission/cost limits. |
| B — Explicit order each visit | Docking presents available services and the crew selects a replenishment order before work starts. | Gives direct control of each stop, but can repeat predictable shopping/queue work. |

**Selected: A at allied ports, with a trade exception.** Allied ports replenish automatically against stock targets. Some ports require trade; those transactions must be started manually, but the buy order is automatically populated and editable. Actual stocks, permission and physical transfer limits still apply. Exact terms, rates and repair initiation remain open; allied access is not unlimited purchasing authority or a guarantee of free services.

## G068 — Personnel traits, duty teams and scarce expertise

**Inclusion recommendation: retain the shared typed-team framework; review named expertise.** G037 already selects the same underlying team System for facility work and player-ship repair/security. G058/G061 makes team allocation, parts and deployed-resource recovery consequential. The [C7 baseline](../gdd/mechanics/duty-teams-and-operations.md) plans anonymous typed Duty Teams with named Duty Officer leaders, plus compatible System/mission postings and explicit traits. Leaderless teams remain functional in that Plan; this is a comparison point, not automatic adoption of every detail.

The user's STO-inspired low-key assignments remain queued at G069. [R8's primary-source follow-up](./research/sto-duty-officers.md) verifies historical specialisations, traits, active-roster effects and assignment contacts, without assuming current timing/odds or MMO roster scale. It suggests distinct expertise can matter, but does not prove a portrait/stat optimisation loop is worthwhile bridge play.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Use bounded typed teams; decide whether named officers add scarce expertise through leadership or compatible postings. Do not simulate every individual crew member. |
| Activity | Choose which operation deserves a team or specialist, preserve reserve capability and redeploy as circumstances change. Routine staffing/execution can be delegated to the relevant individual System AI. |
| Interactions | Repairs, security, investigation and external work can compete for the same supported personnel. Operators explain the opportunity/cost; personnel assignment authority and Station ownership remain for later review. |
| Complexity | Simple presets delegate routine staffing; deeper presets expose useful expertise and risk choices. Basic team function should not require a roster puzzle before the ship can operate. |
| Communication and pillars | Expertise creates a reason to request help or choose another method, not a universal demand that one officer approve every action. |
| Risks | Hundreds of portraits, constant optimal-slot swapping, linear rarity upgrades, arbitrary dead ends when one expert is unavailable and personnel allocation overloading an existing Station. |
| Dependencies | G012 loss continuation, G037 teams, G058 repair, G061 recovery, G069 away work, later Medical review and S079/S080. Exact traits, slots, casualty exposure and assignment authority remain open. |

### Resolved choice: should named Duty Officers supplement typed teams?

| Option | Form | Trade-off |
|---|---|---|
| A — Typed teams only | Expertise mainly follows team type; omit a separate named-officer assignment layer. | Compact staffing and fewer variables, but less distinction between available teams/specialists. |
| B — Teams plus named expertise | Add a bounded roster of named officers with useful specialisations/traits for leadership or compatible assignments; basic teams remain functional without them. | Adds consequential staffing and opportunity costs, but requires useful differences and restrained roster size. |

**Selected: B.** Duty Officers provide explicit bonuses to teams, compatible Systems (for example, manning a specific weapon) or away missions. Basic typed teams remain functional without named leaders. Exact traits, effects, slots, redeployment constraints, casualty exposure and assignment authority remain open. No mandatory named operator or collectible rarity ladder is adopted.

## G069 — Away Missions, extraction and support

**Inclusion recommendation: retain bounded off-screen operations, including low-key assignments and more demanding missions; review how risk is presented.** The user's STO-inspired assignment idea seeks worthwhile quieter work, and G068 now selects officer bonuses to away missions. G061 makes recovery of deployed crew/equipment consequential, while G012 preserves team management after mothership loss. These operations should create decisions through ordinary bridge tools rather than strand the crew watching a result timer.

The [C7 baseline](../gdd/mechanics/duty-teams-and-operations.md) plans timed phases, checks, decision events and support/extraction requests, with exact check percentages at launch and bridge actions modifying outcomes. This is planned scope, not a Current mechanic. [R8's STO follow-up](./research/sto-duty-officers.md) verifies historical assignment/specialist variety but not exact timing/odds; [R6's Carrier Command 2/Stormworks comparisons](./research/comparable-games-and-falling-frontier.md) suggest support, transport and changing conditions as useful transfers.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Support low-key bounded assignments alongside missions whose circumstances can demand bridge intervention. Exact phases, checks and autonomous team behaviour remain candidates. Review whether outcomes are shown as exact probabilities or evidence-based risk assessments. |
| Activity | Choose a worthwhile task, staff/equip it, decide how to deploy, interpret reports and provide supported information/resources/direction or extraction. Routine execution can be delegated; not every mission needs repeated branching prompts. |
| Interactions | Sensors investigates conditions, Comms negotiates access/reports, transport/Helm provides feasible deployment/recovery, defence protects supported operations and stores/Engineering supplies equipment/resources. Officer expertise has G068's explicit effects. |
| Complexity and AI | Ordinary per-System AI executes delegated operation/support within knowledge and authority. Presets can reduce manual workload without revealing undiscovered hazards or guaranteeing mission outcomes. |
| Communication and pillars | A report should make a support or priority decision useful to another operator. Bridge work continues; quiet assignments must not become mandatory reward-collection chores. |
| Risks | Exact odds exposing hidden truth, arbitrary surprise outcomes, compulsory dialogue babysitting, mandatory transport micromanagement and a second game that eclipses the bridge. |
| Dependencies | G012 participation, G017–G024 evidence/Comms, G061 recovery, G068 officers, G070 boarding, later Medical and S079–S088/S101. Exact task library, checks, support, concurrency and extraction remain open. |

### Resolved choice: how should away-mission risk be presented?

| Option | Form | Trade-off |
|---|---|---|
| A — Exact check percentages | Show calculable success/risk percentages and contributing factors as in the C7 Plan, limited to what the model permits. | Clear staffing comparisons, but can encourage optimisation of a number and obscure uncertainty about the actual situation. |
| B — Evidence-based assessment | Show known requirements, expertise/support effects and assessed risks/uncertainties; no universal exact success forecast. Better evidence can improve the assessment. | Encourages investigation and judgement, but needs legible causes and avoids arbitrary outcomes. |

**Selected: A.** Show exact check percentages and contributing factors, retaining this part of the C7 Plan rather than substituting qualitative risk bands. A check percentage is not a guaranteed mission result. Exact checks, modifiers, support/decision phases and the treatment of undiscovered conditions remain open under the existing knowledge model; do not silently reinterpret the selected numbers as qualitative estimates.

## G070 — Boarding and internal defence

**Inclusion recommendation: retain boarding as supported team operations with bridge-scale consequences; review the spatial granularity of an incursion.** The [C7 baseline](../gdd/mechanics/duty-teams-and-operations.md) plans boarding as a combat Away Mission with a selected objective, such as disabling a System, seizing cargo, rescuing personnel, gathering intelligence or capture. Security Teams oppose checks; combat/support continues. Shield, deployment and other eligibility constraints are planned comparisons, not all adopted redesign rules.

G037 already selects the shared team framework; G058/G061 makes team capacity and recovery consequential; G068/G069 adds officer bonuses and exact check percentages. [R6's Battlefleet Gothic comparison](./research/comparable-games-and-falling-frontier.md) supports fleet asymmetry only, not verified detailed boarding rules. This proposal therefore derives its form from Phoenix's team/System model rather than inventing a competitor attribution.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Support bounded boarding objectives and internal defence, with outcomes affecting actual cargo, evidence, personnel or Systems. Review a ship-wide abstract encounter versus located teams acting on particular areas/Systems. |
| Activity | Choose a boarding goal, deploy/support/recover personnel or decide which threatened capability needs defenders. Routine team execution can be delegated to ordinary System AI. |
| Interactions | Local operators report supported signs of intrusion, security/teams responds, Engineering weighs affected capability, Helm/transport controls access/recovery and Tactical supplies supported external protection. |
| Complexity | Simple presets delegate defence execution under useful priorities; deeper presets expose consequential assignments. Logical areas/links can support team location without a walkable interior or first-person combat. |
| Communication and pillars | Location and objective evidence can change which team goes where or which operation continues. Attacker and defender use the same supported physical/knowledge rules, not a hidden capture timer. |
| Risks | Instant ship takeover, opaque troop-number contests, repetitive tactical clicks, one specialist doing a separate game and a disabled console excluding its human operator from all useful participation. |
| Dependencies | G012 participation, G037 teams, G057 damage, G061 recovery, G068 officers, G069 checks/support, G071 casualties and S082/S086–S088. Exact eligibility, objectives, movement/access, detection and capture consequences remain open. |

### Resolved choice: how should boarders and defenders be represented aboard a ship?

| Option | Form | Trade-off |
|---|---|---|
| A — Ship-wide encounter | Assign forces to the incursion as a whole; checks/outcomes apply its selected objective without tracking internal positions. | Compact defence management, but fewer choices about which capability to protect or route to contest. |
| B — Located team operations | Teams act against particular areas/Systems, with supported access/travel and local consequences; defenders can contest those positions. | Connects boarding to other operators' problems and team allocation, but needs readable topology and restrained micromanagement. |

**Selected: B, with a System-marked ship map shared by team activities.** The ship map marks Systems and supports boarding/defence plus moving teams such as repair teams. Teams know their way around: telling a team to repair X System lets it find the destination without manual route input. Manual movement can be supported without becoming routine dispatch work. AD58 selects authored compartment and inter-deck connections for routing; specific passages, access/travel rules and map ownership remain open. Visible team/fault/hostile information follows legitimate observation. No private Captain map, first-person combat or mandatory path-clicking is adopted.

**Architecture refinement — AD56:** completing supported capture requirements transfers operational authority for the whole ship. Located teams can still disable equipment and secure areas beforehand, but do not gain partial System command authority. Keep the same vessel and individual System controllers; prior operator tenure no longer authorises commands after takeover. Behavioural surrender alone does not transfer control. Exact capture requirements and consequences remain open. See [capture control](./architecture/index.html#capture-control).

**Architecture refinement — AD57:** captured Consoles expose retained, already-readable sensor observations, decoded Comms and reports to their newly authorised operators without a universal extraction job. Source, age, uncertainty and Console scope persist; ordinary AI reading time and sharing rules apply. Capture does not reconstruct lost history, decrypt unread material, expose private AI memory or grant automatic continuing access to the former faction's live feeds. See [captured Console knowledge](./architecture/index.html#captured-console-knowledge).

## G071 — Casualties, Medical and persistent recovery

**Inclusion recommendation: retain personnel consequences and bounded treatment; review permanent officer loss.** The [C7 baseline](../gdd/mechanics/duty-teams-and-operations.md) plans limited Medical capacity, triage/recovery priorities and support for evacuation, quarantine or biological hazards. Named officers may be injured, missing or dead, with casualty checks on significant events rather than repeated damage ticks. These are planned comparisons, not automatic adoption of every status/rule.

G068's officer bonuses give absence/injury an operational consequence; G061/G070 makes deployed/location-based personnel relevant. G012 preserves useful human participation after ship loss. [R6's Falling Frontier crew reference](./research/comparable-games-and-falling-frontier.md) advertises capability loss and rescue/capture; it is not evidence of current playtested balance. Medical should concern simulated personnel and supported mission circumstances, not continuously track every ensign's bodily needs.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Limited treatment and recoverable personnel conditions can create triage. Decide whether explicitly lethal circumstances can also permanently kill a named Duty Officer. Exact casualty and recovery rules remain candidates. |
| Activity | Judge urgency, allocate treatment/recovery, decide whether to risk expertise and arrange supported rescue or replacement. Ordinary Medical/System AI executes delegated routine work. |
| Interactions | Team/operation reports establish casualties, Medical assesses treatment, Sensors supplies relevant specialist findings, stores provides supported supplies and transport/Helm enables evacuation. Officer absence changes other operators' choices. |
| Complexity | Simple presets delegate routine treatment under priorities; deeper presets expose useful triage. Do not make every injury a copied treatment code or mandatory puzzle. |
| Communication and pillars | A casualty report should change a staffing, support or extraction decision. Treatment/rescue can provide non-combat work while other operations continue. |
| Risks | Punishing human participation, arbitrary permanent losses, death spirals from unavailable experts, repetitive health maintenance and all incidents funnelled into one overloaded operator. |
| Dependencies | G012 participation, G037 teams, G061 recovery, G068 expertise, G069 checks, G070 locations and S079–S081/S100. Exact risk disclosure, statuses, treatment/evacuation and campaign persistence remain open. |

### Resolved choice: can Duty Officers be permanently killed?

| Option | Form | Trade-off |
|---|---|---|
| A — Recoverable personnel loss | Officers can be injured or unavailable, but default consequences preserve a route to recover them rather than permanent death. | Keeps expertise continuity forgiving, but limits the stakes of lethal operations. |
| B — Permanent loss in explicitly lethal circumstances | Officers can die where the relevant operation/damage event supports lethal risk; other conditions remain recoverable as appropriate. | Makes risk and rescue consequential, but requires fair disclosure and continued useful play after loss. |

**Selected: B.** Duty Officers can die permanently in explicitly lethal circumstances supported by an operation or damage event. Other personnel conditions may remain recoverable. Human players retain useful participation regardless of simulated casualties. Exact risk disclosure, checks, treatment/rescue and campaign persistence remain unresolved. No arbitrary death roll on every damage tick is adopted.

## G072 — Sidegrade loadouts and starbase refit

**Inclusion recommendation: retain material choices with visible operating trade-offs; review the boundary between field replacement and refitting.** The [C5/C6 baseline](../gdd/future/future-modes.md#plan--c5c6-ship-customisation) plans a small reactor/Tactical loadout MVP, station-claimant choices with authored defaults, and later starbase refit. Its [PASM contract](../../pasm/spec/design/ship-customisation.yaml) has a **soft power budget**: excessive demand is a runtime sustainability problem, not a lobby rejection. The redesign's G055 resource model changes those consequences; it does not automatically add a hard fitting currency.

G046 selects recovery of useful surviving components, G064 permits field replacement for most components and G068 allows officers to man particular Systems. It remains undecided whether a field replacement must restore the previous fit or can also change compatible equipment. [R2's fitting comparison](./research/functional-ship-customisation.md) distinguishes fitting legality from runtime endurance, but historical unsupported version details are not adopted.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Equipment choices should change opportunities and weaknesses through actual operating costs. Authored defaults retain easy entry. Decide whether limited compatible equipment changes can happen in the field or loadout changes remain facility-only. |
| Activity | Assess upcoming needs/evidence, choose a worthwhile equipment trade-off, secure parts/equipment and decide whether fitting work is worth its time and loss of current capability. |
| Interactions | Affected operators own useful expertise about their equipment; Engineering/stores explains supply/cooling/material limits; Comms/Helm supports service access and working position. Exact choice authority remains for later review. |
| Complexity and AI | Routine fitting/replacement execution can be delegated to the relevant System AI. Presets/defaults avoid mandatory lobby design work; personal complexity does not alter physical equipment. |
| Communication and pillars | A more demanding weapon can change other Systems' available margin. Equipment decisions should prompt useful discussion, not give one officer every fitting control. |
| Risks | Preflight optimisation replacing play, linear upgrades eclipsing hull roles, silently adding hard budget caps, field swapping without work/cost and changing an occupied Station's useful activities without coordination. |
| Dependencies | G046 salvage, G055 resources, G064 replacement, G067 services, G068 postings and G073–G075 legality/profiles/layout. Exact equipment catalogue, fitting effort, storage and authority remain open. |

### Resolved choice: can field replacement also change the loadout?

| Option | Form | Trade-off |
|---|---|---|
| A — Refit at facilities | Field replacement restores the fitted capability; equipment sidegrade changes use the planned starbase/facility refit path. | Clear service role and fewer underway fitting interruptions, but less adaptation using recovered equipment. |
| B — Limited compatible field swaps too | Suitable equipment can be changed in the field with parts/equipment, team access and work time; major refits still need a suitable facility. | Connects salvage and changing needs to adaptation, but requires readable compatibility and meaningful fitting commitments. |

**Selected: B.** Suitable compatible equipment can be changed in the field with parts/equipment, team access and work time; major refits still need a suitable facility. This extends G064's replacement direction and differs from the existing Plan's starbase-only loadout-change boundary. Exact limits, major-refit criteria, compatibility and authority remain open. The network architecture stays unchanged; no unrestricted ship construction is adopted.

## G073 — Typed mounts, compatibility and partitioned fit budgets

**Inclusion recommendation: retain physical compatibility and the existing soft operating budget; review how broad mount categories are.** The [PASM fitting contract](../../pasm/spec/design/ship-customisation.yaml) plans typed/sized mounts, compatible module fragments and some optional empty slots. It separately and explicitly rejects a hard power fitting ceiling in favour of runtime sustainability. G055 now supplies the proposed operating constraints; G072 requires compatibility to make field swaps intelligible.

[R2's archived fitting comparison](./research/functional-ship-customisation.md) distinguishes mount compatibility, fitting currencies and runtime resources. Its EVE/Starsector/Endless Sky details are research leads, not newly verified rules or permission to introduce additional abstract budgets. Keep legality distinct from the warning that a physically valid fit cannot sustain all equipment together.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Mount type/size and supported interfaces define physical legality. Review narrow equipment-specific mounts versus broader compatible categories. No additional CPU/OP-style currency is selected. |
| Activity | Choose a supported role trade-off, assess the resulting operating demand and decide whether a compatible replacement is worth the fitting effort. Authored defaults preserve easy entry. |
| Interactions | Affected operators identify capability needs; Engineering explains energy/cooling endurance; stores/recovery supplies equipment; fitting teams perform the work. Station workload remains part of hull evaluation. |
| Complexity and AI | Simple choices can present compatible alternatives and concise consequences; deeper presentation can expose causes. AI executes delegated fitting within legality/authority without hidden compatibility advantages. |
| Communication and pillars | Flexible fitting should create choices about shared resources and operational coverage, not permit a silent universal best build. |
| Risks | One-fit-per-slot rigidity, universally interchangeable equipment, compatibility exception overload, hidden hard budget caps and fitting choices removing another human's meaningful activities. |
| Dependencies | G052 hull styles, G055 resources, G064 replacement, G072 swaps, G074 profiles, G075 geometry and later equipment/Station bundles. Exact classes, sizes, interfaces and alternatives remain open. |

### Resolved choice: how flexible should mounts be?

| Option | Form | Trade-off |
|---|---|---|
| A — Narrow equipment families | Most mounts accept a closely specified family, such as variants of one weapon type. | Strong hull identity and clear compatibility, but fewer substantial role changes through fitting. |
| B — Broader compatible categories | A typed/sized mount can accept several equipment families where its interfaces support them; some specialised mounts can remain narrow. | Enables useful sidegrade/role changes, but needs legible limits and consequences. |

**Selected: mostly A, with some B.** Most mounts accept a narrow equipment family; selected mounts can allow broader compatible categories where their interfaces support them. Type/size and actual supported interfaces determine legality. Exact mount classes, sizes and allowed equipment remain open. Preserve the planned soft operating budget; no additional abstract fitting currency is adopted.

## G074 — Reactor profiles and sustainability forecasts

**Inclusion recommendation: retain reactor sidegrades and understandable forecasts; review profile-selection depth.** The [PASM customisation contract](../../pasm/spec/design/ship-customisation.yaml) plans reactor profile variants and a non-blocking sustainability display. The [power baseline](../gdd/mechanics/power-resource-network.md) describes generation, reserve/recovery and dissipation trade-offs. These planned parameters are a comparison point: G055 now selects local batteries, charging/depletion heat and consumable coolant, so future profiles must express trade-offs through that shared model rather than silently retaining every old battery/exhaustion rule.

The [recorded EmptyEpsilon/Elite comparison](./README.md#research-comparison) separates allocation/thermal balancing from capacitor recharge priorities. Neither requires a freeform reactor builder. G073's narrow mounts and G072's supported refit/swaps define where equipment changes can happen; live operational allocation and the queued control-complexity ladder remain separate from choosing fitted reactor hardware.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Offer reactor differences that change sustainable supply, preparation, thermal/coolant endurance or useful peak output. Forecasts should explain constraints without banning physically legal demanding fits. Review a concise profile choice versus fine-grained reactor configuration. |
| Activity | Choose an appropriate supply/endurance trade-off, anticipate a supported operating demand and communicate available margin. Detailed live reactor/distribution controls follow the System review. |
| Interactions | Engineering evaluates supply/cooling; consumers explain intended operations; stores forecasts consumables; fitting/service work determines feasible changes. No Captain allocation dashboard or automatic solution of every crew priority. |
| Complexity and AI | Authored defaults preserve easy entry; ordinary individual System AI operates delegated hardware within the same rules. More control depth must not require a bespoke reactor build before joining. |
| Communication and pillars | A profile should change a worthwhile shared decision, such as duration of supported pressure or preparation time, not simply improve every number. |
| Risks | Optimiser-only setup, dominant reactor upgrades, forecasts that assume hidden future demand, apparent sustainability based on battery balance while ignoring heat/coolant and confusing equipment choices with personal complexity. |
| Dependencies | G055 resources, G056 no extra flux, G072 fitting, G073 mounts and S031–S039. Exact profile parameters, prediction assumptions and control ladder remain open. |

### Resolved choice: how should fitted reactor profiles be chosen?

| Option | Form | Trade-off |
|---|---|---|
| A — Authored sidegrade profiles | Select from a manageable set of coherent reactor variants with clear operating trade-offs and useful forecasts. | Easy to enter and compare, while limiting fine-grained hardware build freedom. |
| B — Fine-grained reactor configuration | Configure several reactor subcomponents/parameters to assemble a profile, with legality and sustainability feedback. | Allows deeper build experimentation, but adds preparation and balancing complexity beyond live Engineering work. |

**Selected: A.** Select from a manageable set of coherent authored reactor sidegrades with clear operating trade-offs and useful forecasts. Do not introduce a fine-grained player reactor builder. Exact profiles, forecast presentation and live controls remain unresolved; fitted hardware choice is separate from personal complexity and live allocation.

## G075 — Hull geometry, mass, layout and logistics topology

**Inclusion recommendation: retain consequential authored geometry/topology; review player layout editing as a separate scope choice.** The [ship baseline](../gdd/systems/ships-and-systems.md) authors equipment, handling and shield geometry; current Workshop authoring is separate from the planned player loadout picker. G073 selects mostly specialised typed/sized mounts; G070 selects a System-marked map and self-routing teams; G065 makes internal replenishment take time. Those choices require coherent topology, but do not by themselves require players to redesign it.

[R4's Cosmoteer reference](./research/README.md) describes layout, autonomous crew, delivery and destructible modules as operationally relevant; [R6's Avorion comparison](./research/comparable-games-and-falling-frontier.md) identifies freely built ships as another approach. These show construction as an optional source of design depth, not a requirement for Phoenix's bridge activity model.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Hull geometry/topology can influence supported arcs, protection, access, travel and supply. Decide whether players choose authored hulls/layouts or can move internal equipment/routes themselves. Exact physical mass effects remain open. |
| Activity | Learn the current ship's strengths/constraints, choose compatible equipment and adapt operation to damage or bottlenecks. Player layout editing, if selected, adds a separate preparation/refit activity. |
| Interactions | Helm assesses handling, Tactical/defence uses aspect, teams use access/travel, and stores/Engineering assesses supply paths. Shared consequences should be readable to affected operators. |
| Complexity and AI | Self-routing teams and automatic stock delivery handle ordinary execution. Station presets change workload rather than topology or physical capacity. Authored defaults preserve immediate play. |
| Communication and pillars | Layout matters when it changes a feasible route, repair priority or exposure decision; it need not become an interior-design task for every crew. |
| Risks | Construction eclipsing bridge play, mandatory preparation, freeform layouts destroying worthwhile Station bundles, geometry changes with unexplained effects and path-clicking chores. |
| Dependencies | G027 movement, G055 resource links, G057 components, G065 stocks, G070 ship map, G072 fitting and G073 mounts. Exact layout, mass/inertia, routing and damage geometry remain open. |

### Resolved choice: can players redesign internal layout?

| Option | Form | Trade-off |
|---|---|---|
| A — Authored hull layouts | Players select hulls and compatible equipment for authored mount positions/topology; ordinary fitting does not move rooms, mounts or routes. | Keeps entry and Station design coherent, while limiting construction freedom. |
| B — Player layout editing too | Add supported redesign of internal equipment positions/routes during preparation or suitable refits. | Enables deeper topology trade-offs, but adds construction, validation and workload beyond operating a bridge. |

**Selected: A.** Players choose authored hulls and compatible equipment without moving rooms, mounts or internal routes during ordinary preparation/refitting. Authors can still create varied ships. The selected ship map, team routing and timed replenishment remain useful operationally. Exact topology and mass/handling effects remain open; no player layout editor is adopted.

## G076 — Crew skill, AI behaviour and officers as build choices

**Retained under existing decisions; no repeat question.** G003 selects competent ordinary System AI with bounded knowledge/judgement; G006 retains supported stances/orders; G068 selects officer bonuses to teams, compatible Systems and away missions. Those already answer this entry's core question without an additional progression gate or AI skill tree. Basic assistance must remain usable by a new/small crew. Exact officer traits, postings and supported policy effects remain for System review; no new behaviour-unlock mechanic is adopted.

## G077 — Loot, upgrades and roguelite unlocks

**Inclusion recommendation: keep rewards connected to a run/campaign; review persistent unlocks across separate games.** G046 already selects actual surviving salvage, and G072–G075 define fitting/adaptation. [R1 #17/#30](./research/feature-gap-analysis-master.md) records historical tiered-component drops and between-session meta-progression proposals, not adopted mechanics or verified replay-value evidence. [R2](./research/functional-ship-customisation.md) provides historical fitting/progression comparisons with version-specific limitations.

The [existing fitting Plan](../../pasm/spec/design/ship-customisation.yaml) prefers sidegrades, with an explicit mission-authored campaign reactor reward exception. [Campaign continuity](../gdd/foundation/campaign-continuity.md) can preserve named consequences, officer status and accepted refits without copying every transient component state. Review this separate from account-level unlock requirements; exact campaign persistence follows later entries.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Reward discovery, recovery or mission outcomes with useful choices. Decide whether unlock progression belongs to the current run/campaign or also affects fresh games through an account/meta layer. |
| Activity | Choose a worthwhile recovered option or offered reward, weigh its costs and adapt within supported fitting rules. Do not add repetitive reward collection as a Station job. |
| Interactions | Sensors assesses a find, Comms resolves claims/offers, stores preserves capacity and affected operators judge fit/resource consequences. Human crew skill remains separate from simulated personnel bonuses. |
| Complexity and AI | Basic Systems/assistance stay available without grinding. Ordinary AI executes delegated acquisition/fitting under the same rules and knowledge. |
| Communication and pillars | Rewards should change an operating choice or opportunity, rather than merely increase a shared power number or demand a preflight unlock checklist. |
| Risks | Grinding, veteran accounts determining basic capability, linear upgrades eclipsing hull roles and persistence overriding a scenario's intended starting conditions. |
| Dependencies | G046 salvage, G068 officers, G072–G075 fitting, later campaign/mode reviews and S089/S090. Exact reward catalogue, sidegrade/upgrade exceptions and persistence remain open. |

### Resolved choice: where should gameplay unlock progression live?

| Option | Form | Trade-off |
|---|---|---|
| A — Run/campaign-bound rewards | A run/campaign can preserve earned assets/choices, but separate fresh games do not require account progression to access their intended equipment and assistance. | Preserves easy entry and scenario control, while limiting account-level build progression. |
| B — Also persistent gameplay unlocks across games | Add earned equipment/trait options that carry into separate games through a meta-progression layer, subject to mode rules. | Gives longer-term unlock goals, but adds access, balance and setup complications. |

**Selected: A.** Gameplay rewards and earned assets/choices belong within a run/campaign, without account-level gameplay unlock progression across separate fresh games. Fresh scenarios provide their intended equipment and assistance. Exact reward catalogues, upgrade exceptions and persistence remain open. An ongoing campaign does not reset merely because a session ends.

## G078 — Ship identity, naming and attachment

**Inclusion recommendation: support lightweight identity and factual history; review whether an extra mechanical attachment layer is useful.** [R1 #45](./research/feature-gap-analysis-master.md) proposes ship naming; it is not an implemented rename feature. The [campaign baseline](../gdd/foundation/campaign-continuity.md#ship-identity-and-condition) allows name, hull lineage, presentation, accepted refits and notable service history. G026 already selects automatic factual records, G068 supports named officer attachment and G077 permits run/campaign continuity.

Mechanical consequences can already follow actual equipment, personnel, damage and relationships. Distinguish recording those consequences from adding separate earned hull-trait modifiers simply to express attachment.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional naming/presentation and a factual service history can support a shared ship identity. Decide whether history also grants a distinct layer of mechanical hull traits. |
| Activity | Choose presentation when useful, recognise shared achievements/losses and make operational choices through the already selected equipment/personnel/relationship Systems. Identity itself need not be an ongoing Station task. |
| Interactions | Crew can share a name/history across changing human attendance; officers, refits and known relationships provide continuity without tying participation to one player's account. |
| Complexity and AI | Factual records can be automatic under G026; no mandatory naming, log-writing or ceremonial role. Real gameplay consequences remain under the owning System's ordinary rules. |
| Communication and pillars | Identity can give choices emotional context, but should not create obligatory chatter or obscure operational information. |
| Risks | Cosmetic administration, invented history, arbitrary stat traits duplicating actual damage/refits and one player's profile controlling the shared ship's continuity. |
| Dependencies | G026 records, G039/G040 actors/relationships, G068 officers, G071 casualties, G077 rewards and later campaign review. Exact presentation and identity persistence remain open. |

### Resolved choice: does ship history need separate mechanical traits?

| Option | Form | Trade-off |
|---|---|---|
| A — Identity/history without another modifier layer | Support optional name/presentation and factual history; mechanical consequences come from actual crew, equipment, condition and relationships. | Encourages attachment with fewer overlapping mechanics, but no separate earned hull-trait progression. |
| B — Earned hull traits too | Significant history can grant persistent ship quirks/traits with additional mechanical effects within the run/campaign. | Makes history visibly affect capability, but adds modifiers and can duplicate consequences already modelled elsewhere. |

**Selected: A.** Support optional ship identity/presentation and factual history, with mechanical consequences coming from actual personnel, equipment, condition and relationships. No separate earned hull-trait modifier layer is included. Exact presentation and identity persistence remain open; no Current rename implementation is claimed.

## G079 — Objectives, recipient scopes and AI directives

**Inclusion recommendation: retain addressed objectives and ordinary typed System-AI directives; review how much the player-facing mission prescribes the method.** The [current objective model](../../wiki/concepts/objectives.md) separates player text/status/targets from optional typed directives. Each ship's scored objective pool supplies ordinary System-affinity AI behaviour, with recipient scopes preserving addressed visibility. [The shared directive contract](../../src/objectives/directive.rs) currently includes movement, combat, sensing, Comms and supported external-work kinds. Neither structure requires an omniscient planner or a procedural checklist for humans.

G041 keeps formal commitments in authored/PCG side missions; G063 supports complementary multi-bridge objectives. G061/G065 already requires work to affect actual target/resource state. The proposed mission form should allow local expertise and coordination to change a method while retaining existing command/alert/mission context for System AI.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Objectives communicate worthwhile goals, addressed information and progress. Decide whether they generally prescribe procedural steps or state outcomes/constraints with room for different supported methods. |
| Activity | Understand a goal, gather needed evidence, choose a feasible approach, coordinate resources and revise the approach when circumstances change. Objective count alone is not a measure of activity. |
| Interactions | Captain coordinates priorities; specialists decide methods in their own authority. Other bridges may have different addressed goals. Comms/Sensors information still depends on actual channels and observations. |
| Complexity and AI | Keep explicit compatible typed directives for ordinary System AI; outcome-focused text does not grant unrestricted plan synthesis. Tutorials/assistance can offer actionable guidance without rewriting the physical goal or revealing hidden information. |
| Communication and pillars | A goal should leave worthwhile choices for operators to discuss, rather than automatically announce every Station's next input. |
| Risks | Checklist play, vague goals with no useful first action, hidden completion criteria, rigid methods rejecting genuine success and an objective planner replacing crew communication. |
| Dependencies | G006 AI context, G011 authority, G022/G024 Comms, G041 commitments, G063 bridges, G080 state outcomes and G081 protected choices. Exact objective UI, method coverage and directive authoring remain open. |

### Resolved choice: what should a typical mission objective prescribe?

| Option | Form | Trade-off |
|---|---|---|
| A — Procedure-first | Objectives usually specify an ordered method/task sequence, with variation chiefly inside each step. | Clear next actions, but can restrict creative solutions and local decision-making. |
| B — Outcome-first | State the desired outcome and relevant known constraints; the crew chooses among supported methods. More prescriptive guidance remains useful for drills or specific tasks. | Supports coordination and adaptation, but goals and evidence must make a feasible first action understandable. |

**Selected: B, with optional procedures for AI decision support.** State the desired outcome and known constraints, leaving humans free to use supported methods. Objectives can offer procedures to help ordinary individual System AI choose actions, alongside existing typed directives. Exact representation and UI remain open. Preserve knowledge/authority, System communication and ordinary Backfill behaviour; no central omniscient planner or network redesign is adopted.

## G080 — State triggers, thresholds, deadlines and branching outcomes

**Inclusion recommendation: retain authoritative state-driven consequences and deadlines; review the default response to a missed goal.** The [general authoring baseline](../gdd/systems/scenario-authoring.md) keeps simulation Systems responsible for physical work while scripts pace/acknowledge events and explicitly resolve terminal outcomes. Timers can advance pressure or fail unfinished work; they must not create a completed transfer or make a protected choice for free. Genuine early work is retained through actual completion facts.

G036/G059 already selects causal propagation rather than independently scripting each consequence; G079 selects outcomes over compulsory methods. G012/G063 preserves useful participation and multi-bridge work after individual losses. The remaining design question is how often a missed intermediate goal should stop the game versus change the situation the crew still has to handle.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Use observed state, supported thresholds and meaningful deadlines to produce consequences. Keep genuine terminal conditions distinct from an optional task or intermediate setback. |
| Activity | Anticipate a developing consequence, decide which goal deserves intervention, change approach and respond to the resulting situation. Ordinary System AI uses the current mission/context under its usual policies. |
| Interactions | Specialists identify causes/options, Comms supplies supported reports, Captain coordinates priorities and affected operators decide feasible interventions. No automatic global danger discovery or alert-setting is added. |
| Complexity | Simple presentation should explain consequential state and current options; deeper investigation can expose causes. Presets do not secretly change whether physical work completed. |
| Communication and pillars | A missed goal can change others' available options, making a new plan useful. Consequences must remain interpretable rather than a random new crisis. |
| Risks | Abrupt repeated restarts, invisible deadlines, free completion at expiry, endless punitive cascades and continuation that offers no attainable meaningful objective. |
| Dependencies | G012 participation, G025 danger/alert, G036/G059 causality, G063 bridges, G079 objectives, G081 protected choices and later mode/Director reviews. Exact consequence patterns and terminal criteria remain open. |

### Resolved choice: what usually happens after a missed intermediate objective/deadline?

| Option | Form | Trade-off |
|---|---|---|
| A — Strong failure boundary | Important intermediate failures commonly end the scenario or require a restart; continuing branches are exceptions. | Clear stakes and shorter failure resolution, but can cut off useful recovery and exclude remaining activity. |
| B — Changed situation with continued play | Usually apply the supported consequence and continue with altered goals/options; end the scenario when a genuine terminal condition is met. | Supports adaptation and recovery, but needs worthwhile remaining choices and a clear eventual resolution. |

**Selected: generally B, with some A.** Missed goals/deadlines usually apply their supported consequence and leave continued play with altered goals/options. Some authored failures can end the scenario. Exact consequence patterns and terminal conditions remain open. Failure remains consequential; continuation does not guarantee rescue, grant free completion or override a terminal condition.

## G081 — Scarcity, permissions and irreversible allocation

**Inclusion recommendation: retain consequential resource/beneficiary choices and protected authority; review how allocations become commitments.** The [external-work baseline](../gdd/mechanics/external-operations.md) separates machinery preparation from choosing an irreversible beneficiary, with ordinary Backfill unable to choose one unless granted authority. The [general authoring contract](../gdd/systems/scenario-authoring.md) likewise distinguishes deadlines from protected decisions. G041 keeps formal commitments within authored/PCG missions; G079's AI procedures do not override those boundaries.

G011 already preserves local specialist authority and Captain coordination; G055/G065 makes real resources scarce. Avoid assigning every consequential choice to Captain/Comms or inventing a new confirmation layer for ordinary reversible work. The question here is whether a meaningful allocation normally commits through an explicit choice followed by work, or through the authorised physical work itself.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Competing uses/beneficiaries have supported needs, stocks and consequences. Identify the actual irreversible boundary and the authorised decision source; preparation remains distinct from commitment. |
| Activity | Establish relevant evidence, compare feasible allocations, coordinate competing needs and commit supported resources/actions. Partial allocation and alternative methods can matter where the physical model permits them. |
| Interactions | Specialists explain cost/feasibility and own their Systems; Captain coordinates goals; Comms handles supported promises/access. No universal decision dashboard or automatic Captain override. |
| Complexity and AI | Ordinary System AI can prepare and execute authorised work; a procedure/stock target does not grant permission to choose a protected beneficiary. Existing human-seeking exceptions remain available. |
| Communication and pillars | A decision should arise from different useful information and actual resource limits, not require a scripted moral vote before every transfer. |
| Risks | Accidental irreversible acts with no legible boundary, choice menus disconnected from machinery, AI making protected decisions through routine execution and all agency concentrated in one Station. |
| Dependencies | G011 authority, G022 Comms, G041 commitments, G055/G065 resources, G061 operations, G079 procedures and G080 consequences. Exact commitment boundaries, permissions and presentation remain open. |

### Resolved choice: how should a significant allocation normally commit?

| Option | Form | Trade-off |
|---|---|---|
| A — Explicit choice followed by execution | Select the beneficiary/allocation through a dedicated mission decision, then perform the required physical work. | Makes intent and protected authority clear, but can separate the interesting decision from operational play. |
| B — Authorised actions determine the allocation | Actual transfers/deployments/interventions establish the outcome, with a legible irreversible boundary; explicit mission choices remain where needed. | Connects decisions to Systems and supported alternatives, but must preserve protected authority and prevent accidental commitments. |

**Selected: B.** Actual authorised transfers, deployments and interventions normally determine significant allocations, with a legible irreversible boundary. Explicit mission decisions remain where needed. Preparation does not commit a protected allocation, and ordinary System AI cannot choose a protected beneficiary without appropriate authority. Exact boundaries, permissions and presentation remain open.

## G082 — Waves and reinforcement escalation

**Inclusion recommendation: retain authored waves for suitable encounters, and give reinforcements causes the crew can investigate and influence.** The catalogue records Current Combat Test waves; [R1 #4](research/feature-gap-analysis-master.md) offers historical wave/countdown inspiration. [R6](research/comparable-games-and-falling-frontier.md) supplies broader inspiration from logistics, actor objectives and information gathering. The recommendation below is a design inference from those comparisons, not a claim that every referenced game implements this model.

G039/G044 already gives actors objectives and bounded knowledge; G036 selects systemic propagation. Reinforcement pressure can therefore involve a distress transmission, a defending force dispatching ships, or an exposed supply route. Timing remains useful, but the crew needs supported ways to affect what arrives rather than merely clear the previous wave faster.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Decide whether most reinforcements arrive on authored escalation schedules or through in-world requests/dispatch and travel. Authored arrival boundaries can represent forces coming from outside the simulated area; a full persistent galaxy is not required. |
| Activity | Detect supported signs of an incoming force, assess its purpose, prepare or reposition, and attempt to prevent/delay dispatch or arrival where the situation allows. |
| Interactions | Sensors identifies observable movement; Comms can discover requests/transmissions under the existing channel rules; Helm and Tactical act on routes and threats; Engineering prepares reserves. Captain coordinates priorities through crew reports. |
| Complexity and AI | Simple presets handle routine tracking/responding with ordinary System AI. Deeper play investigates causes/routes and considers preventive actions. Unknown reinforcements are not automatically exposed to either humans or AI. |
| Communication and pillars | Different operators contribute evidence, arrival estimates and feasible responses. Information sharing supports preparation without prescribing a mandatory countdown announcement. |
| Risks | Endless waves, arrivals materialising in observed space, false precision about unknown travel, tedious waiting, or requiring a huge persistent simulation to support a small scenario. |
| Dependencies | G017–G024 sensing/Comms, G025 danger recognition, G039/G044 actor objectives, G080 consequences, G083 causal chains and G084 Director/generation. Exact dispatch rules, travel representation and encounter bounds remain open. |

### Resolved choice: how should most reinforcements enter an encounter?

| Option | Form | Trade-off |
|---|---|---|
| A — Scheduled waves | Usually arrive on authored timers/triggers; the crew primarily prepares for or survives the next arrival. | Predictable pacing and easy encounter authoring, but fewer opportunities to influence the cause of escalation. |
| B — Caused dispatch and arrival | Usually follow an in-world cause, dispatch and travel, with opportunities to prevent/delay them where feasible. Scheduled waves remain useful for appropriate scenarios. | Adds investigation and preventive choices, but requires readable causes and bounded dispatch/travel rules. |

**Selected: B.** Reinforcements usually follow in-world causes, dispatch and travel, with opportunities to discover and sometimes prevent, delay or intercept them. Scheduled waves remain available for appropriate scenarios; authored entry from outside the simulated area does not require a full galaxy simulation. Exact rules/bounds remain open, and unseen forces are not automatically disclosed. Adaptive difficulty policy remains for G084.

## G083 — Coupled causal chains / emergent problems

**Inclusion recommendation: make existing causal interactions reusable across scenario situations.** G036 already selects automatic propagation through supported dependencies, G059 makes escalation damage-specific, and G082 gives reinforcements causes. This review concerns how freely those rules combine during play, rather than asking again whether consequences should be systemic.

[R6](research/comparable-games-and-falling-frontier.md) draws inspiration from X4's independent actors/production, Space Rangers' coexisting conflicts and Stormworks' interacting rescue conditions. Its Falling Frontier material describes advertised logistics/information ideas, not a playtested implementation. These support a design inference: different initial states and interventions can produce different connected problems through the same rules.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Use reusable causal rules over actual stocks, services, component conditions, actors and information. Decide whether runtime combinations must be individually authored or can arise wherever their prerequisites hold. Initial situation generation belongs to G084. |
| Activity | Recognise symptoms, investigate a cause, compare interventions and observe consequences. Routine monitoring can become active work when a meaningful condition changes; no filler task or universal maintenance clock is added. |
| Interactions | A delayed delivery might reduce repair capacity and leave a facility vulnerable; the crew could restore the route, bring parts, repair directly or change the threat. This is an illustrative proposal, not a change to any existing scenario. |
| Complexity and AI | Presets simplify controls and delegate whole Systems without changing shared causes. Individual System AI acts on its own evidence and communicated context; it does not receive the complete causal graph or invent a central plan. |
| Communication and pillars | Different operators discover different symptoms, constraints and opportunities. Sharing evidence helps choose an intervention; the game need not announce the complete chain or demand a prescribed conversation. |
| Risks | Opaque chains, an expanding urgent backlog, unsupported solutions, incoherent cross-System effects and punitive cascades. Scope scenarios through relevant actors, stocks, dependencies and physical rates rather than arbitrarily stopping an otherwise supported consequence. |
| Dependencies | G009 preparation margin, G017–G025 evidence, G036 dependencies, G038/G039 actors, G059 faults, G065 stocks, G079/G080 objectives/outcomes, G082 reinforcements and later G108/G109/G112 logistics/actors. Exact reusable rules and presentation remain open. |

### Resolved choice: must each connected problem be individually authored?

| Option | Form | Trade-off |
|---|---|---|
| A — Authored combinations | Systemic consequences propagate, but connected problems are mostly assembled as specific authored chains. | Easier to pace and explain, but more situations require bespoke design and repeated chains become familiar. |
| B — Reusable rules combine during play | Supported interactions operate wherever their prerequisites hold, allowing connected problems and solutions that were not individually scripted. Authored chains remain useful. | More room for discovery and varied interventions, but requires testing interactions and making causes understandable. |

**Selected: B.** Reusable rules combine wherever their prerequisites hold, allowing connected problems and supported solutions that were not individually scripted. Authored chains remain useful. Actual state and player/actor actions determine consequences at scenario scale. Exact interactions and presentation remain open; this does not guarantee every consequence is reversible.

## G084 — Procedural situations / shared generator and Director

**Inclusion recommendation: retain the planned shared seeded generator and mode-specific Director; resolve its runtime latitude.** [The C8/C9 baseline](../gdd/future/release-bands-g-i.md) plans authored/versioned libraries, compatible composition, evidence paths, objective/terminal validation, workload constraints and repetition control. Patrol, War and Sandbox share the foundation with different profiles. These remain Plan, not Current implementation.

[R6's Thorium comparison](research/comparable-games-and-falling-frontier.md) distinguishes facilitated storytelling from systemic emergence. Phoenix keeps GM-led sessions optional. G082/G083 now gives escalation and connected problems causal rules, so runtime pacing must respect those rules and the crew's successful interventions.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Generate coherent initial situations from authored rules/content. Decide whether the Director chiefly manages assignment transitions or can introduce additional plausible opportunities/pressures during an encounter. |
| Activity | Discover a new combination, establish relevant evidence and choose an approach. Runtime pacing could offer new work or breathing room; it must not turn idle operators into recipients of arbitrary emergencies. |
| Interactions | New situations use ordinary actors, resources, information channels and travel. Existing commitments, dispatched reinforcements and ongoing faults retain their actual consequences. |
| Complexity and AI | Generation respects the supported crew/Station/assistance configuration and scenario requirements. The Director is a scenario-authoring/pacing service, not a ship AI or a source of hidden knowledge for individual System AI. |
| Communication and pillars | Compose situations that give operators complementary decisions, without prescribing a conversation or erasing investigative work through global announcements. |
| Risks | Punishing good preparation, relentless escalation, quiet-time filler, invisible difficulty changes, impossible generated tasks and runtime interventions that contradict established facts. |
| Dependencies | G008 activity, G009 reserves, G012 participation, G063 bridges, G079–G083 objectives/causality and later mode/GM reviews. Exact pacing inputs, difficulty settings, intervention limits and validation remain open. |

### Resolved choice: how much runtime pacing should the Director perform?

| Option | Form | Trade-off |
|---|---|---|
| A — Setup and assignment transitions | Generate the situation and subsequent assignments; once an encounter starts, its actors and causal rules determine pressure without activity-based additions. | Strong predictability and earned breathing room, but encounters can become quiet or overloaded without pacing intervention. |
| B — Bounded runtime pacing too | Within the mode's rules, introduce or defer plausible new opportunities/pressures based on current workload. Preserve established actors, commitments and physical consequences. | Can sustain a varied rhythm, but needs limits so competent play buys relief and difficulty does not secretly chase performance. |

**Selected: B.** The Director can introduce or defer plausible new opportunities/pressures according to workload within the mode's rules. Preserve established actors, events, commitments and physical consequences. Good preparation buys breathing room; do not erase dispatched threats, secretly change weapon stats or replace each successfully handled crisis merely to maintain pressure. Exact pacing inputs and intervention limits remain open. This is scenario pacing, not a replacement for individual System AI.

## G085 — Authored reusable situation libraries and repetition control

**Inclusion recommendation: retain versioned reusable content libraries and repetition control; judge variety by decisions rather than cosmetic changes.** [The planned generation baseline](../gdd/future/future-modes.md#plan--c6c8-patrol-mode) tracks structures, actor conflicts, hazards and resolution patterns rather than only names. G083/G084 now supports runtime combinations and bounded pacing; the unresolved issue is whether recently used situation types should be excluded or remain eligible when the operational context changes.

[R6](research/comparable-games-and-falling-frontier.md) uses PlateUp! as inspiration for preparation changing bottlenecks, and Into the Breach for interventions beyond damage. Applying that here is a design inference: a familiar rescue can still offer different choices when recovery equipment, hazards, available parts or the surrounding threat differ. Merely changing the rescued ship's name does not provide that variety.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Authors supply reusable situation pieces, constraints, evidence and supported resolutions. Track meaningful recent combinations without requiring every mission to be unprecedented. |
| Activity | Apply learned System knowledge while reconsidering the approach under changed constraints. Familiarity should support mastery; varied context should prevent rote execution. |
| Interactions | Actual equipment, personnel, stocks, actors and environmental conditions can change feasible methods and which operators have useful decisions. Variations must respect existing causal rules. |
| Complexity and AI | Repetition control operates at scenario/run level, not through more personal complexity settings. Ordinary System AI uses its normal knowledge, orders and mission context for familiar or new situations. |
| Communication and pillars | Reuse should still leave worthwhile evidence or resource choices to share. Do not manufacture secret twists solely to invalidate experience. |
| Risks | Cosmetic reskins, predictable mandatory rotation, excessive novelty preventing learning, generation failing because every type is on cooldown, and describing a changed context that does not affect actual play. |
| Dependencies | G008 activity, G009 preparation, G079 objectives, G083 combinations, G084 pacing and later run/mode reviews. Exact history scope, variety measures, selection weights and author diagnostics remain open. |

### Resolved choice: should a recently used situation type be excluded?

| Option | Form | Trade-off |
|---|---|---|
| A — Type cooldowns | Usually exclude recent situation types for a period, rotating between rescue, combat, investigation and other families. | Guarantees broad rotation, but can reject a worthwhile changed context and make the sequence predictable. |
| B — Context-sensitive selection | Downweight recent combinations of problems and solutions; allow a familiar type when changed conditions create meaningfully different decisions. | Preserves mastery and causal continuity, but needs a useful measure of variety beyond names and prose. |

**Selected: B.** Downweight recent combinations of problems and solutions, allowing a familiar situation type when changed conditions create meaningfully different decisions. Prefer actual operational variety over cosmetic changes or mandatory category rotation. Repetition control guides selection of new situations and must not suppress a real consequence already developing through the simulation. Exact history scope and selection rules remain open.

## G086 — Patrol / persistent multi-system run

**Inclusion recommendation: retain Patrol as a continuing series of bounded bridge assignments, with coherent consequences between them.** [The C6/C8 baseline](../gdd/future/future-modes.md#plan--c6c8-patrol-mode) plans an authored Patrol run before generated assignments; it does not require a continuously simulated galaxy. [Campaign continuity](../gdd/foundation/campaign-continuity.md) distinguishes resuming the same mission from carrying selected durable facts into another episode. Detailed persistence is reviewed under G090, rather than adopting every baseline reset rule here.

[R6's PULSAR and Drox Operative 2 comparisons](research/comparable-games-and-falling-frontier.md) offer inspiration for mixed activities and choosing interventions in a changing situation. The following mission-selection proposal is a design inference, not an exact copy of either game's assignment model. G041 keeps commitments mission-specific, G077 keeps rewards within the run/campaign, and G079 leaves supported methods to the crew.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain both useful single-session scenarios and optional continuing Patrol runs. Decide whether Patrol normally issues one primary assignment or offers a small set of worthwhile assignments for the crew to prioritise. |
| Activity | Compare relevant needs, routes, capability, supplies and risks; undertake a bounded assignment and understand its consequences. Avoid a mandatory administrative interlude after every mission. |
| Interactions | Comms discovers requests, Navigation assesses travel, Sensors supplies relevant evidence and operators assess capacity/resources. Crew coordination chooses the next intervention; no private Captain map or comprehensive omniscient job board is added. |
| Complexity and AI | Present enough supported information to choose a viable assignment without inspecting every System. Individual System AI executes the selected mission through ordinary directives, context and communication; it does not choose protected commitments through a new planner. |
| Communication and pillars | Assignment alternatives can give different specialists reasons to recommend a course of action, without requiring a formal vote or everyone's approval. |
| Risks | Choice overload, long briefing/route planning, meaningless alternatives, every unchosen request becoming punishment, and campaign attendance becoming a prerequisite for ordinary play. |
| Dependencies | G022 Comms, G041 commitments, G065 resources, G077 rewards, G079 methods, G080 setbacks, G084/G085 generation and later G087–G090 modes/persistence. Exact assignment count, acceptance authority, travel and consequences remain open. |

### Resolved choice: how should Patrol normally select the next main assignment?

| Option | Form | Trade-off |
|---|---|---|
| A — One assigned priority | Command normally issues the next main assignment, with optional work and tactical freedom along the way. | Quick to start and easy to pace, but less crew influence over what work to undertake. |
| B — A small set of alternatives | Normally offer a few viable assignments with different needs, risks and consequences; the crew chooses what to prioritise. Directed assignments remain available. | Makes readiness, information and priorities matter, but needs concise choices and avoids a huge job list. |

**Selected: A.** Command normally issues one main assignment, with optional work and freedom over supported methods along the way. Retain optional continuing runs and useful single-session play; a player-selected job list is not the Patrol default. Exact assignment delivery, travel and persistence remain open. Optional requests do not automatically become formal obligations.

## G087 — War, allies, coarse fronts and decisive finale

**Inclusion recommendation: retain a bounded War run whose bridge missions change the conditions of a decisive finale.** [The C7/C8 baseline](../gdd/future/release-bands-g-i.md) plans supplies, intelligence, allies and interventions feeding an ordinary final scenario. Coarse fronts and allied availability support consequences without a separate battle resolver or a continuously simulated galaxy. [R6's Drox Operative 2 comparison](research/comparable-games-and-falling-frontier.md) inspires interventions in conflicts the crew does not wholly control; it does not establish Phoenix's finale timing rules.

G022 already handles allied suggestions through Comms rather than directly controlling other ships. G039/G044 supplies objective-driven actors, and G082/G083 supports causal consequences. Patrol's assigned priority does not automatically determine War's strategic structure. Detailed territory/colony management is reviewed separately under G089.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Keep bridge missions and a decisive goal. Decide whether the finale begins at a fixed authored run boundary or the crew can commit within a state-dependent opportunity window. |
| Activity | Gather intelligence, acquire supplies, recruit/support allies, weaken opposition and judge readiness. Strategic decisions produce actual bridge operations rather than repeated abstract map actions. |
| Interactions | Sensors and Comms discover supported strategic information; operators assess capability and preparation; Navigation assesses access. Allied requests use existing channels, relationship and objective rules. No private Captain map or omniscient fleet control. |
| Complexity and AI | Coarse strategic summaries can explain known options while whole-System assistants handle routine mission work. Individual AI keeps normal communication, authority and limited knowledge; strategic assistance does not commit protected choices. |
| Communication and pillars | Readiness is distributed across ship condition, stores, allies and intelligence, giving specialists useful input into whether more preparation is worth its cost. |
| Risks | Endless preparation, arbitrary countdowns, a fixed finale ignoring legitimate early success, unobservable background defeat and strategy administration displacing bridge activity. |
| Dependencies | G022 allies, G039/G044 actors, G065 stocks, G079–G083 outcomes/causality, G084 pacing, G086 Patrol, G089 territory and G090 persistence. Exact run bounds, opportunity windows and strategic information remain open. |

### Resolved choice: when should the decisive final mission begin?

| Option | Form | Trade-off |
|---|---|---|
| A — Fixed run boundary | An authored sequence, number of missions or deadline brings the finale; earlier work changes its conditions. | Predictable length and pacing, but less influence over when preparation becomes sufficient. |
| B — Crew commitment within an opportunity window | The crew can commit when sufficiently prepared and access permits; actual conflict developments can change or close the opportunity. | Makes readiness and timing meaningful, but needs understandable limits and must avoid endless preparation. |

**Selected: A.** A fixed authored run boundary brings the decisive final mission, with earlier work changing its conditions. The boundary could be a sequence, mission count or deadline; its exact form and run length remain open. Keep strategic consequences coarse and allied actors autonomous through Comms, with the finale played through ordinary bridge Systems rather than a separate battle resolver.

## G088 — Sandbox / player-chosen sector work

**Inclusion recommendation: retain optional Sandbox with player-chosen work and discoverable sector opportunities.** [The C9 baseline](../gdd/future/release-bands-g-i.md#band-c9--sandbox-mode) plans a curated persistent sector, coarse event-driven actors/economy and a wider shared Director profile. It has neither Patrol's assignment chain nor War's required destination; GM sessions remain supported and optional.

[R6's X4 and Drox Operative 2 comparisons](research/comparable-games-and-falling-frontier.md) inspire changing outside actors and intervention opportunities. The proposal below concerns Phoenix's supported bridge actions, not importing empire administration or assuming the research games share one mission contract. Territory and colony scope is reviewed separately under G089.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Keep requests/generated missions alongside exploration, salvage, logistics and other supported bridge actions. Decide whether accepted jobs are the main organising unit or self-directed projects are equally valid play. |
| Activity | Discover local needs and opportunities, choose a purpose, prepare, act and observe consequences. A crew-created project might gather useful salvage or investigate a region without first receiving a mission. |
| Interactions | Sensors/Comms discovers relevant evidence; operators assess routes, equipment, personnel and supplies. Actors respond through their ordinary knowledge, relationships and objectives. No universal contract layer is added. |
| Complexity and AI | Clear first actions and optional offers help beginners. Individual System AI follows ordinary orders, mission context and communication; self-directed work does not require a new general-purpose planner. Exact representation of crew intent remains open. |
| Communication and pillars | Crew-chosen purposes should leave useful work for several Stations, while optional opportunities help a group that wants direction. Neither a GM nor a lengthy group planning phase is required. |
| Risks | Aimless downtime, a job board replacing the world, meaningless self-directed actions, strategic management eclipsing bridge work and manufactured urgency whenever the crew chooses quiet preparation. |
| Dependencies | G022 requests, G038/G039 actors, G041 commitments, G046 salvage, G065 inventory, G079 goals, G083–G085 emergence/pacing/variety and later G089/G090/G112. Exact discovery, intent UI, persistence and rewards remain open. |

### Resolved choice: what should organise Sandbox play?

| Option | Form | Trade-off |
|---|---|---|
| A — Primarily accepted jobs | Most purposeful activity comes from choosing offered/generated missions; unassigned actions mainly support those jobs. | Easy to find a clear goal and reward, but makes freedom depend on the mission catalogue. |
| B — Jobs and self-directed projects | Offered missions provide direction, while crews can pursue supported exploration, salvage, logistics or other projects without accepting a job first. | Supports player initiative, but the consequences and useful next actions must remain understandable. |

**Selected: B.** Offered jobs and self-directed projects are valid Sandbox play. Crews can pursue supported exploration, salvage, logistics and other goals without accepting a job first; actual effects give those actions value. Discoverable requests provide optional direction. Exact intent presentation, discovery, persistence and rewards remain open. Retain ordinary individual System AI and mission-specific commitments; detailed territory scope is reviewed next.

## G089 — Territory conquest and colony strategy

**Inclusion recommendation: retain territorial consequences that produce bridge operations; assess whether a separate development-management layer belongs in this redesign.** [R1 #34](research/feature-gap-analysis-master.md) proposes persistent conquest as a high-effort campaign idea. [R6's Carrier Command 2, X4 and Starpoint Gemini Warlords comparisons](research/comparable-games-and-falling-frontier.md) provide inspiration for supply lines, autonomous outside actors and territorial development. They do not by themselves establish that Phoenix needs colony administration.

The [C7/C9 baseline](../gdd/future/release-bands-g-i.md) already plans coarse fronts, sector control, production/needs and autonomous allies. G035/G036 treats facilities as physical Systems with dependencies; G037 uses shared teams. G087/G088 permits war interventions and self-directed projects. The remaining scope choice is between influencing territory through bridge work and adding a player-managed colony/base economy alongside it.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Territory can affect access, services, actor needs and threats. Compare facility/route operations with a separate layer of development choices, production queues and territorial budgets. Exact capture/construction rules are not settled by this choice. |
| Activity | Survey, negotiate, escort, supply, defend, repair or secure facilities/routes through supported actions. A development layer would add recurring investment and production decisions. |
| Interactions | Facilities retain shields, Systems, components, stocks and teams. Their ability to provide services follows actual conditions; allied actors keep their own objectives and Comms channels. |
| Complexity and AI | Routine facility operations can use ordinary actor/System behaviour. Do not assume whole-System ship assistance automatically supplies a new colony-manager AI; that would need its own design if selected. |
| Communication and pillars | Territorial changes should create complementary bridge decisions about approach, threat, supplies and personnel. Abstract investment controls may concentrate work in one operator and leave others waiting. |
| Risks | Territory as cosmetic colour, magical capture instantly restoring services, repeated supply chores, compulsory management between missions and expanding into a second strategy game. |
| Dependencies | G035–G040 facilities/actors, G061 deployment/recovery, G065/G066 stores/cargo, G069 operations, G087/G088 modes and later G090/G108/G112 continuity/logistics/actors. Exact control transfer, construction and long-term production remain open. |

### Resolved choice: how far should player territory management go?

| Option | Form | Trade-off |
|---|---|---|
| A — Coarse control through bridge operations | Influence facilities, routes and territory by supplying, defending, repairing, negotiating or securing them. Outside actors handle routine development. | Keeps strategic consequences connected to Station activity, with less administration. |
| B — Player-managed development too | Add colony/base investment, production queues and territorial budgets alongside bridge operations. | Adds long-term ownership and strategy, but requires another management loop and meaningful crew participation. |

**Selected: A.** Influence facilities, routes and territory through bridge operations, with outside actors handling routine development. Do not add a separate player-managed colony/base investment, production-queue and territorial-budget loop. Particular construction or facility-deployment operations remain possible; exact operation and control-transfer rules remain open.

## G090 — Campaign facts, branching persistence and recap

**Inclusion recommendation: retain consequence-led campaign continuity and concise recaps; resolve the operational-condition boundary between episodes.** [The campaign baseline](../gdd/foundation/campaign-continuity.md) distinguishes Current mission snapshots and selected-fact projection from the planned campaign shell. Its directional default assumes routine fuel/ammunition/repairable wear restored between episodes unless a durable exception is recorded. Resuming the same mission is different and retains its snapshot state; G091 reviews that usability separately.

G065/G067 now gives stores and replenishment physical meaning, G068/G071 establishes named officers and consequential casualties, and G077/G078 keeps progression and factual identity within the run/campaign. G086/G087/G088 provides different continuing modes. The choice here is whether episode boundaries normally assume routine servicing or preserve supplies/component condition until supported work changes them. It does not decide every save field or require carrying runtime entity IDs into a new world.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Carry relevant outcomes, evidence, relationships, assets and personnel consequences with a concise recap. Decide the default for ship supplies and repairable component conditions when advancing to another episode. |
| Activity | Understand what changed, assess readiness and adapt future operations. Replenishment/repair can use previously selected automation rather than mandatory manual bookkeeping. |
| Interactions | Supplies, salvage, teams, facilities and port services can matter across assignments. Authored transitions may include real downtime and available servicing; this need not require playing every routine repair in real time. |
| Complexity and AI | Normal System/stock automation can perform supported servicing. Personal complexity presets and changing human attendance do not reset shared consequences or remove ordinary Backfill. |
| Communication and pillars | Recaps give arriving players enough context to participate; readiness differences provide useful specialist advice about priorities and methods. Knowledge remains scoped to what was learned/shared. |
| Risks | Arbitrary free resets, attrition spirals, compulsory cleanup sessions, confusing old obligations, importing transient state into incompatible episodes and requiring account-level unlocks or fixed attendance. |
| Dependencies | G023/G026 evidence/history, G040 relationships, G064–G071 repair/stores/personnel, G077/G078 progression/identity, G086–G088 modes and G091 saves. Exact durable fields, mode exceptions, servicing and recap presentation remain open. |

### Resolved choice: what normally happens to supplies and repairable damage between episodes?

| Option | Form | Trade-off |
|---|---|---|
| A — Assume routine servicing | Normally restore routine supplies and repairable wear between episodes; explicitly marked shortages/damage remain as durable consequences. | Keeps each episode easy to enter, but reduces the long-term value of operational reserves and servicing. |
| B — Preserve condition until serviced | Carry relevant stocks and component conditions forward; supported repair, resupply and downtime change them, including automatic/offscreen servicing where resources and access permit. | Gives logistics and preparation lasting meaning, but needs efficient servicing and safeguards against repetitive cleanup or unwinnable attrition. |

**Selected: B.** Relevant stocks and component conditions carry forward until supported servicing changes them. Repair, resupply and available downtime can include automatic/offscreen work where resources and access permit, without playing every minute. Fresh standalone scenarios retain their intended starting state. Exact durable fields, servicing and mode exceptions remain open. This changes the baseline's assumed routine between-episode reset direction, not Current snapshot contracts.

## G091 — Save, checkpoint and compatible resume

**Retained under existing constraints; no additional gameplay choice required.** [Current save catalogues](../../wiki/concepts/save-catalogues.md) provide same-mission save/resume and compatibility handling. Preserve recovery and convenient session stopping as usability features, not a new Station maintenance loop. G090's later-episode continuity remains a separate contract. No change to networking, snapshot architecture or individual System AI follows from this review.

## G092 — Outcomes, debrief and repeat-session learning

**Inclusion recommendation: retain a concise shared debrief with inspectable factual consequences, and resolve how much hidden information it reveals.** [The campaign baseline](../gdd/foundation/campaign-continuity.md) calls for outcomes, evidence, relationships, surviving/lost assets and committed facts without collapsing trade-offs into one score. [The playtest framework](../gdd/foundation/difficulty-balance-playtesting.md) distinguishes causal reports from what humans understood: logs alone do not establish enjoyment or communication quality.

G026 selects automatic factual history, G077/G078 preserves run-bound consequences and identity, and G090 retains operational condition. A useful debrief can connect observed decisions to results. It must also distinguish information the crew learned from simulation truth that might remain relevant to a later investigation or adversary.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Give a short shared outcome/consequence summary with optional detail. Decide whether player debriefs respect crew knowledge or expose an omniscient account after an episode. Author/test diagnostics remain separate. |
| Activity | Compare expectations with results, discuss a consequential decision and identify a different approach for next time. Debrief is optional review, not a compulsory quiz or report-writing task. |
| Interactions | Include meaningful transfers, repairs, recoveries, personnel outcomes and relationships where supported. Link cause/effect only when evidence supports the explanation; distinguish uncertainty. |
| Complexity and AI | Automatic records avoid clerical work. Detail can be inspected without another complexity setting, and debrief does not retroactively grant individual System AI new knowledge. |
| Communication and pillars | A shared account can help the crew discuss coordination and preparation without grading players by input count or assigning blame from incomplete evidence. |
| Risks | Spoiling continuing mysteries, carrying hidden facts into later play, opaque scoring, lengthy mandatory reviews and presenting uncertain interpretations as authoritative causes. |
| Dependencies | G017/G018 evidence, G023/G024 scoped sharing, G026 history, G077/G078 identity/rewards, G090 continuity and G093 playtesting. Exact report contents, knowledge scope and diagnostic access remain open. |

### Resolved choice: should a normal player debrief reveal hidden causes?

| Option | Form | Trade-off |
|---|---|---|
| A — Crew-knowledge debrief | Explain observed outcomes and supported causes, retaining uncertainty about unlearned motives, events and mechanisms. | Preserves discovery and campaign information boundaries, but cannot explain every failure immediately. |
| B — Omniscient debrief | Reveal the underlying causes and unseen events after the episode, even when the crew did not discover them. | Gives fuller immediate learning, but can spoil continuing investigations and reveal adversary secrets. |

**Selected: A.** Normal player debriefs report observed outcomes and evidence-supported explanations, preserving uncertainty about undiscovered events, motives and mechanisms. Keep full causal diagnostics separate for author/testing work. Exact report contents and knowledge scope remain open; the debrief does not silently turn hidden simulation truth into campaign knowledge.

## G093 — Workload instrumentation and playtest scenarios

**Retained under the agreed pillars and evaluation framework; no additional choice required.** [The existing playtest framework](../gdd/foundation/difficulty-balance-playtesting.md) and this proposal's evaluation section already pair logs with observation and player reports. Assess meaningful decisions, communication, waiting, overload, understanding and helpful delegation across complete travel/observation, negotiation, external-work, combat and recovery situations. Start with six humans at mixed depth, then test two/eight and optional GM variants. Do not add input-count quotas or claim instrumentation measures enjoyment/flow directly. Exact numerical targets need prototype evidence; this review introduces no runtime telemetry implementation.

## G094 — GM observing, puppeting and directing

**Inclusion recommendation: retain optional GM observation, actor performance and supported directing, with ordinary autonomous viability.** [The GM baseline](../gdd/systems/gm-and-workshop.md) documents Current observation, authentic Station puppeting, actor Comms and attributed directing actions; some live human acceptance remains open. These are existing operator capabilities, not a request to redesign networking or remove privileged GM tools.

[R6's Thorium comparison](research/comparable-games-and-falling-frontier.md) inspires facilitated actor performance but does not make a GM required. G084 now selects bounded runtime Director pacing. Resolve whether bringing a GM into a session normally leaves that pacing running or makes the GM its primary pacing source. Ordinary actors and individual System AI continue in either case.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain observation, actor performance, supported puppeting and directing. Decide the default relationship between GM presence and Director introduction of new situations. |
| Activity | Perform a conversation, develop a selected actor or intervene in pacing while the crew continues meaningful bridge work. A GM should be able to focus on a few useful contributions. |
| Interactions | GM actor messages use existing Comms identities/channels; puppeting respects its existing admission contract. The Director must not introduce events that contradict a GM's established intervention. |
| Complexity and AI | GM presence does not add knowledge to crew System AI or create a special backfill mode. Pausing Director introductions does not pause physical consequences or autonomous actor behaviour. |
| Communication and pillars | GM performance enriches requests and responses while the crew still gathers evidence and coordinates through its ordinary tools. Baseline activity remains viable without facilitation. |
| Risks | GM/Director competing to escalate, compulsory GM workload, an unattended manual session going quiet and facilitated fixes concealing poor Station design. |
| Dependencies | G022/G024 Comms, G079–G084 missions/pacing, G093 testing and G095–G098 facilitation/recovery/authoring. Exact override scope, handoff and UI remain open. |

### Resolved choice: when a GM joins, should Director pacing continue by default?

| Option | Form | Trade-off |
|---|---|---|
| A — Continue, with GM overrides | The Director continues bounded pacing; the GM performs actors and directs selected moments, with controls to pause/override pacing where needed. | Lets a GM contribute without running every beat, but requires coordination between automated introductions and GM interventions. |
| B — GM takes pacing responsibility | Director introductions normally pause in facilitated sessions; the GM chooses new beats, with automatic pacing available when requested. | Gives tighter direct control, but increases facilitator workload and needs a clear handoff. |

**Selected: A.** Director pacing continues by default when a GM joins, with controls to pause or override it. A GM can perform actors and direct selected moments without being responsible for sustaining all new work; deliberate full pacing control remains available. Exact override/handoff rules remain open. Ordinary actors, individual System AI and physical consequences retain their normal behaviour.

## G095 — GM attention, history, undo and checkpoint recovery

**Retained under existing constraints; no additional gameplay choice required.** [The GM baseline](../gdd/systems/gm-and-workshop.md) provides advisory attention tools, attributed history, supported invertible undo and checkpoints. Technical reversibility differs from reversing fictional consequences. Preserve these capabilities and their documented human acceptance work; no universal rewind or new recovery architecture is proposed. G093 still tests whether baseline Station activity works without GM repairs to pacing.

## G096 — Information control and in-sim event cues

**Inclusion recommendation: retain information/presentation tools and resolve their normal fictional scope.** [Current information control](../../pasm/spec/design/t4-information-control.yaml) permits observer-specific reported classifications, ghost contacts and delayed/degraded reports without changing material world truth. Classification overrides do not erase independently earned scan evidence; ghosts are reports, not physical target entities. These tools can be invoked by scenario authoring as well as GMs.

G017–G024 makes investigation and channel-based sharing substantive play; G083 makes material consequences causal; G092 preserves undiscovered information in normal debriefs. The remaining question is whether false reports normally need a simulated source or may also be deliberately authored information effects. Neither option makes a ghost physically real or gives crew AI hidden truth.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain classification/report controls and in-sim cues. Decide whether fictional use requires actual malfunction, interference or deception machinery, or permits authored/GM reports representing causes outside the model. |
| Activity | Compare evidence, investigate a discrepancy, communicate uncertainty and revise an interpretation. A misleading report should create useful investigation rather than arbitrary wrong answers. |
| Interactions | Sensors readings, Comms, observed actions and other evidence can corroborate or contradict a report. Shared Viewscreen presentation retains knowledge scope; cues do not create physical completion. |
| Complexity and AI | Ordinary System AI keeps its existing report/knowledge contract; no new omniscient evaluator or special backfill response. Exact AI treatment of future report types needs later System review. |
| Communication and pillars | Operators can share conflicting evidence and choose what to trust. Avoid mandatory code copying or hiding every usable clue behind one operator. |
| Risks | Unfalsifiable GM deception, misleading information erasing earned findings, cinematic cues obscuring controls and false reports becoming physical targets. |
| Dependencies | G017–G025 observations/Comms, G083 causality, G092 debriefs, G094 facilitation and later Sensors/EWar System reviews. Exact discovery paths, override limits and cue use remain open. |

### Resolved choice: must misleading reports have a simulated source?

| Option | Form | Trade-off |
|---|---|---|
| A — Simulated causes | Normally use false/degraded reports only when actual faults, interference or deception equipment produce them. | Strong causal consistency, but requires modelling each source before using the effect in a story. |
| B — Authored information effects too | Also allow scenario/GM-created reports representing deception or phenomena outside the detailed simulation, with evidence through which the crew can investigate. | Supports flexible mysteries and facilitation, but needs coherent clues and respect for independently earned evidence. |

**Selected: B.** Also allow scenario/GM-created misleading reports representing deception or phenomena outside the detailed simulation, with clues the crew can investigate. Preserve independently earned evidence and distinguish reported information from material truth. Exact discovery paths and limits remain open; false contacts do not become physical ships.

## G097 — Campaign continuity / advisory pacing tools

**Retained under agreed constraints; no additional gameplay choice required.** [M9's baseline](../gdd/systems/gm-and-workshop.md) remains Plan: campaign continuity and deeper pressure/beat-stacking/lull advice. Align continuity with G090 and GM/Director interaction with G084/G094. GM advice remains advisory and does not automatically execute suggested actions; the Director has its separately agreed bounded pacing remit.

## G098 — Workshop, authoring, disposable Test and packs

**Retained supporting capabilities; no architecture redesign proposed.** [Current Workshop](../gdd/systems/gm-and-workshop.md) supports authored content, validation and disposable Test with distinct Author/Test/Live authority. Preserve this basis for prototyping activity combinations and comparing them under G093. Human workflow acceptance remains as documented; experimentation does not automatically amend PASM or delivery commitments.

## G099 — Models, views, audio and localisation

**Retained under the pillars and accessibility constraints.** Presentation should make supported state readable, help operators distinguish consequential cues and support sharing through existing views. Keep localisation and accessible alternatives, non-voice urgency sounds and G010's no-AI-voices choice. G096 distinguishes presentation/information effects from material consequences. Detailed cues and information ownership should be assessed alongside each ship System rather than fixed here as one universal interface.

## G100 — External peripherals and venue show control

**Retained as optional planned adapters, outside the gameplay/network redesign.** The ordinary browser/phone route remains complete. Optional hardware can supply controls or presentation, but must not hold essential information or authority unavailable through ordinary tools. No peripheral protocol or networking change is proposed.

## G101 — Difficulty, assistance and challenge tuning

**Inclusion recommendation: retain separate shared challenge and personal complexity/assistance choices; resolve automatic scaling by human count.** [The difficulty baseline](../gdd/foundation/difficulty-balance-playtesting.md) distinguishes mechanical/scenario challenge from operating preferences and accessibility. G002/G003/G004 already selects presets, competent individual System AI and ordinary Backfill; the ideal crew is six humans, with two to eight supported.

G084 allows bounded pacing according to workload without erasing established consequences or secretly altering weapon stats. The choice here concerns the initial shared challenge when the same scenario runs with different human crew sizes, not whether its Director can manage introductions or whether missing Stations have AI.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Keep a clear shared challenge choice. Decide whether human count automatically changes the initial opposition/objective workload, or chiefly changes the human/AI distribution under the chosen challenge. |
| Activity | Select a suitable scenario/challenge, choose personal depth, then make worthwhile decisions with competent assistance. Two humans should not have to manually cover all six Stations. |
| Interactions | Human presence affects who handles a System, not its physical rules or authority. Scenarios may retain explicit human-control requirements under G002. |
| Complexity and AI | Do not infer skill or desired challenge from a complexity preset. AI remains ordinary individual System AI; automatic scaling, if selected, must not replace it with a central planner. |
| Communication and pillars | Mixed-depth crews can coordinate without everyone adopting expert controls. Explain expected human requirements and challenge before play. |
| Risks | Hidden scaling, difficulty changing on reconnect, expert settings increasing colleagues' burden, two-human crews being forced to micromanage and assuming more humans always makes play easier. |
| Dependencies | G002–G004 presets/AI, G008/G009 activity/margins, G084 pacing and G093 testing. Exact challenge profiles, roster handling and human-control requirements remain open. |

### Resolved choice: should human crew size automatically change the chosen scenario's challenge?

| Option | Form | Trade-off |
|---|---|---|
| A — Explicit challenge, AI covers vacant Stations | The chosen challenge stays independent of human count; fewer humans chiefly means more ordinary AI coverage. Recommend suitable scenarios/settings where needed. | Predictable shared rules and comparable scenarios, but AI-supported small crews need deliberate validation. |
| B — Automatic initial scaling too | Human count also adjusts the initial opposition/objective workload within declared bounds, while ordinary AI still covers vacant Stations. | Can tailor workload to crew size, but count is an imperfect measure of ability and scaling needs clear handling. |

**Selected: A.** Players choose shared challenge explicitly, independent of human crew size; fewer humans chiefly means more ordinary whole-System AI coverage. Do not automatically scale initial opposition/objective workload from human count or infer skill from personal complexity. Retain G084's bounded pacing and G002's explicit scenario human-control requirements. Exact challenge profiles and suitability still need playtest evidence.

## G102 — Explicit research boundaries

**Inclusion recommendation: keep research broad while choosing a coherent bridge experience; resolve direct on-foot participation separately from remote operations.** [R1's historical exclusions](research/feature-gap-analysis-master.md) are not current authority: optional GM sessions, PvP, abstract personnel operations and full-3D small craft are already supported directions in this review. Networking and individual System AI remain fixed by the user's brief.

[R6](research/comparable-games-and-falling-frontier.md) inventories PULSAR's away activity, Avorion's construction and larger economy/strategy references without importing their complete genres. G038/G089 already bounds economy and territory; G075 selects authored hull layouts. G069 retains offscreen Away Missions, G070 located teams and G027 full-3D small craft. None requires first-person characters.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Keep useful consequences of boarding, rescue, investigation and other external work. Decide whether humans participate through consoles/teams/craft or also through optional directly controlled on-foot characters. |
| Activity | Console operations offer deployment, equipment/personnel commitment, support, investigation and recovery. Direct characters would add movement/exploration/action at a separate player scale. |
| Interactions | Ship support, transport, supplies, Comms and recoverable teams remain meaningful either way. Direct character play would need a clear relationship to ordinary Backfill and ongoing bridge work. |
| Complexity and AI | Existing whole-System assistance is retained; direct character play would require additional input/AI design, rather than assuming a ship System AI can control it. Console-managed teams keep the shared team framework. |
| Communication and pillars | External work should generate decisions for the bridge while personnel are deployed. Avoid an away player getting all the interesting activity while the bridge waits. |
| Risks | A second action game, extended absence from Stations, phone-control burden and restricting worthwhile rescue/boarding ideas merely because their research source is first-person. |
| Dependencies | G004 Backfill, G027 craft, G037 shared teams, G061 recovery, G069/G070 operations, G075 layout and later ship transport/operations Systems. Exact team mission/craft controls remain for System review. |

### Resolved choice: should this redesign include directly playable on-foot characters?

| Option | Form | Trade-off |
|---|---|---|
| A — Console-managed external work | Humans manage teams and support operations through bridge tools; retain small-craft capabilities separately. No first-person/on-foot player mode in this proposal. | Concentrates design on shared bridge decisions, but away activity is experienced through reports and operations. |
| B — Optional direct on-foot play too | Some missions let humans directly explore/fight/act outside the ship, with ordinary Backfill covering their Stations. | Adds personal exploration/action, but needs a second gameplay/control model and useful work for the remaining bridge. |

**Selected: A.** Humans manage external work through bridge consoles, deployed teams, support and recovery. Do not include a directly playable first-person/on-foot mode in this proposal. Rescue, boarding and EVA teams remain valid mission subjects; small-craft capabilities are separate. Exact operation/craft controls remain for ship System review.

## G103 — Transcription, human-error puzzles and multi-officer rituals

**Inclusion recommendation: use complementary interpretation for communication; decide whether a separate authored puzzle layer belongs alongside it.** [R1's historical research](research/feature-gap-analysis-master.md) advocates coordinate transcription, reactivation codes and cyphers for party-game drama. [R6's Keep Talking and Nobody Explodes/Captain Sonar comparisons](research/comparable-games-and-falling-frontier.md) instead identifies useful transfers in complementary information and one operator's actions changing another's choices. These are design inferences, not a requirement to copy those games' input rituals.

G011/G022 already makes distributed information, local authority and sharing central; G058 gives diagnosis actual evidence and component causes. Communication can arise from combining a scan, equipment behaviour and a technical record. The open question is whether to add explicit puzzle modules beyond those operational problems. Encryption levels under G024 remain valid whichever option is selected; this does not decide interception/decryption System controls.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Prioritise operational interpretation. Decide whether selected scenarios should additionally feature explicit code/pattern/manual puzzles with authored answers. |
| Activity | Share observations, compare explanations and choose a supported intervention. An explicit puzzle would add decoding/pattern reasoning, rather than simply reading an already solved value aloud. |
| Interactions | System evidence and consequences provide the core shared task. Puzzle integration, if chosen, should affect useful action and preserve actual physical/authority rules. |
| Complexity and AI | Keep ordinary whole-System AI and personal presets. Scenarios can explicitly require a System under human control; do not invent deliberately incompetent AI just to preserve a puzzle. Exact puzzle delegation would need design if included. |
| Communication and pillars | Different useful information can justify talk without compulsory transcription, repeated acknowledgements or artificial mistakes. Optional explicit puzzles must not become a routine duty on every Station. |
| Risks | Rote code copying, inaccessible timers, memorised answers, gatekeeping by the keenest puzzle solver and interesting operational choices reduced to unlocking a scripted answer. |
| Dependencies | G002 human-control exceptions, G011 expertise, G022/G024 sharing/encryption, G058 diagnosis, G079 methods and later Comms/Sensors/repair System reviews. Exact domain interpretation and any puzzle content remain open. |

### Resolved choice: should explicit authored puzzle modules be part of the proposal?

| Option | Form | Trade-off |
|---|---|---|
| A — Operational interpretation | Communication challenges come from interpreting different evidence and System constraints; no separate code/pattern/manual puzzle layer is proposed. | Connects cooperation to reusable game rules, with fewer bespoke minigames. |
| B — Selected puzzle encounters too | Add occasional authored decoding/pattern/manual puzzles alongside operational interpretation, rather than as routine maintenance. | Adds a distinct cooperative challenge, but needs careful accessibility, AI delegation and replay design. |

**Selected: A.** Communication challenges come from operational interpretation: combine complementary evidence and System constraints to choose supported interventions. Do not add a separate authored code/pattern/manual puzzle layer. Encryption, investigation and domain knowledge remain relevant; exact controls stay open for ship System review. Avoid routine transcription, acknowledgements and induced mistakes as the basis of cooperation.

## G104 — Fitting simulator, blueprint sharing and theorycrafting

**Inclusion recommendation: keep fitting/rehearsal optional and decide whether it is author-only or also a crew tool.** [R2](research/functional-ship-customisation.md) describes EVE's hypothetical fitting simulator and shareable fitting formats, alongside other games' planning/blueprint ecosystems. These inspire comparing compatible equipment without first acquiring/installing it; Phoenix need not adopt their skill gates, fitting currencies or construction models. [Current Workshop/Test](../gdd/systems/gm-and-workshop.md) already supplies an author-oriented rehearsal foundation, not a finished player fitting planner.

G072/G073 chooses limited field swaps and compatible mounts, G074 authored reactor profiles, G075 authored layouts and G077 run-bound rewards. A blueprint here means a planned compatible equipment configuration, not rearranging rooms or freely building a hull. Planning must not grant equipment, permission or instant installation.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Decide whether the proposal relies on Workshop/Test for fitting experiments or adds a simpler optional crew-facing compare/save/share/rehearse experience. |
| Activity | Compare useful capabilities and resource demands, rehearse operation and share a candidate fit. Preparation can involve specialists without becoming mandatory before joining. |
| Interactions | Use the same mount compatibility and operational power/heat/stores rules as the ship. A planned fit can exceed sustainable output without implying an abstract hard power-fitting cap. Actual installation follows G072. |
| Complexity and AI | Ready-to-play authored ships remain complete. Rehearsals use ordinary System AI and physics; no unique fit-testing AI or extra personal complexity configuration is required. |
| Communication and pillars | Specialists can compare trade-offs and try a configuration together, while novice crews can use defaults immediately. A shared plan is advice, not authority to install it. |
| Risks | Mandatory homework, the keenest optimiser controlling the session, theoretical statistics hiding operational constraints, invalid shared plans and preparation tools becoming a second game. |
| Dependencies | G015 rehearsal, G065 stores, G072–G075 fitting/layout, G077 rewards, G093 evaluation and G098 Workshop. Exact scope, sharing format, availability and rehearsal scenarios remain open. |

### Resolved choice: should fitting experiments have a crew-facing tool?

| Option | Form | Trade-off |
|---|---|---|
| A — Workshop/Test only | Keep advanced fitting experiments and sharing in author tools; ordinary crews use offered ships and in-game refitting. | Smaller preparation surface, but interested crews must learn author tools to compare hypothetical fits. |
| B — Optional crew planner too | Offer a simpler way to compare compatible equipment, save/share planned fits and rehearse them, alongside Workshop/Test. | Supports voluntary experimentation and specialist depth, but needs a clear boundary from actual stock/installation and ready-to-play defaults. |

**Selected: B, with personal favourites per ship, per Station or whole ship.** Provide an optional crew-facing compare/save/share/rehearse planner alongside Workshop/Test. Players can bring their favourite loadout into a game, covering either one Station or the whole ship for the relevant hull. Persisting a saved plan is not an account-level equipment unlock; compatibility, scenario options and actual fitting/refit rules still apply. Ready-to-play defaults remain complete. Exact tool format, availability and preset composition remain open.

### Resolved dependency: how do whole-ship presets compose with individual Station choices?

[The planned fitting authority](../../pasm/spec/design/ship-customisation.yaml) gives a Station claimant its materiel choices and leaves unclaimed Stations on authored defaults. Whole-ship favourites add a new composition case: one person's saved plan may cover Systems other players want to configure. This is a pre-launch fitting question, not granting anyone universal in-mission controls.

| Option | Form | Trade-off |
|---|---|---|
| A — Whole-ship preset seeds defaults | A whole-ship favourite supplies initial choices, while each Station claimant can bring their own favourite or change their Station's fit. Loading another whole-ship preset preserves explicit claimant choices. | Lets personal favourites coexist without a confirmation round, but the final combined fit may differ from the original whole-ship plan. |
| B — Whole-ship preset is an explicit group replacement | Applying a whole-ship favourite replaces Station choices as one coordinated crew decision; individual changes can follow afterward. | Makes complete fleet/build concepts easy to apply, but needs clear authority to replace other players' choices. |

**Selected: A.** Whole-ship favourites seed initial defaults. Each Station claimant can bring a personal favourite or change their Station's fit; later whole-ship imports preserve explicit claimant choices. Compose the result into one shared ship without a hard reactor-budget cap. Exact conflict presentation and shared-System handling remain open.

## G105 — Search-to-engagement chain and guidance continuity

**Inclusion recommendation: distinguish useful detection from the information/support required by particular weapons.** [R6's NEBULOUS comparison](research/comparable-games-and-falling-frontier.md) links detection, fire-control/illumination, guidance and terrain. The Phoenix transfer is an inference: combat can include choosing exposure and supporting a solution, not merely selecting a visible contact and firing. Detailed equipment remains for ship System review.

G017/G020 already keeps useful broad contacts and independent focused scanning; G049 allows weapon-dependent effectiveness and G051 actual interception coverage. Additional guidance rules must not make focused investigation a universal prerequisite for every fleet target or reduce all weapons to one fragile lock mechanic.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Decide whether existing contact quality usually supplies the engagement model or some equipment also requires a maintained fire-control/illumination/guidance solution. Requirements differ by weapon; independent seekers and unguided weapons can offer alternatives. |
| Activity | Choose targets, allocate available tracking/support, expose or shelter the ship, and respond when a supported solution degrades. Routine acquisition/maintenance can be delegated as a whole System. |
| Interactions | Helm changes geometry/occlusion; Sensors provides evidence; weapon/support Systems establish useful solutions; Engineering supports their power/thermal demands. Broad/focused scanning remains independent under G020. |
| Complexity and AI | Beginners can choose targets while ordinary individual AI acquires/maintains supported solutions. Deeper operators choose priorities and alternative methods; no manual lock ritual is required. AI uses available evidence and communication. |
| Communication and pillars | Changes in geometry, tracking capacity or guidance can justify requests and timing decisions across Stations, without needing a spoken acknowledgement for every shot. |
| Risks | Universal lock bottlenecks, one operator marking every contact, hidden launch failures, guidance micromanagement and unfair target knowledge. |
| Dependencies | G017–G021 evidence, G027 movement, G048/G049 weapon choices, G051 defence, G101 assistance and later weapon/Sensor/fire-control Systems. Exact capacities, solution states, loss behaviour and ownership remain open. |

### Resolved choice: should some weapons need maintained engagement support beyond detecting a contact?

| Option | Form | Trade-off |
|---|---|---|
| A — Contact quality is usually sufficient | Weapons chiefly use available contact quality and geometry; avoid another maintained fire-control/guidance layer. | Easier fleet targeting and fewer support dependencies, but fewer ways to disrupt or coordinate an engagement. |
| B — Weapon-specific support chains | Some equipment requires maintained tracking, illumination or guidance, while other weapons use independent seekers, simpler observations or unguided fire. | Adds equipment variety and cross-Station choices, but requires readable support states and competent routine AI. |

**Selected: B.** Some equipment requires maintained tracking, illumination or guidance, while other weapons use independent seekers, simpler observations or unguided fire. Ordinary individual System AI handles routine acquisition/maintenance while operators choose targets, support priorities and exposure. No universal manual lock or focused-scan prerequisite is adopted. Exact hardware, capacity and support-loss behaviour remain open for ship System review.

## G106 — Bearing triangulation / multisensor hypothesis testing

**Inclusion recommendation: support complementary observations and useful investigation of genuine ambiguity, with automatic plotting.** [R6's HighFleet/Wolfpack/Captain Sonar comparisons](research/comparable-games-and-falling-frontier.md) distinguishes bearings, search/tracking, motion estimates and evidence from another operator's observations. The transfer is to uncertainty that can be reduced through action, not mandatory arithmetic or instrument-copying.

G017 already supplies broad routine information without examining every fleet contact; G018/G019 separates finding confidence and prediction from truth. G020 retains independent broad/focused sensors. The question here is whether ambiguous observations collapse to one best guess or preserve inspectable alternative explanations for selective investigation.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Sensors automatically plots/filters available observations. Decide whether unresolved associations or motion/identity hypotheses are shown as one estimate or retained as a small set of alternatives when meaningful. |
| Activity | Choose a useful observation, reposition for another bearing, compare emissions/reports or request corroboration. Focus on what to investigate rather than calculating intersections by hand. |
| Interactions | Helm changes observation geometry; other assets can contribute shared observations under G022; Comms adds claims with provenance. A single bearing does not secretly provide exact range, identity and velocity. |
| Complexity and AI | Routine clear contacts remain automatic. A simpler preset/ordinary System AI can pursue a useful next observation; deeper inspection exposes evidence and ambiguity without forcing every contact through manual association. |
| Communication and pillars | Operators can share why they favour an explanation and request evidence that would distinguish it. Contradiction should prompt investigation rather than arbitrary certainty or compulsory puzzle solving. |
| Risks | Crowded radar, perpetual uncertainty, misleading false precision, every contact requiring interpretation and hypotheses generated from hidden ground truth rather than observations. |
| Dependencies | G017–G024 evidence/sharing, G103 interpretation, G105 engagement support, G109 reports and later sensor/fire-control Systems. Exact hypothesis bounds, fusion/association rules and display remain open. |

### Resolved choice: how should the system present genuinely ambiguous observations?

| Option | Form | Trade-off |
|---|---|---|
| A — One best estimate | Automatically select the most plausible association/interpretation and show its uncertainty, with underlying observations available for inspection. | Keeps the picture simple, but can conceal a competing explanation worth testing. |
| B — Selective competing hypotheses | Keep routine contacts automatic, but expose a few plausible alternatives for meaningful ambiguity and let further evidence resolve them. | Creates investigative choices and useful corroboration, but needs a clear display and restrained scope. |

**Selected: B, with Viewscreen wireframes and scanner confidence radii.** Keep routine contacts automatic; expose a few plausible alternatives for meaningful ambiguity. Ambiguous contacts appear as wireframes on the Viewscreen, and scanners can present a confidence radius refined over time as evidence improves. Position uncertainty is distinct from identity/condition confidence under G018; a wireframe must not reveal an unknown hull's true geometry. Automatic plotting and ordinary individual System AI remain. Exact fusion, refinement, stale-track behaviour and alternative-hypothesis presentation remain open.

## G107 — Preparation, commitment and recovery windows

**Inclusion recommendation: support preparation and deliberate timing in real time; resolve whether selected actions can be staged.** [R6's Battlestar Galactica Deadlock comparison](research/comparable-games-and-falling-frontier.md) inspires preparation, commitment and observation, without importing turns or a required global pause. G009/G061 already gives preparation margins and deployed operations consequences; G105 adds weapon-specific support that can make timing useful.

This review concerns optional staging, not a compulsory ship-wide combat cycle. Systems keep their own operation/recovery rules, and operators retain local authority. Continuous AI phaser fire or an immediate emergency response need not wait for everyone to ready an action.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Decide whether preparation chiefly means adjusting settings/resources before immediate commands, or selected Systems can also ready a concrete action and hold it for release. |
| Activity | Prepare reserves, target/support and equipment; communicate readiness; choose a useful commitment moment; recover or revise when conditions change. |
| Interactions | A staged salvo or deployment can coordinate with geometry, guidance, protection or team readiness. Actual resources and supported conditions still determine execution. Staging does not create free preparation or guarantee success. |
| Complexity and AI | Immediate/continuous operation remains available. Whole-System AI can handle routine preparation/execution under ordinary orders and authority; no special temporary Backfill behaviour is introduced. |
| Communication and pillars | Readiness reports and changing conditions can justify coordination, but no mandatory spoken approval or central Captain launch control is added. |
| Risks | Rote ready/fire cycles, frozen staged actions ignoring changed context, unlimited stored readiness, one operator directing every action and preparation leaving others waiting. |
| Dependencies | G003/G004 ordinary AI, G009 reserves, G048/G105 targeting/support, G061 recovery, G079 procedures and later weapon/operations Systems. Exact supported staged actions, costs, cancellation and release conditions remain open. |

### Resolved choice: should selected Systems support staging an action for later release?

| Option | Form | Trade-off |
|---|---|---|
| A — Prepare settings, then act immediately | Prepare resources/settings, but concrete fire/deploy commands execute when issued and feasible; no separate staged-action state. | Fewer action states and straightforward controls, but less support for coordinating prepared actions. |
| B — Optional staged actions | Selected Systems can ready an action and hold it for release by their operator or an authorised ordinary-AI condition, alongside immediate/continuous operation. | Supports coordinated timing and preparation, but needs readable readiness, cancellation and resource rules. |

**Selected: B.** Selected Systems can ready an action and hold it for release by their operator or an authorised ordinary-AI condition, alongside immediate and continuous operation. Keep local authority, actual resource/condition requirements and ordinary Backfill. No universal ready/fire cycle or required global pause is adopted. Exact supported actions, costs, cancellation and release conditions remain open for ship System review.

## G108 — Located resources, delivery, interception and service dependencies

**Inclusion recommendation: connect supply to routes, time and actual stocks; decide the level of delivery simulation.** [R6's Carrier Command 2/Foxhole comparisons](research/comparable-games-and-falling-frontier.md) inspires protected supply routes and physically transported resources. Falling Frontier's logistics material is advertised design, not a playtested reference. The proposed transfer is a few consequential deliveries that change bridge options, rather than importing production labour or a galaxy-wide convoy simulation.

G038 already chooses real stocks/needs within a bounded actor model; G065/G067 defines stores and replenishment; G083/G090 makes consequences and continuing stocks meaningful. The open issue is how much routine traffic to simulate as physical vessels rather than coarse stock/route events. Both choices must respect resource availability, access and travel/delivery time.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Represent resources at locations with supported transfers. Compare physical vessels for every inter-location delivery with physical scenario-relevant shipments plus coarse routine background flows. |
| Activity | Discover a shortage or blocked route, protect/reroute a shipment, substitute onboard stock, repair locally or change the goal. Routine transport execution belongs to ordinary actor/System AI. |
| Interactions | Actual delivery can enable repair, ammunition supply, coolant replenishment or service restoration. Sensors/Comms supplies supported route reports; Navigation and defence choose feasible protection. |
| Complexity and AI | Low-depth operators receive useful shortage/blocked-delivery information rather than dispatching every routine shipment. AI keeps existing objectives, channels and limited knowledge; no central logistics planner is implied. |
| Communication and pillars | A shipment's availability can change Engineering's estimate, Comms' supported promise and the route/protection priorities. Different interventions should remain possible. |
| Risks | Invisible supplies appearing on demand, endless escort chores, irrelevant civilian congestion, arbitrary arrival timers and coarse updates contradicting a vessel already observed/intercepted. |
| Dependencies | G033 traffic, G035/G036 services, G038 stocks, G041 commitments, G065–G067 supply, G083 chains, G090 persistence and G112 actors. Exact relevance/detail boundaries, route rules and transfer rates remain open. |

### Resolved choice: how many deliveries should be simulated as physical vessels?

| Option | Form | Trade-off |
|---|---|---|
| A — All deliveries in the active sector | Inter-location resource deliveries use physical vessels, including routine background trade. | Consistent opportunities to observe/intercept every shipment, but more traffic, simulation cost and logistics detail. |
| B — Consequential shipments in detail | Physical vessels carry scenario-relevant shipments; routine background flows use coarse stock/route/time events. | Focuses attention on useful deliveries, but requires consistent boundaries so coarse flows respect access and cannot contradict observed events. |

**Selected: A.** Every inter-location resource delivery in the active sector uses physical vessels, including routine background trade. Actual stocks, access, travel and delivery constrain services; ordinary actor/System AI runs routine transport. This goes beyond only consequential shipments while retaining a bounded active sector, not a whole-galaxy simulation. Exact traffic scale, routes, outside-sector boundaries and delivery rules remain open.

## G109 — Reports, requests and verified versus claimed state

**Retained under selected knowledge/report rules; no additional choice required here.** [R6's Radio Commander comparison](research/comparable-games-and-falling-frontier.md) motivates source/time-aware reports. G018/G019/G022/G026 already preserves confidence, observed versus predicted information, provenance and automatic factual history. Treat an actor's claim as a report, not automatic verified truth; Sensors, Comms and other observations can corroborate or contradict it. G096 permits authored deception with investigable clues even without a fully simulated source. Do not add a universal verification chore, manual transcription or the assumption that every distress call is deceptive. Detailed request/report controls remain for ship System review.

## G110 — Manoeuvre and operation effects on readiness / faults

**Inclusion recommendation: keep manoeuvre effects causal and assess whether they extend to onboard work.** [R6's Captain Sonar comparison](research/comparable-games-and-falling-frontier.md) motivates one operator's route choices changing another's preferred response. Phoenix has already rejected importing arbitrary damage per move: G049/G105 uses geometry/support, G057/G059 component conditions, and G060 stress-dependent wear.

The unresolved question is whether selected onboard activities are also affected by physically relevant manoeuvres. External towing/docking already has its own geometry constraints; this question does not replace those rules or require Engineering to approve steering.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Preserve propulsion resource/heat costs and relevant component stress. Decide whether severe manoeuvres can also slow, interrupt or change risk for selected repair/personnel/material-handling work. |
| Activity | Anticipate a readiness conflict, request a useful course/speed change, accept a cost or find another way to perform the work. Ordinary steering must not become a stream of repair vetoes. |
| Interactions | Helm weighs immediate movement needs against reported work constraints; Engineering/Operations explains affected work and alternatives. External operations still obey their actual geometry/connection conditions. |
| Complexity and AI | Ordinary System AI can report and react to supported constraints using existing channels/orders. No hidden ship-wide manoeuvre permission gate, new AI planner or artificial per-turn damage is introduced. |
| Communication and pillars | A request such as holding a steadier course briefly can create a real trade-off while Helm retains authority. Routine cruising should leave normal work viable. |
| Risks | Arbitrary penalties, nagging Helm for every turn, disabling all work during combat and multiplying maintenance merely to create chatter. |
| Dependencies | G011 authority, G027/G032 movement, G049/G105 solutions, G058/G060 repair/stress, G061 recovery, G070 teams and G107 staged actions. Exact affected work, thresholds, warnings and recovery remain open. |

### Resolved choice: should severe manoeuvres affect selected onboard operations too?

| Option | Form | Trade-off |
|---|---|---|
| A — Movement and equipment effects only | Manoeuvres affect propulsion costs, component stress and external geometry; onboard team work does not receive an additional manoeuvre penalty. | Keeps ordinary repair/personnel work predictable, with fewer cross-Station restrictions. |
| B — Selected work has physical constraints | Severe manoeuvres can affect particular onboard repair, team or material-handling operations where the fiction supports it; routine movement remains forgiving. | Adds useful readiness/route trade-offs, but needs clear limits so Helm retains freedom and work remains possible. |

**Selected: B, limited to selected mission-critical work.** Use onboard manoeuvre constraints for selected mission-critical operations: surgery requiring a steady course should be a major plot beat, not ordinary healing of a crew member. Do not generalise these constraints to routine repair, treatment or material-handling work. Helm retains authority; existing propulsion costs, equipment stress and external geometry rules remain separate. Exact eligible operations, thresholds, warnings and consequences remain open.

## G111 — Throughput, queues, buffers and shifting bottlenecks

**Retained under selected reserve/inventory/delegation rules; no additional general choice required.** [R6's PlateUp! comparison](research/comparable-games-and-falling-frontier.md) motivates buffers buying time and automation changing bottlenecks. G003/G008/G009 already requires worthwhile agency, whole-System delegation and prepared margins; G065/G067 selects target stocks and automatic replenishment. Use real throughput and queues where the relevant System needs them, with legible consequences. Delegation should reduce personal burden; do not manufacture equivalent busywork elsewhere. Exact rates, queue policies and physical bottlenecks belong in ship System review, not a universal ship-wide queue mechanic here.

## G112 — Autonomous outside actors and bounded competing crises

**Inclusion recommendation: retain bounded actors pursuing their own objectives; resolve whether other actors can complete work assigned to the crew.** [R6's X4/Drox Operative 2/Space Rangers comparisons](research/comparable-games-and-falling-frontier.md) motivates independent actors and coexisting conflicts. G033/G038/G039/G083 already selects purposeful traffic, needs, objectives and reusable causal interactions; G108 makes deliveries physical in the active sector.

The open issue is whether accepting an assignment reserves its resolution for the player ship. Another actor might reach a rescue, deliver supplies or repair a facility first. That requires actual supported work, knowledge, resources and access; it is not automatic completion on a timer. Patrol can still issue a main priority and War can retain its fixed finale.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Keep actor population and relevant goals at scenario scale. Decide whether assigned tasks are normally protected from outside completion or remain open to other capable actors. |
| Activity | Discover what others intend, coordinate or redirect help, select a useful complementary intervention and react to changed needs. Autonomous progress should not leave the crew with no worthwhile work. |
| Interactions | Generic requests/Comms can reveal supported intent; actors can accept/refuse or need help. Stocks, teams, transport and physical operations govern their outcomes under the same causal rules. |
| Complexity and AI | Outside actors use ordinary individual System AI and communication, not an omniscient sector planner. Personal presets do not make the world wait for a human. |
| Communication and pillars | Sharing intent can avoid duplicated effort or support joint work; competing priorities create useful decisions without making every actor a hostile rival. |
| Risks | NPCs completing all interesting work, fake independent activity, reserved targets preventing credible help, hidden offscreen failures and unbounded competing crises. |
| Dependencies | G022 requests, G033/G038/G039 actors, G063 bridges, G079/G080 objectives, G083/G084 emergence/pacing, G086–G088 modes and G108 delivery. Exact actor scope, assignment updates and authored exceptions remain open. |

### Resolved choice: can another actor resolve an assignment before the crew does?

| Option | Form | Trade-off |
|---|---|---|
| A — Usually reserve assigned work | Once assigned, the key resolution normally waits for the player crew; other actors mainly assist or generate context. | Protects planned player activity, but makes independence and outside help more limited. |
| B — Actual work determines resolution | Other capable actors can complete or change the need, even after assignment; update goals and leave worthwhile alternatives. Authored exceptions remain possible. | Supports a living situation and genuine cooperation, but needs clear reports and protection against player activity being displaced wholesale. |

**Selected: B, connected to existing multi-ship/faction Objective assignment.** Other capable actors can complete or change an assigned need through actual supported work, with updated goals, useful complementary work and authored exceptions where appropriate. Keep actor goals at scenario scale; independence should create choices rather than displace all worthwhile crew activity. Exact outside-actor completion/update rules remain open.

**Verified related Current contract:** [Objective instances](../../pasm/spec/architecture/objectives.yaml) and [the implementation index](../../wiki/concepts/objectives.md) define named instances with independent progress/lifecycle. Recipients match authored ship slots, live faction membership or all player ships, with explicit slot > faction > all precedence; equal-specificity conflicts are refused. Completion captures effective members once; later membership changes do not reopen an instance or replay credit/effects. Recipients are distinct from subject targets. This supplies assignment, visibility and credit semantics, not an automatic rule that every outside actor's work completes every matching instance. Physical facts must satisfy the relevant objective conditions; effects on other instances may instead change needs or goals. Preserve these distinctions when designing the new completion/update behaviour.

## G113 — Telegraphing intent and preventive counterplay

**Inclusion recommendation: give discoverable developing threats enough detail for preventive choices, without revealing hidden plans.** [R6's Into the Breach comparison](research/comparable-games-and-falling-frontier.md) inspires prevention, displacement and protection alongside damage. Phoenix remains real time with evidence-derived knowledge; it does not import perfect turn-based attack previews.

G025 already leaves danger recognition to ordinary player tools and Captain alert-setting. G018/G106 provides confidence and ambiguity, G105 equipment-specific guidance, and G081 actual consequential actions. Resolve how precise an intent forecast can become when the crew has earned good evidence.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Telegraph observable preparation and supported consequences. Decide whether normal forecasts remain broad intent categories or can include a specific likely target/action/time window when evidence warrants it. |
| Activity | Recognise a developing action, obtain better evidence and choose prevention, displacement, protection, negotiation or accepted risk. Do not reduce every response to maximising damage. |
| Interactions | Sensors interprets observable preparations; Comms adds supported orders/claims; Helm/defence/weapon operators choose feasible responses. Engineering prepares needed reserves; Captain coordinates from reports. |
| Complexity and AI | Ordinary individual System AI can respond to available evidence under its usual policies. Better manual interpretation does not require deliberately blind AI or a global predictive planner. |
| Communication and pillars | Useful detail supports timing/resource requests; uncertainty leaves room to compare evidence and revise a plan. A forecast does not automatically set alert condition. |
| Risks | Omniscient target previews, false precision, unavoidable attacks dressed as warnings, excessive alerts and forecasts presented as guarantees despite changing conditions. |
| Dependencies | G018/G025 evidence/danger, G049/G051 counterplay, G081 commitments, G105–G107 support/ambiguity/staging and later Sensors/EWar/weapon Systems. Exact observable cues, forecast models and update rules remain open. |

### Resolved choice: how detailed can an earned threat forecast become?

| Option | Form | Trade-off |
|---|---|---|
| A — Broad intent | Usually show categories such as preparing to attack, retreat or deploy; players infer specific targets/timing from ordinary observations. | Less predictive UI and more interpretation, but can make precise preventive action difficult. |
| B — Evidence-dependent specific forecasts | With sufficient evidence, show a likely action/target and approximate time window, retaining uncertainty and changing as conditions change. | Supports concrete prevention and coordination, but requires clear confidence and no access to hidden plans. |

**Selected: B, through observable preparation and inference.** The crew can see signs such as weapon batteries charging and infer the likely threat from there. More useful evidence can support a more specific interpretation, but this does not adopt an automatic exact target/timer overlay or reveal private enemy plans. Preserve uncertainty and changing conditions; ordinary System AI uses only available evidence. Exact visible cues, confidence and interpretation remain open for ship System review.

## G114 — Persistent settings, explicit work ownership and temporary handoffs

**Retained under existing ownership/Backfill decisions; no additional choice required.** [R6's Sea of Thieves comparison](research/comparable-games-and-falling-frontier.md) motivates stable settings and complementary information. G001–G004/G009–G011 already establishes dedicated human/AI Stations, persistent viable operation, whole-System delegation and ordinary Backfill. Temporary handoffs use normal orders, alert, mission context and ordinary hysteresis; AI may change settings appropriately. Show ownership and meaningful changes, without a unique temporary-fill policy or blanket restoration of prior settings. Do not require physical role swapping or combined manual Stations.

## Ship System review

The first pass over G001–G114 is complete. Selected directions and dependencies remain provisional until combined into coherent Station experiences; ship-specific controls, costs and interactions are now reviewed in S001–S102 order. This does not promote the proposal to PASM or implementation planning.

## S001 — Phasers / sustained beam banks

**Inclusion recommendation: retain direct-fire beam banks as a distinct weapon family; resolve the firing-duration model.** [The Current weapons baseline](../gdd/mechanics/targeting-weapons.md) uses an authored active beam period followed by per-bank cooldown, subject to readiness/range/arc. This is Current behaviour; variable-duration operation below is a redesign option, not an implementation claim.

G048 permits primary and individual weapon-System targets, G049 graduated equipment-dependent effectiveness, G050 optional shield analysis, and G105 specific support requirements. The selected local-battery/heat/coolant model can make maintaining a beam a resource decision. Phasers need not become universal long-range weapons or require manual frequency tuning every shot.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain authored beam-bank layouts/arcs and meaningful variants. Compare fixed-duration beam attacks with beams whose duration can be extended or curtailed within actual operating limits. |
| Ongoing activity | Monitor target/geometry, reserves and heat; communicate useful exposure or power needs. Ordinary AI can handle aiming/firing under target and policy constraints. |
| Active decisions | Choose when to open fire, stop, conserve reserve or take another opportunity. Staging can be supported where appropriate under G107. |
| Complexity and AI | Simple operation is target choice plus whole-System auto-fire; deeper control can expose firing/conservation policy. Exact personal presets remain to be designed, not separate per-System complexity overrides. |
| Interactions and communication | Helm presents firing geometry, Sensors/support contributes a usable solution and Engineering supports output. No compulsory shot-by-shot request/approval. |
| Risks | A permanent hold-to-fire chore, all banks exhausting reserves automatically, beam damage without maintained geometry and beams overshadowing projectiles/missiles. |
| Dependencies | Local power/heat model, G048–G050/G105/G107, S031–S039 resources and later shields/fire-control. Exact range, duration, output, interruption and cooldown remain open. |

### Resolved choice: should normal beam attacks have fixed or variable duration?

| Option | Form | Trade-off |
|---|---|---|
| A — Fixed active period | A fire command runs the bank's authored beam period, then cooldown; timing focuses on opening the attack. | Clear commitments and familiar readiness cycles, but less control over sustaining an opportunity. |
| B — Variable sustain | Beams can continue or stop within geometry, local-energy and thermal limits; operators/ordinary AI choose when to sustain or conserve. | Connects sustained exposure and power support to firing decisions, but needs good auto-fire/conservation policies and readable limits. |

**Selected: B.** Beam attacks can continue or stop within geometry, local battery and thermal limits. Ordinary individual System AI handles delegated aiming/firing and conservation; operators choose targets, timing and policy without a mandatory hold-to-fire chore. Exact bank variants, output, interruption and recovery limits remain open. This is a proposal change from Current authored active periods and cooldowns, not an implementation claim.

## S002 — Blasters / unguided projectile banks

**Inclusion recommendation: retain unguided direct-fire projectiles, distinct from sustained beams and guided missiles; resolve the equipment family's resource variants.** [The Current weapons baseline](../gdd/mechanics/targeting-weapons.md) describes predicted straight-line volleys with authored projectile speed, damage, reach, cooldown and optional charge. [The Current admitted control path](../../crates/phoenix-simulation/src/console/weapons/blaster.rs) has power/charge gating. Adding ammunition-fed variants below is Proposal, not a claim about Current blaster magazines.

G049 makes distance/motion/solution quality matter where appropriate; G054/G065 gives ammunition production/stocks meaning. An unguided shot's travel time, cover and target motion can distinguish it from a beam without requiring manual aim. PDC/broadside roles remain separate reviews; this entry does not make every projectile bank suitable for every role.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Keep authored projectile banks and lead/volley behaviour. Decide whether this family contains energy bolts only or also conventional ammunition-fed kinetic variants with distinct operational costs. |
| Ongoing activity | Monitor useful range/lead, contact quality, readiness and resources. Ordinary AI calculates lead and operates a delegated bank. |
| Active decisions | Choose target, volley timing, charge where supported, and whether ammunition/reserve use is worthwhile. Staged volleys can follow G107 without a universal charge minigame. |
| Complexity and AI | Beginners choose targets and delegate firing; deeper operators choose policies, shot/volley timing and reserves. No manual trajectory arithmetic is required. |
| Interactions and communication | Helm supplies a useful firing pass; Sensors/support reduces uncertainty; Engineering supports energy/heat; Inventory supplies physical rounds where fitted. |
| Risks | Projectile misses looking arbitrary, energy/kinetic variants differing only cosmetically, ammunition chores and every bank acquiring the same charge/heat/stock controls. |
| Dependencies | G049/G054/G065/G105/G107, S003/S004 ordnance, S006 charge and S007/S008 battery/defence roles. Exact projectile variants, resource costs and charge/volley rules remain open. |

### Resolved choice: should unguided projectile banks include kinetic variants too?

| Option | Form | Trade-off |
|---|---|---|
| A — Energy bolts | Keep blasters as energy-based projectiles, with battery/heat costs; consider ammunition-fed guns in their specialised weapon entries. | A clear family identity and fewer resource variants, while flight/lead already distinguishes them from beams. |
| B — Energy and kinetic equipment | Support energy-bolt and ammunition-fed kinetic banks using shared lead/volley concepts but different resource constraints. | Adds fitting/logistics choices, but variants need meaningful differences and suitable roles. |

**Selected: B.** Support energy-bolt and ammunition-fed kinetic banks using shared lead/volley concepts with distinct resource constraints. Kinetic banks consume physical rounds as well as operating power. Ordinary individual System AI handles routine lead/firing; authored equipment roles determine suitable use, with neither type mandatory on every hull. Exact variants, costs and charge/volley rules remain open.

## S003 — Guided torpedoes / missiles

**Inclusion recommendation: retain guided ordnance with meaningful launch, support and defence interactions; resolve post-launch control.** [The Current baseline](../gdd/mechanics/targeting-weapons.md) has guided proximity torpedoes, tube arcs/reloads and a shared magazine. Payload/damage properties are captured at launch. C5 payload sidegrades remain Plan. [R6's NEBULOUS comparison](research/comparable-games-and-falling-frontier.md) inspires guidance/support and counterplay, not a required missile-first combat sequence.

G027 gives small craft/torpedoes full 3D movement; G051 allows actual interception coverage; G105 selects equipment-specific guidance, including independent alternatives. Post-launch instructions can be distinct from changing the payload's physical properties. This review does not settle all seeker, payload, flight or magazine details.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Keep guided weapon variants and salvo choices. Decide whether mission/target intent is normally fixed at launch or suitable ordnance can receive later instructions over an actual supported link. |
| Ongoing activity | Monitor in-flight salvo/support status and ammunition commitments. Ordinary AI handles flight/guidance and routine firing under operator intent; humans need not steer every missile. |
| Active decisions | Choose target, launch geometry, payload and salvo timing; post-launch control could add retarget/abort or supported route changes when circumstances change. |
| Complexity and AI | Simple target-and-auto-launch remains useful. Optional deeper salvo instructions use whole-System assistance and available evidence, not omniscient projectile targeting or a second manual piloting duty. |
| Interactions and communication | Sensors/fire-control supplies usable evidence, Helm shapes launch geometry, defence can disrupt/intercept and Engineering supports launch/guidance. Actual link capability matters to remote instructions. |
| Risks | Guaranteed retargeting despite broken links, missiles gaining damage from later ship changes, compulsory per-round clicks, indefinite pursuit and guided ordnance displacing other weapon roles. |
| Dependencies | G027/G048/G051/G054/G105/G107, S004 magazines, later seekers/EWar/payloads and S031–S039 resources. Exact command scope, links, endurance and loss behaviour remain open. |

### Resolved choice: should suitable missiles accept post-launch instructions?

| Option | Form | Trade-off |
|---|---|---|
| A — Intent fixed at launch | Select target/behaviour before launch; ordnance follows its configured guidance and seeker rules without later operator changes. | Clear commitment and simpler controls, but less flexibility as the tactical situation changes. |
| B — Link-dependent changes | Suitable ordnance can accept retarget/abort or supported route instructions while its command link works; independent/fixed-intent variants remain available. | Adds adaptation and link counterplay, but needs restrained salvo-level controls and readable communication limits. |

**Selected: B, with longer-range, slower torpedoes for layered combat.** Suitable ordnance can accept post-launch instructions while an actual command link works, with independent/fixed-intent variants still available. Preserve physical payload properties at launch and ordinary individual System AI execution. For the torpedo/PDC/broadside combat model, torpedoes should be much longer-ranged and slower than Current, leaving meaningful time for detection, interception and adaptation. This reference model does not impose a mandatory sequence on every ship. Exact commands, link-loss behaviour, speed and endurance/range remain open.

**Verified comparison baseline:** [Alliance Cruiser](../../assets/entities/alliance_cruiser.toml) and [Alliance Battleship](../../assets/entities/alliance_battleship.toml) currently author torpedo speed `15` world units/second and lifespan `20` seconds. [The projectile model](../../crates/phoenix-sim-gameplay/src/weapons/torpedo.rs) defines lifespan as expiry time and speed as travel speed. Slower travel with greater reach therefore requires reviewing endurance as well as speed; these values are recorded for comparison, not changed here.

## S004 — Magazine, tube loading and reload

**Inclusion recommendation: retain finite magazines, prepared volleys and ordinary automatic loading; resolve shared loading capacity.** [The Current baseline](../gdd/mechanics/targeting-weapons.md) has a shared magazine and per-tube reload state. Current hull authoring also supports tube volley capacity/targets. G065 already establishes target local stocks and timed automatic replenishment; G054 allows ammunition fabrication/restocking. Operators should not have to manually transfer every round.

S003's longer flight window and G107 staged actions makes salvo preparation meaningful. Decide whether tube load times are chiefly independent or whether groups of launchers compete for finite handling/loading capacity. This is a physical throughput rule, not a generic queue imposed on every ship System.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Keep magazine stocks and authored tube capacity/readiness. Compare independent per-tube loading with shared loader capacity within authored groups. |
| Ongoing activity | Monitor ready salvo, remaining stocks and reload progress. Ordinary AI keeps requested readiness using available rounds and capacity. |
| Active decisions | Choose payload/desired volley and reserve policy; shared capacity could add which tube/group to prepare first after a launch or fault. |
| Complexity and AI | Simple presets delegate load scheduling from targets/priorities. Deeper operators can change those priorities without a round-by-round loading task. |
| Interactions and communication | Inventory supplies available rounds; Engineering supports machinery; damage can affect actual loader/tube components. Different arcs can change which prepared salvo helps Helm's next pass. |
| Risks | Loading click chores, invisible queues, every salvo needing manual approval, one universal loader disabling all ordnance and independent reloads making extra tubes an unlimited throughput gain. |
| Dependencies | G054/G065/G107/G111, S003 ordnance, S010+ payloads and later component/resource Systems. Exact grouping, throughput, payload-change costs and priorities remain open. |

### Resolved choice: should launcher groups share loading capacity?

| Option | Form | Trade-off |
|---|---|---|
| A — Independent tube reloads | Each tube loads at its own authored rate, limited by available stock and its own condition. | Straightforward readiness and fewer scheduling decisions, but adding tubes also adds reload throughput. |
| B — Shared loaders where fitted | Authored launcher groups share finite loading/handling capacity; priorities determine which tubes are readied first. Independent-loader designs remain possible. | Creates salvo/reload trade-offs and equipment variation, but needs legible queues and competent automation. |

**Selected: B.** Authored launcher groups share finite loading/handling capacity where fitted, with priorities determining which tubes reload first. Independent-loader designs remain possible. Ordinary individual System AI handles routine scheduling and requested readiness from actual stocks. No compulsory single ship-wide loader or round-by-round loading chore is adopted. Exact grouping, throughput, damage and payload-change behaviour remain open.

## S005 — Weapon tuning and beam frequency

**Retained under G050; no repeated choice required.** Optional shield probing with suitable scanners can support weapon tuning where fitted equipment benefits. The usual advantage is modest; special shields or penetrating phasers can make it much stronger. Findings generally persist for the observed configuration; selected enemy equipment may rotate frequencies. Ordinary System AI can perform delegated tuning. Do not add universal frequency guessing, recurring retune chores or switching costs previously left unselected. Current frequency behaviour and planned deeper calibration remain distinct from this proposal; exact controls belong in the relevant weapon/shield review.

## S006 — Weapon-bank charge, heat and discharge

**Inclusion recommendation: make readiness and discharge use the selected power/thermal model; resolve whether a separate energy store is needed.** [The Current baseline](../gdd/mechanics/targeting-weapons.md) includes optional blaster hold-to-charge; [R1 #24](research/feature-gap-analysis-master.md) discusses split charge/firing work. The user has already selected Engineering's flow allocation, local System batteries with fullness-dependent losses, and heat from charging/depletion. A separate operator charging weapons is not a settled Station role.

S001/S002 selects sustained beams and projectile variants; G107 permits staged actions. Charge preparation can remain an equipment-specific timed state without automatically creating another universal resource meter. Engineering retains final power allocation; weapon-side choices must respect available flow/reserves.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Decide whether weapon reserves use the existing local System battery or selected equipment adds a separate shot capacitor alongside it. Keep charge timing and energy storage conceptually distinct. |
| Ongoing activity | Monitor reserve/thermal readiness and conserve or prepare for expected use. Full readiness has the selected inefficiency/thermal costs rather than a free indefinite charge. |
| Active decisions | Stage a supported shot/volley, choose when to discharge, cancel or conserve. Exact timing/output trade-offs depend on equipment. |
| Complexity and AI | Whole-System AI manages ordinary readiness/firing; deeper controls expose useful policy and timing. Do not split charging and firing into mandatory separate human Stations. |
| Interactions and communication | Engineering supplies flow; weapon use changes local levels/heat. Requests to prepare reserves can occur ahead of an opportunity without a shot-by-shot approval ritual. |
| Risks | Duplicate meters, separate charge minigames for every bank, hidden energy creation on cancellation, weapon controls overriding Engineering and readiness costs that make stable preparation impossible. |
| Dependencies | Selected local power/heat model, G056 no extra flux, G107 staging, S001/S002 weapons and later S031–S039 resources. Exact charge timing, cancellation, thermal limits and bank/System ownership remain open. |

### Resolved choice: should weapons add a separate shot-energy store?

| Option | Form | Trade-off |
|---|---|---|
| A — Reuse local batteries | Use local System battery reserves for weapon energy; equipment can still have timed shot preparation without another energy reservoir. | Fewer overlapping resources and direct coordination with Engineering's allocation. |
| B — Separate capacitors where fitted | Some weapons charge a dedicated shot capacitor from their local battery, adding another preparation/discharge constraint. | Can distinguish specialised weapons, but adds storage, losses and controls on top of the selected battery model. |

**Selected: A.** Weapon energy reserves use local System batteries without a separate shot-energy capacitor layer. Equipment can still have timed preparation, staging and particular discharge/recovery rules. Engineering retains flow allocation; ordinary individual System AI handles delegated readiness/firing. Exact timing, cancellation and thermal behaviour remain open.

## S007 — Broadside batteries

**Inclusion recommendation: retain broadside batteries on suitable hulls under G052; resolve how strongly hull facing constrains them.** [R6's Rebel Galaxy comparison](research/comparable-games-and-falling-frontier.md) inspires aspect-dependent capital combat with automated gunnery. Current authored bank arcs provide a foundation, not a completed redesign broadside System. S002 now supports energy/kinetic projectile equipment; broadside is a layout/operating role, not necessarily a new damage type.

G052 already keeps this style equipment-dependent and Helm in control. The user's torpedo/PDC/broadside reference suggests close engagement after longer-range ordnance, without making that sequence compulsory. Strong side arcs can make a pass or parallel course worthwhile, while wider traversal can reduce the steering demand.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Author side-mounted battery layouts and range/resource profiles. Decide whether their strongest fire requires a distinct side-on aspect or has broadly forgiving turret coverage. |
| Ongoing activity | Maintain a useful relative position, track battery readiness and the exposed side's protection. Ordinary AI solves aim/lead within the fitted arcs. |
| Active decisions | Choose a pass/course, expose a side, stage or release a battery, change target or conserve for another opportunity. No mandatory manual alignment of each barrel. |
| Complexity and AI | Beginner gunnery can be target choice/auto-fire; Helm can steer with ordinary assistance. Deeper operators coordinate aspect, salvo timing and support without transferring Helm authority. |
| Interactions and communication | Facing changes which batteries and defences can act; Engineering supports discharge/recovery and Sensors supplies usable solutions. A bearing request advises Helm rather than moving the ship itself. |
| Risks | Pixel-perfect passes, steering reduced to constant weapon obedience, broadside guns behaving like all-aspect turrets and every hull being forced into the same close-range style. |
| Dependencies | G027/G048/G049/G051/G052, S001–S004/S006, later shields/Helm and G107 staging. Exact arcs, traverse, range, battery grouping and defence exposure remain open. |

### Resolved choice: how demanding should broadside firing geometry be?

| Option | Form | Trade-off |
|---|---|---|
| A — Distinct side-on aspect | Strong batteries have limited side-facing arcs/traverse, making hull orientation and relative course central to using them. | Creates clear Helm/gunnery coordination, but needs forgiving enough windows for practical play. |
| B — Broad turret coverage | Side batteries track over wide arcs, with positioning mainly affecting range, cover and defence exposure. | Easier to keep guns active, but the broadside role depends less on deliberate hull facing. |

**Selected: A, with wider-arc turret variants constrained by tracking speed.** Distinct side-on geometry makes hull orientation and relative course central to broadside batteries. Turrets can have better arcs with finite tracking speed; coverage does not guarantee instant alignment or the ability to follow a close fast target. Ordinary individual System AI handles aiming and Helm retains movement authority. Exact arcs, tracking rates, battery grouping and resource/range profiles remain open.

## S008 — Point-defence cannons / PDCs

**Inclusion recommendation: include automatic point defence with meaningful coverage, tracking and saturation limits; resolve secondary offensive use.** [R6's NEBULOUS comparison](research/comparable-games-and-falling-frontier.md) inspires defence/guidance interactions. G051 already permits protection of other assets within actual coverage, competing with own-ship safety. S003's slower long-range ordnance gives interception time, while S007's finite turret tracking supplies another limit besides damage output.

Physical ammunition, local power/heat, useful detection, firing geometry and tracking can all limit a fitted PDC; their exact balance is open. A player should choose priorities and exposure rather than twitch-aim at each missile. The new question is whether the same equipment can also engage suitable ships/craft offensively.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain PDC coverage and ordinary automatic interception. Compare defence-only targeting with optional short-range offensive use of the same fitted guns. |
| Ongoing activity | Monitor defended sectors, readiness/stocks and saturation; listen for protection requests. Normal operation handles threats without selecting every incoming round. |
| Active decisions | Choose protection priorities, reserve policy, position or coverage. Offensive use could add a choice between spending readiness on a target and keeping it for interception. |
| Complexity and AI | Whole-System AI tracks/aims/fires under ordinary priorities, authority and evidence. Simple presets retain automatic defence; deeper policies can expose competing uses without compulsory manual firing. |
| Interactions and communication | Helm positions coverage; Sensors/support supplies useful observations; Engineering/Inventory supports power/heat/ammunition; other ships can request protection through Comms. |
| Risks | Perfect defence, invisible tracking limits, offensive fire exhausting safety reserves, PDCs replacing all close-range weapons and a defender required to click every missile. |
| Dependencies | G048/G049/G051/G065/G105, S002/S003/S007 and later Sensors/shields/resources. Exact interceptable threats, offensive effectiveness, priorities and tracking/ammunition rates remain open. |

### Resolved choice: should PDCs also have short-range offensive use?

| Option | Form | Trade-off |
|---|---|---|
| A — Defence role only | Use PDCs for incoming ordnance and appropriate threatening craft; ship attacks use other weapon banks. | Clear priorities and fewer ways to accidentally spend defence capacity. |
| B — Dual use where effective | Also permit fire on suitable ships/craft at short range, competing with interception for tracking, ammunition, power and heat capacity. | Adds close-range choices and equipment flexibility, but needs strong priority/reserve policies. |

**Selected: B, with a generic composable weapon model.** PDCs can also engage suitable ships/craft at short range, competing with interception for actual tracking, ammunition, power and thermal capacity. Ordinary individual System AI handles aiming/interception under priority and reserve policies. Exact effectiveness and control rules remain open.

### Selected dependency — generic weapon System and behaviour library

The user selects a generic configurable weapon System: firing arc, traversal/tracking speed, effective range, targeting rules, projectile type (beam, bolt, round, missile, etc.), projectile speed, damage amount/type (laser, physical, EMP, etc.), area of effect and which barrels fire in which combinations. Build a reusable behaviour library and mix compatible behaviours into equipment, rather than treating each named family as unrelated mechanics. Projectile form and damage type remain distinct; fields apply where the chosen behaviour needs them.

Compose resource/ammunition/loading, preparation/sustain/recovery and guidance behaviours with the already selected game rules. Retain individual AI per configured weapon System, ordinary communication/limited knowledge and shared authority/physics; no network redesign or implementation package structure is selected. S001–S008 still identifies useful roles and choices. Exact interfaces/compatibility and runtime pattern editing remain open.

### Resolved dependency: who composes barrel combinations and firing patterns?

| Option | Form | Trade-off |
|---|---|---|
| A — Authored modes, player selection | Authors compose valid combinations/patterns for fitted equipment; players select modes, targets, priorities and timing. Workshop can provide deeper composition. | Gives useful operating choices without requiring live pattern construction; variants need well-chosen modes. |
| B — Runtime composition too | Players can additionally build/edit barrel combinations and sequences during the mission within equipment constraints. | Adds fine manual depth, but needs another configuration surface and strong assistance/defaults. |

**Selected: A, with arbitrary-length authored barrel cycles.** Authors compose modes; players select them rather than constructing patterns during play. Define a list of barrel markers and an ordered cycle of arbitrary authored length, each step selecting one or more listed barrels. Support alternating twin barrels, missile racks and coordinated multi-barrel salvos, including Itano Circus-style launch patterns. Flight spread/trajectory behaviour composes separately where needed. Examples: `[[left], [right]]` alternates barrels; `[[left, right]]` fires them together. These illustrate the model, not a ratified file syntax. Exact timing, repeat/reset and unavailable-barrel handling remain open.

### Resolved dependency: what happens when a cycle step names an unavailable barrel?

Damage, empty ammunition, reload state or another real readiness condition can make a selected barrel unavailable. The pattern must respect those conditions without silently firing a substitute or creating resources. Decide whether its scheduler always makes progress, waits for the full step, or follows a mode-specific authored rule.

| Option | Form | Trade-off |
|---|---|---|
| A — Best effort | Fire ready barrels in the step and advance, skipping unavailable ones. | Maintains partial capability, but a coordinated salvo can lose its intended shape. |
| B — Wait for the step | Hold until every selected barrel is ready, then execute the step together. | Preserves exact combinations, but a destroyed barrel can block the pattern. |
| C — Authored policy | Each mode declares best-effort or all-ready behaviour, with blocked status visible. | Supports both alternating guns and coordinated salvos, but adds a small authoring rule to validate. |

**Selected: A.** Use best-effort firing cycles: fire ready selected barrels and advance past unavailable ones, including damaged, empty or reloading barrels. Do not wait for every barrel in a step or substitute unlisted barrels. Actual readiness/resources still constrain firing; exact cadence and repeat/reset remain open. The earlier authored-policy recommendation is not selected.

## S009 — Long-range kinetic guns / railguns

**Inclusion recommendation: retain heavy long-range direct-fire kinetic equipment as a configuration of the generic weapon model, and resolve meaningful recoil.** [R1 #38 and R2 weapon fitting](research/README.md) supplies research candidates; S002 already includes kinetic projectile banks. This entry describes a heavy range/velocity/resource role rather than a separate bespoke railgun framework. Generic parameters, barrel patterns, local batteries, physical ammunition and ordinary System AI still apply.

Compared with S003's slower long-range guided ordnance, a high-velocity unguided gun can offer different timing, guidance and evasion choices. Exact speed, range and effectiveness are open. Recoil is a possible additional behaviour: decide whether it stays in readiness/cost abstractions or can impart a meaningful physical impulse for fitted heavy guns.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Author suitable long-range kinetic equipment using existing projectile/damage/resource/traverse behaviours. Assess recoil as an optional reusable equipment behaviour, not a cost required on every weapon. |
| Ongoing activity | Maintain a useful solution, readiness and ammunition; ordinary AI handles lead and firing under targets/policies. |
| Active decisions | Commit a costly shot/volley, choose timing and exposure, conserve or prepare another opportunity. Physical recoil could change the value of a simultaneous versus staggered salvo. |
| Complexity and AI | Routine compensation can use ordinary Helm assistance where supported; no compulsory counter-steer minigame. Weapon and Helm operators keep their own authority and communicate useful constraints. |
| Interactions and communication | Engineering supports reserve/heat, Sensors supplies evidence, Inventory supplies rounds and Helm manages geometry. Existing mission-critical manoeuvre constraints remain limited under G110. |
| Risks | Recoil as arbitrary punishment, high-speed shots with no counterplay, more meters than decisions and repeated mandatory steering correction after every small shot. |
| Dependencies | G049/G105/G107, S002/S006/S007, generic weapon behaviours and later Helm/resource/damage Systems. Exact equipment niche, impulse magnitude and compensation remain open. |

### Resolved choice: should selected heavy guns impart meaningful recoil to the ship?

| Option | Form | Trade-off |
|---|---|---|
| A — Readiness/resource abstraction | Express heavy firing costs through battery, heat, ammunition and recovery; recoil does not meaningfully alter ship motion. | Keeps aim/movement predictable while preserving resource trade-offs. |
| B — Physical recoil where fitted | Selected heavy guns impart an authored impulse, so mounting, firing pattern and compensation can matter. | Adds another geometry/timing interaction, but must avoid routine steering chores. |

**Selected: B.** Selected heavy guns can impart a physical impulse, making mounting and simultaneous/staggered firing matter. Express this as a reusable authored behaviour in the common weapon model. Ordinary assistance can handle routine compensation where supported; do not impose a steering chore for every small projectile. Exact impulse, handling, range and resource profiles remain open.

## S010 — EMP / disruption payloads

**Inclusion recommendation: retain disruption as a generic damage/effect behaviour rather than a missile-only special case; resolve repeated-hit recovery.** [The C5 baseline](../gdd/mechanics/targeting-weapons.md) plans payload sidegrades including disruption, not a ratified catalogue of exact effects. The selected generic weapon model explicitly includes EMP damage type independent of projectile form. G057's component-condition direction includes temporary EMP failure that needs no repair; exact thresholds/resilience are still open.

Disruption should offer capability suppression alongside destruction. Component knowledge, shielding, delivery and equipment resilience can matter without revealing hidden target state. Repeated applications need a rule: simply refreshing interruption can allow prolonged suppression, while diminishing effect can create a recovery opportunity.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Define authored temporary disruption conditions on affected components, composed with delivery and damage routing. Decide the default treatment of repeated EMP hits before recovery. |
| Ongoing activity | Observe supported capability changes and manage available alternatives/reserves while affected components recover. Ordinary AI adapts through the usual evidence/context. |
| Active decisions | Choose disruption versus destructive fire, time a disabling opportunity and coordinate supported repair/rescue/boarding or withdrawal. Automatic recovery does not require a repair click. |
| Complexity and AI | Delegated weapons use ordinary targeting/policies; affected Systems use normal fallback/hysteresis and knowledge. Do not invent a unique EMP-disabled ship AI or a hidden global difficulty modifier. |
| Interactions and communication | Sensors identifies relevant capability where observable; weapon delivery/shields/resilience affects what is disrupted. Other operators can exploit or mitigate the temporary loss. |
| Risks | Permanent stun-lock, EMP dominating every weapon choice, every hit stopping the whole ship and recovery presented as arbitrary unexplained immunity. |
| Dependencies | G050/G057/G059/G105, generic damage behaviours, S003/S005 and later shield/component/security Systems. Exact routing, conditions, duration, resilience and stacking remain open. |

### Resolved choice: how should repeated EMP hits affect recovery?

| Option | Form | Trade-off |
|---|---|---|
| A — Full refresh | Eligible hits refresh interruption at full effectiveness; sustained suppression is possible if attackers can maintain delivery and resources. | Clear effects and strong suppression tactics, but risks long periods without usable capability. |
| B — Diminishing repeated interruption | Repeated hits on affected components have reduced interruption effect until they recover from that temporary resilience state. | Preserves recovery/counterplay, but needs a clear authored rule rather than unexplained blanket immunity. |

**Selected: B, with explicit debuff and resistance buff behaviours.** A component can receive an EMP debuff that refreshes on reapplication, plus a longer-duration EMP-resistance buff that stacks and reduces the duration of subsequent applications. Temporary disruption recovers without repair when the EMP debuff expires. These are reusable damage/status behaviours independent of projectile form. Exact durations, reduction formula, application ordering, stack decay/caps and shorter-refresh handling remain open; the selected direction alone does not prove a particular numerical recovery bound.

### Resolved dependency: can reapplication shorten an EMP debuff already running?

As resistance accumulates, a new application can have a shorter duration than the existing debuff's remaining time. Distinguish replacing the timer from extending it: either is implementable, but replacing can make another hostile hit restore the component sooner.

| Option | Form | Trade-off |
|---|---|---|
| A — Replace remaining time | Reset the debuff to the newly calculated duration, even if shorter than its current remaining time. | Simple reset semantics; another hit can shorten the existing interruption. |
| B — Keep the longer remaining time | Set remaining time to the greater of current remaining time and the new resistance-adjusted duration. | Reapplication cannot shorten an existing interruption; resistance still reduces the added interruption. |

**Selected: B.** Reapplication sets the EMP debuff's remaining time to the greater of its current remaining time and the new resistance-adjusted duration. Another hostile hit cannot shorten the existing interruption. Exact durations, reduction formula, stack decay/caps and application ordering remain open.

## S011 — Explosive / area-effect warheads

**Inclusion recommendation: retain area effects as reusable weapon behaviours; resolve collateral rules.** [R1 #14](research/feature-gap-analysis-master.md) offers area-blast warheads as Research, not a specific delivered C5 payload. The selected generic model already includes area of effect independent of projectile/damage type, so this entry covers explosive blasts and other fitted area effects without separate bespoke mechanics for each named warhead.

G027 retains full-3D small craft/projectiles, G051 physical interception and G081 consequential actions. Area effects can make concentration, separation and protection relevant. Target-selection rules govern what a weapon chooses to engage; decide whether those rules also make friendly entities immune to a resulting area effect.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Author suitable area shape/reach, effect and detonation rules within the generic model. Decide whether physically affected allies/civilians can suffer collateral or are excluded by faction-aware damage filtering. |
| Ongoing activity | Monitor nearby traffic, protected assets and in-flight ordnance while ordinary AI handles supported targeting/interception. |
| Active decisions | Choose a firing opportunity, wait for separation, change payload or accept an authorised risk. Area use should offer alternatives rather than force collateral as the only solution. |
| Complexity and AI | Simple policies can avoid known unsafe shots using available evidence; deeper operators assess actual risk. AI does not gain hidden exact positions to guarantee safety. No manual blast arithmetic is required. |
| Interactions and communication | Helm/Comms coordinates separation and protected routes; Sensors provides known positions/uncertainty; weapon/defence operators account for actual effects and interception. |
| Risks | Invisible collateral, magical friendly immunity, unusable area weapons in every fleet fight, uncertainty treated as certainty and one operator needing approval for every routine shot. |
| Dependencies | G018/G027/G049/G051/G081, generic damage/area behaviours, S003/S008/S010 and later shields/terrain/civilian Systems. Exact shape/falloff, cover, detonation and safety policy remain open. |

### Resolved choice: should area effects be able to harm allies and civilians?

| Option | Form | Trade-off |
|---|---|---|
| A — Physical collateral | Apply the effect to physically exposed eligible entities regardless of relationship; target/risk policies constrain firing rather than grant immunity. | Makes position and protection consequential, but requires legible effects and useful safety automation. |
| B — Friendly-safe effects | Normally exclude allies/protected civilians from area damage, even when inside the effect region. | Easier fleet use and fewer accidental losses, but less physical consistency and separation work. |

**Selected: A.** Area effects follow physical exposure and can harm eligible allies and civilians. Target/risk policies constrain firing rather than grant faction immunity. Ordinary AI avoids known unsafe opportunities under configured rules using available evidence; it does not gain hidden exact positions. Keep footprint/risk understandable and alternative actions viable. Exact shape/falloff, cover, detonation and safety handling remain open.

## S012 — Penetrating / marking payloads

**Inclusion recommendation: retain equipment-specific penetration and investigate physical marking payloads within the generic weapon model.** [The C5 baseline](../gdd/mechanics/targeting-weapons.md) lists armour penetration and marking among possible authored payload sidegrades, without selecting exact effects. G050/S005 already retain optional shield-weakness exploitation, including stronger interactions with suitable penetrating equipment. Those decisions do not establish a universal shield bypass or an exact armour model.

Marking has a separate unresolved purpose: a weapon-delivered beacon could provide a continuing observation source, whereas a tactical tag could simply improve follow-up targeting. G018/G019 preserve confidence and observation age; G022 permits selected faction data sharing. Neither establishes a new observation source merely because an operator labels a contact.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Compose equipment-specific penetration with the eventual shield/armour routing rules. For marking payloads, decide whether to model a delivered beacon or an abstract follow-up bonus. Exact penetration strengths remain open. |
| Ongoing activity | Monitor supported beacon reports, link quality and contact confidence; ordinary AI handles routine delivery and reception. |
| Active decisions | Spend a specialised round to establish an observation source, choose recipients and exploit the resulting opportunity. An ordinary tactical label remains distinct from a physical payload. |
| Complexity and AI | Simple presets delegate suitable ammunition selection and reporting; deeper play chooses when improved contact continuity is worth delivery risk and stock use. No per-beacon manual polling chore. |
| Interactions and communication | Sensors interprets reports, Comms supports their transmission/sharing, Weapons delivers and Inventory supplies ammunition. Information remains limited to what the equipment actually reports. |
| Risks | Marking reveals every hidden component, grants permanent perfect tracking, duplicates free tactical tags, or becomes compulsory before every attack. |
| Dependencies | G018/G019/G022/G050/G105, S003/S005 and later Sensors/Comms/shield/armour review. Exact beacon measurements, lifetime, links, removal and countermeasures remain open. |

### Resolved choice: what should a marking payload do?

| Option | Form | Trade-off |
|---|---|---|
| A — Physical beacon | Deliver equipment that reports limited observations while operating and communicating; failure, expiry or countermeasures can interrupt it. | Creates tracking and communication opportunities grounded in an actual information source, with more equipment rules to define. |
| B — Tactical mark | Apply a temporary follow-up targeting benefit, without simulating a beacon or its transmission. | Simpler combat sidegrade, but offers less interaction with Sensors and Comms. |

**Selected: A.** Suitable marking payloads deliver physical beacons that report limited observations while operating and communicating. They do not grant a generic targeting bonus, reveal all hidden components or establish permanent perfect tracking. Ordinary AI can handle routine delivery/reporting. Exact measurements, lifetime, links, removal and countermeasures remain open. Equipment-specific penetration remains a retained candidate under the existing G050/S005 direction; exact shield/armour routing is unresolved.

## S013 — Persistent mines and minelayers

**Inclusion recommendation: retain optional persistent deployable ordnance, and resolve trigger discrimination.** [The unscheduled Plan baseline](../gdd/future/planned-not-scheduled.md#mines) requires arming, trigger, detection, ownership, expiry/recovery, AI and civilian-risk contracts. [R1 #52](research/feature-gap-analysis-master.md) proposes a simple static proximity mine that detonates when any ship enters its radius. This is a research starting point, not a selected trigger rule. Persistent mines remain outside scheduled C5 payload work.

Mines can create preparation, route denial, escort and clearance activities rather than another direct-fire weapon. They use the common equipment/effect rules where applicable, including S011 physical collateral. Selective triggering affects whether detonation occurs; it does not make nearby allies immune to a triggered explosion.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional fitted deployment/clearance capability, not a required weapon on every ship. Persistent physical ordnance has authored arming, detection and lifecycle properties. Decide whether suitable mines can discriminate triggering contacts. |
| Ongoing activity | Monitor known fields, safe routes, stock and supported mine status; automate routine deployment and clearance under bounded orders. No manual polling of every mine. |
| Active decisions | Choose a field or corridor, coordinate placement with Helm, warn others through Comms, and clear or bypass a suspected field. Pattern controls and exact clearance tools remain open. |
| Complexity and AI | Simple presets delegate field execution and supported trigger policy; deeper play assesses geometry, uncertain traffic and detection. Mine sensing uses actual equipment/evidence, not hidden faction truth. |
| Interactions and communication | Navigation supplies routes, Sensors investigates hazards, Comms shares supported warnings and Weapons/Inventory provide deployment and clearance resources. Physical collateral and civilian consequences remain. |
| Risks | Undetectable unavoidable traps, perfect friend recognition despite deception, endless individual placement clicks, persistent litter and a mandatory mine-clearance phase in every scenario. |
| Dependencies | G018/G022/G027/G029/G061/G065, S011 and later Sensors/Comms/deployable lifecycle review. Exact placement, recognition, triggers, expiry/recovery and countermeasures remain open. |

### Resolved choice: can mines discriminate triggering contacts?

| Option | Form | Trade-off |
|---|---|---|
| A — Equipment-dependent selective triggers | Suitable mines can use authored trigger policies and locally available observations to select contacts; simple indiscriminate proximity variants remain possible. | Supports defended corridors and deception/counterplay, but requires recognition limits to be clear. |
| B — Proximity only | Mines trigger on any eligible contact entering their armed proximity zone; safe use relies on placement, warnings and disarming. | Simple and hazardous, but makes mixed traffic and friendly passage difficult. |

**Selected: A.** Suitable mines use authored trigger policies and locally available sensing/identification to discriminate contacts. Simple indiscriminate proximity variants remain possible. No hidden faction truth is supplied for recognition, and physical collateral still applies after detonation. Exact placement, trigger controls, recognition, expiry/recovery and countermeasures remain open.

## S014 — Ammunition fabrication

**Inclusion recommendation: retain the selected G054 direction and resolve resource overlap with repair.** [The C5 baseline](../gdd/mechanics/targeting-weapons.md) plans ammunition fabrication. G054 already selects finite materials, power and production time on suitable ships, plus starbase restocking; G065 selects automatic timed local replenishment from main stores. Do not re-ask whether to include fabrication or add mandatory queue clicks.

The unresolved decision is whether manufacturing rounds can consume some of the same material categories needed for component restoration. G058 selects typed repair parts, not universal interchangeable currency. Shared feedstock can create a meaningful reserve decision without allowing every finished repair part to become any missile.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Equipment-dependent production with authored recipes and finite capacity; ordinary AI maintains desired stock levels/priorities. Decide whether some manufacturing inputs overlap repair resources. |
| Ongoing activity | Monitor reserves and bottlenecks while delegated production/replenishment runs automatically. Avoid repeated approval of routine batches. |
| Active decisions | Change ammunition mix or production priority, preserve material for repairs, request power or arrange resupply when stocks constrain the mission. |
| Complexity and AI | Simple presets delegate stock maintenance within reserve policies; deeper operators assess actual competing needs. Preserve individual System AI and communication, without a new central planner. |
| Interactions and communication | Weapons reports expected expenditure, repair operators report anticipated material needs, Engineering allocates power and Inventory supplies available stock. Shared inputs can make those reports consequential. |
| Risks | Too many resource types, universal magic feedstock, hidden consumption of protected repair reserves, constant batch micromanagement and scarcity that leaves players with no useful alternatives. |
| Dependencies | G054/G058/G065/G067/G111, S004 and later repair/logistics review. Exact recipes, reserve policies, throughput and input categories remain open. |

### Resolved choice: should ammunition fabrication compete with repair for materials?

| Option | Form | Trade-off |
|---|---|---|
| A — Some shared inputs | Authored recipes can share selected material categories with repair supply/production, alongside specialised inputs and finished parts. | Creates meaningful reserve and resupply decisions, but needs clear consumption and reserve policies. |
| B — Separate inputs | Ammunition fabrication uses dedicated stocks that repairs do not consume. | Easier reserves to understand, but less interaction between offensive expenditure and restoration. |

**Selected: A.** Authored ammunition recipes can share selected material categories with repair supply/production, alongside specialised inputs and finished parts. Ordinary AI maintains stocks while respecting configured reserves. This does not make every finished repair part convertible into any ammunition. Exact recipes, input categories, reserve policies and throughput remain open.

## S015 — Fire groups and independent bank targeting

**Inclusion recommendation: retain G048 independent weapon-System targets and resolve group editing scope.** [R6's NEBULOUS comparison](research/comparable-games-and-falling-frontier.md) describes weapon groups engaging different targets. G048 already selects a primary target plus separate assignments/priorities for supported weapon Systems, with simple presets retaining main-target play. S008 selects authored firing modes and barrel cycles; fire groups should not reopen live barrel-pattern construction.

A fire group can provide convenient commands to several weapon Systems while each retains its own AI, knowledge, readiness and authority. It is not a new central AI, a guarantee that every weapon has a solution, or permission to control another Station's Systems. Decide whether players can redefine group membership during a mission or use groups prepared with their loadout.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional grouping of whole weapon Systems for common target/policy or supported coordinated fire. Individual assignments remain available; exact simultaneous release semantics remain open. |
| Ongoing activity | Monitor useful coverage/readiness while ordinary System AI executes delegated engagements. |
| Active decisions | Split attention across threats, coordinate a salvo or conserve a bank. Decide whether regrouping itself belongs in live play. |
| Complexity and AI | Simple presets use the primary target and prepared groups. Greater depth exposes individual assignments and group controls without changing physical weapon limits or creating per-emitter chores. |
| Interactions and communication | Weapons coordinates firing intent with Helm geometry, Sensors evidence and Engineering reserves. Group commands remain subject to the usual System ownership/admission rules. |
| Risks | Group editing consumes crisis attention, group commands hide individual readiness, duplicate/conflicting assignments, or one operator gains authority over another Station. |
| Dependencies | G048/G104/G105/G107/G114, S004/S007/S008. Exact membership constraints, conflict handling, saved configuration and coordinated-release controls remain open. |

### Resolved choice: when can players define fire-group membership?

| Option | Form | Trade-off |
|---|---|---|
| A — Editable during play | Players can regroup their authorised weapon Systems during a mission, with prepared defaults for simple play. | Flexible response to damage or changing threats, but adds live configuration work. |
| B — Prepared groups | Group membership comes from authored or saved loadout configuration; during play players change targets, policies and supported firing intent. | Keeps live decisions focused on employment, but unexpected situations rely on individual assignments rather than new groups. |

**Selected: B, with individual override.** Group membership comes from authored or saved configurations, but an operator can de-group a weapon System during play to give it an individual override. Group commands must not overwrite that active override. This does not introduce general live group construction or live barrel-pattern editing. Each System retains ordinary AI, knowledge, readiness and authority. Exact override cancellation/rejoining, membership constraints and coordinated-release controls remain open.

## S016 — Targeted subsystem attack

**Inclusion recommendation: retain optional capability-focused attacks and resolve their relationship to physical geometry.** [R1's subsystem damage/targeting candidates and R2's Cosmoteer comparison](research/README.md) motivate disabling equipment rather than only destroying hulls. G057 selects component conditions on ships, but does not by itself select how attackers can deliberately cause them. G018/G020 limit contact knowledge; weapon AI needs evidence of the compartment containing equipment. AD48 supersedes exact internal component coordinates.

Two approaches can support a disable-engines intent: aim at a known physical component with exposure/penetration determining reach, or bias damage routing toward the chosen System after a normal ship hit. The choice affects how much Helm geometry and Sensors investigation contribute. Physical targeting need not imply freely built interiors or a full voxel simulation; authored layouts remain selected under G075.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional equipment-supported subsystem attack, with ordinary hull targeting available. Decide whether component reach follows actual geometry or an abstract routing bias. |
| Ongoing activity | Maintain a usable solution and relevant evidence while ordinary weapon AI handles aiming/firing. |
| Active decisions | Seek engine, weapon or sensor suppression; coordinate an exposing manoeuvre or penetrating shot where supported; choose disablement versus destruction. |
| Complexity and AI | Simple presets retain ship targets or delegate a capability intent. Deeper operators select supported targets from available evidence. No mandatory component selection for every shot. |
| Interactions and communication | Sensors identifies relevant equipment; Helm presents a useful angle; Weapons chooses delivery; observed effects inform follow-up. Enemy component conditions remain subject to knowledge limits. |
| Risks | Perfect internal diagrams without scanning, every shot guaranteeing a chosen failure, targeting that ignores shielding/geometry, or a component-clicking chore. |
| Dependencies | G018/G020/G049/G057/G075/G105, S005/S012 and later shields/armour review. Exact component representation, precision, damage routing and evidence requirements remain open. |

### Resolved choice: how should deliberate subsystem attacks reach their target?

| Option | Form | Trade-off |
|---|---|---|
| A — Physical component targeting | Aim at known component locations; exposure, intervening protection and weapon penetration determine whether damage reaches them. | Makes investigation and approach angle useful, but requires legible authored geometry. |
| B — Damage-routing bias | Select a System to favour when a normal ship hit routes damage, without needing a physical path to a particular component. | Easier to read and implement as a combat abstraction, but offers less manoeuvre-driven interaction. |

**Selected: A, with a shared repair/damage map and progressive penetration, refined by AD39–50 below.** Model damage using the authored ship map intended for repairs. A hit follows the incoming direction through intervening compartments; targeting engines from the front encounters the compartments before them. Damage reduces compartment HP and applies component status effects. Attackers scan for the compartment containing equipment and receive firing-angle advice from available evidence, without guaranteed deck selection. Components have no precise internal positions; AD48 selects effects from the compartment-wide eligible pool, weighted by damage type and susceptibility. Progressive breakthrough uses compartment state; AD46 makes resistance fall progressively as compartment HP falls. Ordinary weapon AI handles aiming under the same knowledge limits. This does not select omniscient internal diagrams or automatic Helm manoeuvres. Exact map geometry, payload deposition, component-effect selection, repair interaction and angle-advice precision remain open.

**Architecture refinements — AD39–40, extended by AD45:** each deck has a **2D** compartment layout. Incoming damage selects a weighted random deck, independently of shot Z height, then penetrates along its incoming planar direction. Ordinary hits use distributed lines across the facing profile under supported accuracy/spread rules; deliberate targeting concentrates planar aim toward the compartment known to contain equipment, without bypassing deck selection. See the [architecture report](./architecture/index.html#damage-decks); exact deck weights, sampling and accuracy integration remain under review.

**Architecture refinements — AD41–50:** penetration strength is configured separately from delivered damage/effects. Material properties are set **per compartment only**, and resistance is charged per unit of 2D path length, progressively weakening as compartment HP falls. Compartments have HP; individual damage events apply component status effects through authored rules using deposited damage amount/type and component susceptibility. Chance and severity vary, and a small hit need not cause a fault. This replaces finer structural HP or local breach-grid interpretations while preserving component conditions and repair targets. Component selection is compartment-wide without precise internal positions. At zero compartment HP, AD49 disables the contents through a condition removed by compartment restoration, without clearing other component faults. AD50 clears the condition at a modest authored HP fraction above zero and below full restoration, with reapplication only at zero HP. Exact deposition, effect weights, the resistance curve and threshold tuning remain under review in the [architecture proposal](./architecture/index.html#compartment-hp).

## S017 — Fighter / attack-drone launch and recovery

**Inclusion recommendation: retain optional carried combat craft, and resolve direct human piloting scope.** [R6's Artemis, Deadlock, Homeworld and Carrier Command comparisons](research/comparable-games-and-falling-frontier.md) offer fighter participation, squadron roles and supported remote operations. Their control models differ; none establishes a mandatory fighter Station for Phoenix. G027 already selects full-3D small-craft movement, and G061 selects recovery commitments for deployed crew/equipment.

Craft can contribute reconnaissance, interception, attack and escort, with launch readiness, endurance and recovery creating support decisions. Each craft's Systems retain ordinary individual AI, communication and limited knowledge. A sortie order is bounded mission intent, not a new omniscient wing planner. Decide whether this capability should also include direct human flight control.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional fitted carried-craft capability and bounded sortie orders. Exact craft families, group orders and support facilities remain open. |
| Ongoing activity | Monitor readiness, supported reports, endurance and recovery capacity; ordinary AI handles flight and routine execution. |
| Active decisions | Commit craft to scouting/protection/attack, change supported orders, recall or accept losses and coordinate safe recovery. |
| Complexity and AI | Simple presets delegate sortie operation; deeper operators manage missions/risk without per-craft steering. Decide whether optional direct piloting belongs in scope as well. |
| Interactions and communication | Sensors interprets craft observations, Comms carries orders/reports, Helm supports launch/recovery geometry and Engineering/Inventory sustain facilities and craft. |
| Risks | A pilot waits for deployment, a commander monopolises decisions, dozens of vehicle controls overwhelm the bridge, craft report hidden truth or recovery is a repetitive docking chore. |
| Dependencies | G003/G008/G012/G027/G061/G068/G102, S008 and later hangar/shuttle/logistics review. Exact endurance, recovery, loss, command links and control surfaces remain open. |

### Resolved choice: should carried combat craft support direct human piloting?

| Option | Form | Trade-off |
|---|---|---|
| A — Sortie management | Players give bounded missions and priorities; ordinary AI flies combat craft. | Keeps bridge activity focused on coordination, without requiring a separate flight-control experience. |
| B — Optional direct piloting | Ordinary AI remains the default, but a human can take a supported craft's controls. | Adds another participation style, but requires useful duties outside deployment and suitable flight controls. |

**Selected: A — Sortie management.** Players assign bounded missions, priorities and recall orders; ordinary individual craft System AI handles flight and execution with normal communication and limited knowledge. Preparation, reconnaissance, protection, attack and recovery supply bridge decisions. Direct human piloting is outside this redesign's initial scope; full-3D small-craft movement remains selected. Exact craft families, endurance, links, launch/recovery capacity and losses remain open.

## S018 — Electronic attack / Jam

**Inclusion recommendation: retain jamming and resolve whether effects follow signals or a selected-ship debuff.** [The C5 baseline](../gdd/mechanics/sensors-epistemics.md) plans Sensors/Comms degradation with increased jammer signature. [R6's HighFleet and NEBULOUS comparisons](research/comparable-games-and-falling-frontier.md) motivate exposure, guidance and geometry interactions. G020 separates broad and focused sensing; G024 retains real shared communication channels; G105 selects equipment-dependent guidance. No exact propagation or interference model is selected yet.

Jamming can protect an approach or disrupt an engagement while also revealing the emitter and interfering with useful information. Decide whether operating geometry and affected signals determine interference or selecting an enemy applies an abstract penalty to that ship. This need not require a detailed radio-frequency simulation or manual frequency arithmetic.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Equipment-specific interference with finite power/thermal/emission costs. Resolve the effect model before selecting jammer families or exact channel controls. |
| Ongoing activity | Sustain useful interference, monitor supported effects and manage exposure; ordinary System AI executes delegated operation. |
| Active decisions | Choose protection versus information access, disrupt a supported sensor/communication/guidance function, reposition or stop transmitting. |
| Complexity and AI | Simple presets delegate modes/priorities; deeper operators choose supported coverage and signal classes. Personal depth does not change physical interference rules. |
| Interactions and communication | Sensors/Comms report degraded observations or links; Helm manages geometry; Engineering supplies flow; weapon operators adapt to equipment-specific guidance limits. |
| Risks | A universal off switch, magical enemy-only interference, unreadable loss of information, continuous channel-tuning chores and AI knowing hidden receiver state. |
| Dependencies | G018/G020/G021/G024/G105, S003/S012/S017 and S019/S020. Exact coverage, strength, signal classes, countermeasures and reporting remain open. |

### Resolved choice: how should jamming determine what it affects?

| Option | Form | Trade-off |
|---|---|---|
| A — Selected-ship penalty | Select a ship and apply equipment-defined Sensors/Comms/guidance penalties while engagement conditions hold. | Straightforward control and feedback, but less interaction with nearby links and friendly operations. |
| B — Signal and geometry effects | Equipment coverage, operating conditions and signal/receiver properties determine interference, potentially affecting friendly functions too. | Creates positioning and communication trade-offs, but needs clear feedback and bounded rules. |

**Selected: B, with tracking equipment and explicit interaction feedback.** Jamming follows actual signal/geometry and equipment properties, potentially affecting friendly functions. A targeted jammer can track its selected contact but still interfere with other craft in its coverage. Jammers, tractor beams and other aimed equipment can reuse suitable weapon-model arc/range/turret/traversal behaviours while retaining their distinct effects/resources.

Complex interactions must be surfaced to players. Comms can show a contact unavailable because the ship is jamming it, with that supported cause stated. If an affected ship can still communicate, it should report interference to the attacker through ordinary channels, including friendly-fire cases. Reports use available evidence, express uncertain attribution where necessary and cannot bypass blocked links. This is normal individual System AI communication, not a unique jamming AI or privileged state feed. Exact coverage, strength, attribution, reports and controls remain open.

## S019 — Spoofing and decoy emitters

**Inclusion recommendation: retain evidence-testable spoofing and resolve expendable decoy equipment.** [The C5 baseline](../gdd/mechanics/sensors-epistemics.md) plans uncertain spoof contacts that observation can disprove. The catalogue also collects Research decoy emitters; these are not a claimed delivered C5 capability. G018/G106 retain evidence-dependent classification and ambiguous presentation. S018 establishes real coverage and player-visible interaction feedback.

An onboard emitter can mislead supported observations while operating. A deployed decoy would additionally be physical equipment with position, stock, endurance and potential recovery/destruction. Its emitted apparent contact need not represent its actual physical size or capabilities. Decide whether to include both sources, without granting a phantom ship real weapons or physics merely because a sensor report resembles one.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Equipment-specific spoofing; decide whether expendable physical decoys join onboard emitters. Exact deceptive signatures and deployment variants remain open. |
| Ongoing activity | Sustain supported deception and monitor available reports/stock; ordinary AI operates delegated equipment. |
| Active decisions | Create a diversion, conceal a useful movement, spend a decoy or investigate contradictory evidence. Targeting/guidance reacts only to the evidence its equipment uses. |
| Complexity and AI | Simple presets delegate approved deception; deeper operators select timing and supported profiles. No live waveform programming or mandatory validation of every routine contact. |
| Interactions and communication | Sensors compares observations, Helm exploits geometry, Comms shares supported findings, and Inventory supplies physical decoys if selected. Useful feedback must not reveal hidden success or enemy beliefs. |
| Risks | Perfect false contacts, guaranteed missile diversion regardless of seeker, omniscient success reports, fabricated physical ships and another repeated launch chore. |
| Dependencies | G018/G019/G021/G024/G065/G105/G106, S003/S018/S020 and later Sensors/deployables. Exact detectable evidence, countermeasures, lifetime and recovery remain open. |

### Resolved choice: should deception include expendable physical decoys?

| Option | Form | Trade-off |
|---|---|---|
| A — Onboard emitters only | Spoofing comes from installed equipment operating on the ship. | Fewer supply/lifecycle rules, but deception remains tied to the ship's position and equipment. |
| B — Emitters and deployed decoys | Retain onboard spoofing and add suitable physical decoy equipment with finite stock and operating life. | Adds placement, timing and seeker-specific diversion opportunities, with additional supply and lifecycle constraints. |

**Selected: B.** Retain onboard spoofing and add suitable deployed physical decoys with finite stock, operating life and actual position. Their apparent signatures can differ from their actual equipment/size. Effectiveness follows the observer or seeker's available evidence and behaviour, not guaranteed diversion or a false report becoming a real ship. Ordinary individual System AI operates delegated equipment. Exact profiles, deployment, lifetime, recovery and countermeasures remain open.

**User refinement — common ship/entity composition:** deployed equipment uses the same ship model as other entities, with identical rules for player and AI ships apart from the controllers of some Systems. A turret is an engineless ship with a weapon, battery and simple ordinary AI; a decoy is a short-lived engineless ship with a battery and sensor-spoofing System. Compose other deployables from the same Systems rather than creating bespoke simulation rules. This retains existing networking and individual System AI with communication and limited knowledge. The decoy entity is physical; an apparent larger ship in its spoofed reports is not thereby physical.

## S020 — Electronic hardening / ECCM

**Inclusion recommendation: retain equipment-based resistance to electronic interference and resolve protection allocation.** [The C5 baseline](../gdd/mechanics/sensors-epistemics.md) plans Harden for Sensors and Comms with authored power, heat or capability costs. S018 selects signal/geometry interference and S019 equipment-dependent deception. Hardening must improve supported receiver operation rather than automatically reveal hidden truth, restore a physically absent link or cancel every deceptive observation.

Protection can be a ship-wide operating mode, or limited capacity allocated among supported functions. The latter creates a decision about which observations or links matter most now. This is an operating choice within the fitted equipment, not a change to personal complexity presets or control ownership.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Authored electronic protection with real costs and bounded effectiveness. Decide whether protection covers all eligible ship functions together or is allocated by priorities. |
| Ongoing activity | Maintain supported protection and monitor degraded operation; ordinary AI executes priorities without repeated debuff-cancellation clicks. |
| Active decisions | Preserve a crucial communication link, broad awareness or focused observation as interference changes. Guidance protection may exist where fitted; it is not claimed as delivered C5 scope. |
| Complexity and AI | Simple presets delegate allocation using priorities and context; deeper operators choose supported protection priorities. Same equipment capacity and knowledge limits apply at every depth. |
| Interactions and communication | Sensors/Comms explain useful capabilities and degraded links; Engineering supplies power and handles thermal capacity; operators communicate what protection their current task needs. |
| Risks | Universal immunity, perfect recognition of spoofing, redundant power sliders, invisible protection switching and an opaque cancel/reapply contest. |
| Dependencies | G018/G020/G024/G105/G114, S018/S019 and later Sensors/Comms/resource review. Exact capacity, costs, protected functions and strength remain open. |

### Resolved choice: how should electronic protection be allocated?

| Option | Form | Trade-off |
|---|---|---|
| A — Ship-wide mode | Hardening applies across eligible ship functions together, with an overall operating cost. | Simple to operate, but fewer choices between competing information needs. |
| B — Prioritised protection | Finite equipment capacity supports selected functions/links according to priorities, with ordinary AI handling routine allocation. | Creates meaningful coordination and rationing, but requires clear protected/degraded status. |

**Selected: B, at System level or simplified Station level.** Finite protection supports selected Systems according to priorities; do not expose component-by-component allocation. Simplified presets can offer Station-level selection mapped to the supported Systems, with the same physical rules. Ordinary individual System AI executes routine allocation and operation. Protected/degraded status is clear under S018. Exact capacity, costs, supported Systems and allocation policy remain open.

## S021 — Hacking / remote access / mind-control devices

**Inclusion recommendation: consider bounded remote access/disruption, and explicitly decide whether hostile control takeover belongs in scope.** [R2's FTL comparison](research/functional-ship-customisation.md) includes hacking and mind control as equipment candidates. [Expanded EWar in the unscheduled Plan](../gdd/future/planned-not-scheduled.md#expanded-ewar) allows later authored remote disruption/access operations; it does not select FTL's exact mechanics or arbitrary enemy control.

Remote access could retrieve supported information, open an authored access path or interfere with a particular capability. Hostile takeover would go further: temporarily make an enemy System act against its operator's intent. These are in-fiction simulation effects, not changes to transport security or permission to issue network commands as another participant. G103 already excludes a separate code/manual puzzle layer.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional equipment- and target-dependent operations with authored prerequisites/effects. Decide whether hostile System control is included alongside bounded access/disruption. |
| Ongoing activity | Maintain supported access and observe its status; ordinary AI executes the delegated operation with limited knowledge. |
| Active decisions | Choose a useful access objective, commit equipment/resources, interrupt or withdraw and respond to detected intrusion. |
| Complexity and AI | Simple presets delegate bounded procedures; deeper operators choose objectives and risk. No live programming puzzle or omniscient internal access. |
| Interactions and communication | Sensors identifies supported opportunities, Comms or fitted links carries access where applicable, Engineering supports operation and defenders respond through relevant Systems. |
| Risks | Removing a human player's useful agency, automatic whole-ship capture, opaque immunity/debuff loops, bypassing real admission rules or imposing a new puzzle Station. |
| Dependencies | G003/G018/G057/G079/G081/G103, S018/S020 and later security/access review. Exact delivery, prerequisites, countermeasures, duration and control boundaries remain open. |

### Resolved choice: should remote attacks include hostile System takeover?

| Option | Form | Trade-off |
|---|---|---|
| A — Bounded access and disruption | Support authored information/access objectives and interference, without letting the attacker command an enemy System. | Preserves operator agency and clearer control ownership, while retaining remote operations. |
| B — Limited hostile takeover too | Suitable equipment can temporarily make a specific enemy System perform bounded hostile actions, with detection and counterplay. | Adds dramatic sabotage options, but needs careful limits on losing control and affecting fellow players. |

**Selected: A.** Retain bounded authored information/access objectives and capability disruption, without allowing attackers to command enemy Systems. Ordinary individual System AI executes delegated operations with normal communication and limited knowledge. These are in-fiction effects, not changes to networking or command admission. Exact prerequisites, delivery, effects, duration and countermeasures remain open; G103 still excludes a separate code/manual puzzle layer.

## S022 — Shield generators and directional arcs

**Inclusion recommendation: retain authored directional protection and resolve its relationship to local batteries.** [The Current shield baseline](../gdd/mechanics/shields.md) has authored arcs with geometry, strength, regeneration and collapse/recovery. It does not require four conventional facings. Helm changes exposure; Shields operates focus; power supports regeneration. Selected power rules replace abstract power availability with flow into local System batteries, but do not yet specify whether shield strength itself is the battery reserve.

Retain geometry-driven interception and actual penetration/overflow toward the ship's damage map under S016. S023 reviews redistribution/focus next; this entry first resolves the energy/protection distinction without assuming a separate generator for every facing.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Hull-authored directional arcs, with local-battery-supported operation. Decide whether absorbed damage reduces distinct arc strength or draws directly from generator batteries. |
| Ongoing activity | Monitor exposed arcs and recovery while ordinary System AI operates delegated protection. |
| Active decisions | Prepare reserves, request flow, change protection policy or coordinate a turn/withdrawal as a facing weakens. |
| Complexity and AI | Simple presets delegate arc operation and display useful exposure; deeper operators assess recovery and resource trade-offs. Shared physical rules apply at every depth. |
| Interactions and communication | Shields reports weak protection to Helm and expected recovery demand to Engineering; Sensors/Weapons help interpret observed threats through ordinary communication. |
| Risks | Redundant energy meters, instantaneous full recovery, damage draining unrelated Systems through an unexplained rule, or one operator solving all defence without movement/coordination. |
| Dependencies | Selected power/thermal rules, G049/G050/G057, S005/S016 and S023–S025. Exact absorption, regeneration costs, generator-to-arc topology and collapse rules remain open. |

### Resolved choice: is shield strength separate from the generator's local battery?

| Option | Form | Trade-off |
|---|---|---|
| A — Separate protection strength | Hits deplete arc strength; generator operation/regeneration consumes local battery energy. | Separates having energy available from rebuilding protection, allowing recovery and geometry decisions. |
| B — Battery-backed absorption directly | Absorbed hits consume the relevant generator battery reserve; available energy directly determines protection endurance. | Fewer distinct reserves, but ties incoming damage directly to the power buffer. |

**Selected: A, with raised-shield upkeep and recharge-dominated consumption.** Hits deplete separate arc strength. Local batteries supply continuous operating draw while shields are raised, with most shield-related consumption coming from recharging protection. This upkeep is distinct from the general fullness-dependent passive battery loss already selected. Having energy ready does not itself restore depleted protection instantly. Exact upkeep/recharge rates, generator-to-arc topology and collapse rules remain open.

## S023 — Shield focus / redistribution

**Inclusion recommendation: retain directional allocation decisions and resolve movement of existing protection.** [Current focus](../gdd/mechanics/shields.md#focus) applies authored protection/trade-off multipliers to a selected arc. S022 now distinguishes existing arc strength from energy in local batteries. Decide whether a redesigned operator can transfer established protection between arcs, or only change where recharge effort goes. Neither option needs rapid repetitive focus clicks.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Directional priorities executed by ordinary Shields System AI; resolve whether existing arc strength can move between facings. Exact rates and controls remain open. |
| Ongoing activity | Maintain a useful protection distribution while listening to Helm manoeuvres and expected threat directions. |
| Active decisions | Prepare a facing for a pass, protect a stationary operation or sacrifice coverage elsewhere to recover a threatened side. |
| Complexity and AI | Simple presets delegate allocation from intent/context; deeper operators select priorities and, if supported, redistribution. No individual component allocation is implied. |
| Interactions and communication | Helm changes exposure and communicates planned turns; Engineering supplies recharge flow; Shields reports what coverage can be sustained and which sides are vulnerable. |
| Risks | Instant full shields facing every shot, arbitrary free strength creation, repetitive flicking, focus masking depleted protection or redistribution making Helm positioning irrelevant. |
| Dependencies | S022, selected power/thermal rules, G049/G050 and S024/S025. Exact recharge capacity, transfer efficiency, rate and energy/heat cost remain open. |

### Resolved choice: can shield strength be transferred between arcs?

| Option | Form | Trade-off |
|---|---|---|
| A — Recharge priorities only | Prioritise which arcs rebuild; existing protection stays in its current arc. | Clear geometry and recovery constraints, but fewer emergency redistribution options. |
| B — Redistribution too | Transfer existing protection from donor arcs to recipient arcs at a finite rate, alongside recharge priorities. | Adds preparation and emergency trade-offs, but needs limits so every incoming shot cannot be met instantly. |

**Selected: B, through battery discharge and recharge.** Discharge a donor shield arc into a battery, then use that reserve to recharge another arc faster than relying only on incoming reactor flow. The donor loses protection; the recipient still follows its recharge process rather than an instantaneous direct strength transfer. Ordinary Shields System AI can execute delegated priorities. Existing battery/thermal rules apply. Exact conversion efficiency, discharge/recharge rates, capacity handling and battery-to-arc topology remain open; this is a proposal change from Current focus multipliers.

## S024 — Shield modulation / frequency retuning

**Inclusion recommendation: retain G050's optional probing and resolve defensive retuning availability.** [The baseline](../gdd/mechanics/shields.md) retains frequency interaction, with deeper shield controls in Plan C4. G050/S005 already select optional scanner-dependent weakness investigation, modest effect against most foes, stronger special equipment interactions and findings that persist unless configuration changes. Special equipment may rotate shield/phaser frequencies. Do not reintroduce a universal frequency-guessing ritual or assume a switching penalty already selected.

The remaining inclusion choice is whether changing defensive modulation is a capability of suitable fitted equipment, or a standard control on every shield generator. Automated rotation and manual choices can be authored for equipment that supports them; neither requires continuous human retuning.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain persistent observed weaknesses and equipment-dependent interactions. Decide whether defensive retuning requires suitable equipment or exists on every generator. |
| Ongoing activity | Monitor supported evidence about shield behaviour; ordinary AI handles delegated modulation where fitted. |
| Active decisions | Respond to an observed exploitable configuration, choose a supported defensive profile or communicate a changed vulnerability. Exact profiles and effect strengths remain open. |
| Complexity and AI | Basic play delegates operation without repeated matching clicks. Deeper play uses evidence and supported equipment choices; no personal-depth changes to physical effectiveness. |
| Interactions and communication | Sensors investigates, Weapons exploits supported findings and Shields responds using available evidence. Engineering supplies actual operating/recovery resources. |
| Risks | Everyone must retune continually, arbitrary frequency guessing, automatic immunity to an earned scan, or hidden perfect knowledge of the attacker's settings. |
| Dependencies | G018/G050/G114, S005/S022/S023 and later focused scanners. Exact retuning costs, timing, detected changes and counter-probing remain open. |

### Resolved choice: which shield generators support defensive retuning?

| Option | Form | Trade-off |
|---|---|---|
| A — Suitable equipment only | Retuning/rotation is a fitted equipment capability; ordinary generators keep a persistent configuration. | Preserves special equipment variety and limits routine tuning work. |
| B — Every generator | All shield generators offer defensive retuning, with exact trade-offs to be designed. | Gives every Shields operator this option, but risks making it a universal recurring task. |

**Selected: B, with equipment-dependent performance.** Every shield generator supports defensive retuning as a consistent capability; some equipment performs better than others. Ordinary System AI operates delegated retuning using available evidence. The earlier equipment-only recommendation is not selected. Optional attacker probing and generally persistent findings until configuration changes remain. Exact transitions, performance differences, costs and change detection remain open; universal availability does not imply mandatory recurring manual matching.

### Resolved dependency: does retuning require lowering the affected shield?

Universal retuning needs a clear transition rule so changing configuration does not automatically invalidate an earned scan at no operational consequence. Determine the broad form before numerical costs. Retuning applies to the affected generator/arcs, not automatically every shield on the ship; exact topology remains open under S022.

| Option | Form | Trade-off |
|---|---|---|
| A — Retune while raised | Change configuration over time while retaining protection, with a temporary protection reduction and battery/thermal cost. | Allows an active defensive response while exposing a readable transition window. |
| B — Lower and rebuild | Lower the affected protection to retune, then rebuild it through normal battery-powered recharge. | Uses a clear commitment and recovery cost, but can make a response impractical under sustained fire. |

**Selected: A.** Retune while raised through a timed transition that temporarily reduces protection and consumes battery energy/generates heat. Equipment can differ in performance without changing the common capability. Exact duration, reduction, costs, interruption rules and change detection remain open. Ordinary AI uses available evidence; no automatic perfect response to hidden attacker settings is selected.

## S025 — Armour, hull sections and structural protection

**Inclusion recommendation: retain physical protective material in the authored repair/damage map and resolve field restoration.** [The baseline damage route](../gdd/mechanics/damage-repair.md) distributes shield leakage to damage-bearing Systems; it does not implement the proposed physical penetration map. [R2's Cosmoteer comparison](research/functional-ship-customisation.md) motivates layout-dependent protection. S016 already selects material along hit paths and repeated-fire breaching; G075 retains authored layouts rather than player-built interiors.

Armour and structure should therefore influence what a shot reaches and how concentrating fire changes a local path. Do not re-ask whether armour geometry matters. The unresolved operational question is whether field work patches a breach with weaker protection or can restore the original armour where the required resources and access exist.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Authored protective material/sections integrated with hit paths and team repair. Decide the scope of field armour restoration; exact materials and penetration response remain open. |
| Ongoing activity | Monitor supported damage/exposure and preserve useful aspects; passive hardware does not require a separate armour-tending chore. |
| Active decisions | Turn a breached side away, exploit an enemy breach, allocate repair teams/materials or seek supported servicing. |
| Complexity and AI | Ordinary repair AI executes approved work and reports achievable restoration; simple presets summarise weak areas while deeper tools show the relevant map. No privileged enemy layout or mandatory patch clicks. |
| Interactions and communication | Helm controls exposure, Sensors investigates enemy protection, Weapons chooses attack paths, and repair/Inventory operators assess actual restoration capacity. |
| Risks | Repairs erase focused-fire consequences instantly, armour becomes a global health bar, invisible holes dominate combat or separate arbitrary patch rules duplicate component restoration. |
| Dependencies | G057/G058/G064/G065/G072/G075/G090, S012/S016/S022. Exact protective materials, access, replacement size, work time and catastrophic limits remain open. |

### Resolved choice: how much armour can field repairs restore?

| Option | Form | Trade-off |
|---|---|---|
| A — Limited field patches | Field work closes/patches damaged sections with reduced protection; restoring original armour needs suitable facilities. | Preserves lasting exposure and servicing decisions, but adds a distinction between patching and full replacement. |
| B — Resource-dependent full restoration | Field work can restore original armour where suitable parts/materials, teams, access and time permit; large/catastrophic cases can still require facilities. | Fits the shared component-restoration model, but requires meaningful costs and work limits to preserve combat consequences. |

**Selected: B.** Field work can restore original armour/structural protection where appropriate parts/materials, teams, access and time permit. Large or catastrophic damage can still exceed field capability and require facilities under G064. Use the shared repair model rather than a universal weaker-patch ceiling; restoration consumes real resources and time. Exact recipes, access, work rates and field limits remain open.

## S026 — Cloaking / concealment device

**Inclusion recommendation: retain optional fitted concealment under normal contact rules, and resolve high-emission actions.** [The unscheduled Plan baseline](../gdd/future/planned-not-scheduled.md#cloaking) describes suppression rather than invisibility, power/heat costs, high-emission restrictions and observation counterplay. [R1 #15/#51](research/feature-gap-analysis-master.md) contains earlier fire-to-decloak and timed-disappearance ideas, not selected Phoenix mechanics. G018/G019 retain uncertain contact evidence and predicted lost tracks; engaging a cloak must not erase another actor's acquired knowledge.

G021 already selects operating signatures from major Systems. A concealment device can alter those signatures while useful operations compete with hiding. Decide whether a high-emission action merely exposes the ship through the ordinary detection model or also forces the cloak to disengage.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional authored cloak equipment with power/thermal constraints, interacting with actual sensing, terrain and observed tracks. No guaranteed disappearance or universal fixed-duration hiding. |
| Ongoing activity | Maintain useful concealment while listening to the capabilities other operators need; ordinary AI runs the selected intent. |
| Active decisions | Risk transmission, active scanning or firing; trade approach speed/protection/work against exposure; abandon concealment or investigate a suspected hidden contact. |
| Complexity and AI | Simple presets delegate operation; deeper players choose supported trade-offs. No restriction on real player speech, rote silence ritual or perfect enemy-detection forecast. |
| Interactions and communication | Engineering supplies flow/cooling; Sensors and Comms weigh active emissions; Weapons considers revealing attacks; Helm manages position and terrain. Feedback follows S018 without hidden observer knowledge. |
| Risks | Guaranteed immunity, erasing earned tracks, enforced idle crew, every System disabled by default, or an uncommunicated action exposing the whole bridge. |
| Dependencies | G018/G019/G021/G029/G105/G113, S018/S019 and S027. Exact signature effects, operating restrictions, transitions and counterplay remain open. |

### Resolved choice: must high-emission actions disengage the cloak?

| Option | Form | Trade-off |
|---|---|---|
| A — Exposure through emissions | Actions increase observable signatures according to equipment and sensing rules; the cloak can remain operating unless physically incompatible with the action. | Supports graded exposure and equipment variation without a universal decloak trigger. |
| B — Forced disengagement | Defined actions such as firing force the cloak off, followed by an authored recovery/re-engagement process. | Clear attack commitment, but less flexibility for nuanced emission-based concealment. |

**Selected: A.** Emitting actions increase observable signatures according to actual equipment and sensing rules; a cloak can remain operating rather than universally disengaging. Specific equipment incompatibilities can still restrict simultaneous operation. Cloaking preserves observers' previously acquired information and predicted tracks under G018/G019. Exact signatures, restrictions, costs and transitions remain open; no particular incompatibility is selected here.

## S027 — Signature and emission controls

**Inclusion recommendation: retain readable emission profiles and resolve their effect on human inputs.** G021 selects automatic signatures from a small readable set of operating causes. [R6's HighFleet comparison](research/comparable-games-and-falling-frontier.md) connects sensing/transmission to exposure; Falling Frontier's advertised sensor/logistics features are inspiration, not playtest evidence. S026 selects emission-based concealment; S018 requires complex interactions to be surfaced through normal tools and communication.

A quiet-operation profile can guide ordinary System AI and explain which human actions increase exposure. It need not remotely switch every human-controlled System or add another central planner. Decide whether profiles only advise humans or also prevent accidental emitting actions unless the local operator explicitly overrides the profile.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Prepared readable profiles or priorities affecting supported System operation, with actual physical signatures. Decide whether human inputs receive advisory feedback or an overrideable inhibition. |
| Ongoing activity | Listen to other operators' capability needs and maintain useful concealment rather than blindly keeping every System off. Ordinary AI follows selected intent and normal context. |
| Active decisions | Accept a revealing scan/transmission/shot, change a local operating choice or coordinate a profile change when the mission needs it. |
| Complexity and AI | Simple presets use prepared profiles; deeper controls expose supported System choices. No per-component emission checklist or personal-depth changes to detectability. |
| Interactions and communication | Sensors explains available observations, Comms explains link needs, Weapons/Helm report imminent activity and Engineering supports the resulting resource changes. No exact enemy-awareness forecast is granted. |
| Risks | A global button automates away coordination, humans cannot respond to crises, accidental exposure is unexplained, or each action requires a repetitive confirmation. |
| Dependencies | G011/G021/G024/G025/G114, S018/S026 and later Sensors/Comms/propulsion review. Exact profile ownership, controls, hysteresis and feedback remain open. |

### Resolved choice: should an emission profile inhibit human actions?

| Option | Form | Trade-off |
|---|---|---|
| A — Advisory for humans | Ordinary AI follows the profile; human operators receive clear exposure warnings but their controls remain immediately usable. | Preserves fast local decisions, but makes accidental exposure easier. |
| B — Locally overrideable inhibition | The profile inhibits specified emitting actions; the owning human operator can explicitly override it when needed. | Prevents accidental exposure, but must avoid repeated confirmation chores or remote removal of local authority. |

**Selected: advisory for humans, with GUI marking and no use limits.** When a low-emission profile is selected, high-emission actions are marked on the relevant human GUI. Controls remain usable without inhibition, override requirements or repeated confirmation. Ordinary individual System AI follows selected intent and normal context; actual emissions retain physical consequences. The earlier overrideable-inhibition recommendation is not selected. Exact profile controls, ownership and presentation remain open.

## S028 — Damage isolation / breakers / bypasses

**Inclusion recommendation: retain causal containment and degraded alternatives, and resolve remote switching versus physical reconfiguration.** Plan C4 supplies network/diagnosis foundations; G059 selects condition-specific escalation/spread and supported containment. S016 uses the repair map for physical damage; G058 selects teams and typed parts; G070 lets teams find their way to assigned work. Those selections do not yet make every damaged connection remotely reroutable.

Isolation can stop a supported failure path while depriving downstream equipment of service. A bypass can restore some capability only if an alternative path or a feasible physical modification exists. Keep the decision legible without requiring manual electrical-circuit design or routing every team step.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Equipment-dependent isolation and bypass, with actual dependencies and degraded capacity. Decide which work can be commanded remotely and which needs a team/materials. |
| Ongoing activity | Monitor contained faults and reduced services; ordinary AI operates delegated controls and work orders. Stable harmless conditions need no automatic intervention. |
| Active decisions | Isolate a spreading fault, preserve a critical service, choose an existing alternate path or commit a team to a feasible bypass. |
| Complexity and AI | Simple presets delegate containment within priorities; deeper tools expose affected Systems and supported alternatives. No component-level switching chore is required at minimum depth. |
| Interactions and communication | Engineering/repair report downstream consequences; affected operators communicate needed capability; Inventory supplies material where work requires it. Authority stays with the owning System. |
| Risks | A magic reroute button repairs destruction, every fault starts a manual circuit puzzle, containment silently disables teammates or AI uses undiagnosed hidden faults. |
| Dependencies | G057–G060/G065/G070/G114, S016 and S029/S031–S039. Exact topology, controls, team access, costs and bypass capacity remain open. |

### Resolved choice: how are isolation and bypass work performed?

| Option | Form | Trade-off |
|---|---|---|
| A — Console-operated supported changes | Supported isolation and bypass configurations are operated from the console, with authored transition time/capacity limits. | Keeps routing work focused on decisions rather than team deployment, but abstracts physical modifications. |
| B — Fitted switches plus physical work | Remotely operate fitted breakers/alternate routes; creating or modifying a bypass requires feasible team work and appropriate materials. | Connects containment to the shared repair/logistics model, with more access and resource constraints. |

**Selected: B.** Remotely operate fitted breakers and alternate routes; creating or modifying a bypass needs feasible team work, access and appropriate materials. Console controls do not create missing connections or repair destroyed equipment. Ordinary individual System AI can execute delegated switching or work orders using available evidence, and teams find their own paths. Exact topology, controls, recipes, work time and bypass capacity remain open; no manual circuit-building minigame is implied.

## S029 — Fire suppression, compartment seals and venting

**Inclusion recommendation: retain bridge-managed incident containment and resolve compartment scope.** [R2's FTL and Cosmoteer comparisons](research/functional-ship-customisation.md) motivate doors, crew access and layout-dependent incidents. [R6's Barotrauma/Stormworks comparisons](research/comparable-games-and-falling-frontier.md) offer containment and rescue constraints without requiring first-person play. G059 already selects condition-dependent spread; S028 selects fitted isolation plus team work; S016/G070 supply the ship map.

Suppression, sealing and venting can offer distinct answers to a supported incident, with consequences for access, equipment and crew safety. Decide whether incidents inhabit coarse physical compartments or remain attached to affected Systems without a separate compartment state. Detailed atmosphere/pressure scope remains for S102; this choice does not select individual oxygen simulation.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Authored containment capability and team intervention. Decide whether compartment layout governs incident extent and isolation. Not every condition spreads or requires venting. |
| Ongoing activity | Monitor known incidents and contained areas while delegated suppression/work proceeds. Stable conditions need no repetitive upkeep clicks. |
| Active decisions | Suppress, seal a boundary, redirect teams or vent where equipment supports it; weigh lost access and safety against containing damage. |
| Complexity and AI | Simple presets delegate response within priorities; deeper tools show relevant affected areas and available methods. Ordinary AI uses discovered conditions and known team locations, not hidden faults. |
| Interactions and communication | Repair/Engineering coordinate access and services; team operators report endangered deployments; Helm/other Systems adapt to lost capability. Consequences should be surfaced under S018. |
| Risks | Every damage event becomes a spreading fire, sealed doors silently trap teams, mandatory first-person firefighting, or a full atmosphere simulation overwhelms the intended bridge work. |
| Dependencies | G057–G060/G068/G070/G071/G102, S016/S028 and S040/S102. Exact incident causes/spread, suppression resources, seals, venting and safety rules remain open. |

### Resolved choice: should containment use physical compartments?

| Option | Form | Trade-off |
|---|---|---|
| A — Coarse compartments | Incidents occupy authored areas on the ship map; boundaries, seals and team access affect spread and response. | Makes layout and access consequential without requiring individual room-by-room survival play. |
| B — System conditions only | Incidents attach to Systems/components; containment acts on those conditions without a separate compartment state. | Fewer spatial states to track, but less interaction between sealing, access and shared areas. |

**Selected: A.** Incidents occupy coarse authored compartments on the ship map. Boundaries, fitted seals and team access affect spread and response; ordinary individual System AI executes delegated containment using known conditions and team information. Not every condition spreads. Exact incident causes, suppression resources, seals/venting and atmosphere rules remain open; S102 reviews atmosphere scope later. No first-person firefighting, fine fluid simulation or individual oxygen model is selected here.

**Architecture refinement — AD75:** movement/access and applicable atmosphere/hazard mechanisms bind to one physical door/hatch state, retaining their own properties and permissions. Commanded closure is distinct from an actual seal, and operator knowledge remains observation-based. Atmosphere stays coarse. AD76 requires automatic routing to respect deliberate seals and request an authorised access change; further behaviour detail is deferred. See the [shared boundary decision](./architecture/index.html#compartment-boundaries).

## S030 — Self-destruct interlocks

**Selected direction: optional self-destruct through a distributed operational procedure.** [The unscheduled Plan baseline](../gdd/future/planned-not-scheduled.md#self-destruct) requires dedicated interlocks on human-seeking surfaces and a visible cancellable countdown. The redesign makes the required links meaningful System operations: arm self-destruct, prime the reactor, override safeties and other authored steps. These examples are not a final exhaustive sequence. This is Proposal/Plan, not Current capability.

**User correction to the vote framing:** every required link must be completed; any missing link stops completion. Do not substitute a two-person or majority vote for the operational procedure. No numerical approval threshold is selected. Keep relevant System ownership, communication and explicit authorisation; the role of AI in completing authorised steps still needs resolution.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional authored procedure across relevant Systems, with every required link necessary and progress/blockers legible. Retain supported physical consequences. |
| Ongoing activity | Monitor an armed countdown and opportunities to cancel, evacuate or withdraw; no routine self-destruct upkeep. |
| Active decisions | Arm, prepare required Systems, override supported safeties, coordinate escape/recovery, cancel or commit. |
| Complexity and AI | Accessible at every Station depth where an interlock is assigned; ordinary AI executes only authorised operation. Two-to-eight-player support must not require a full six-person crew. |
| Interactions and communication | Crew agreement precedes arming; team/craft operators manage recovery and evacuation; nearby ships can experience actual consequences. Optional GM/scenario authority remains distinct from ordinary player consent. |
| Risks | A vote replaces operational cooperation, meaningless duplicate confirmation buttons, absent seats prevent legitimate use, or AI invents authorisation. |
| Dependencies | G002/G005/G011/G012/G061/G071/G081, S011/S017 and later reactor/command/evacuation review. Exact steps, ordering, AI execution, cancellation and consequences remain open. |

### Superseded framing: role interlocks versus distinct-human votes

| Option | Form | Trade-off |
|---|---|---|
| A — Role interlocks | Activate all assigned interlocks; one human can deliberately activate several if they hold the relevant controls, as in the existing Plan. | Preserves role-based procedures and consolidation, but may allow one person to authorise the whole action. |
| B — Distinct humans | Require agreement from more than one human, with the rule suitable for two-to-eight-player crews rather than requiring six occupied Stations. | Ensures actual crew agreement, but needs a clear threshold and separate treatment of AI-only ships. |

**Replaced by the procedure clarification above.** The initial B response must not be extended into a numerical vote/quorum rule. Required System operations, rather than counting approvals, govern completion. Human initiation and ordinary AI execution are resolved below.

### Resolved dependency: can ordinary AI execute an explicitly authorised procedure step?

Two-to-eight-player support means some required Systems may be AI-controlled. Distinguish carrying out an explicit authorised instruction from independently deciding to start or approve self-destruct. Existing individual System AI, limited knowledge and communication remain unchanged.

| Option | Form | Trade-off |
|---|---|---|
| A — AI executes authorised steps | Ordinary AI can perform its required operation when explicitly authorised through the procedure; it cannot supply missing authorisation or skip a link. | Supports smaller crews without requiring every relevant System to be human-held. |
| B — Human execution required | Required critical operations must be performed by humans, who take control of the relevant Systems if necessary. | Keeps every critical action directly human-operated, but adds handoffs for smaller crews. |

**Selected: A, with human initiation and timed announced AI steps.** A human always starts the procedure when humans are present. If the next required step is AI-controlled, ordinary individual System AI executes it, taking a few seconds and announcing it through normal text/report channels. No AI voice is introduced. It does not skip missing links or act for human-held Systems. Exact sequence, timing, cancellation and initiation policy for ships without humans remain open; no numerical vote threshold is selected.

## S031 — Reactor / power generation

**Inclusion recommendation: retain generation and authored reactor sidegrades, and resolve live output control.** [The Current/Plan power baseline](../gdd/mechanics/power-resource-network.md) distinguishes current generation/allocation from planned supply-demand networks and thermal overload. G074 selects manageable authored hardware profiles rather than a reactor builder. [R3's EmptyEpsilon reference](research/README.md) motivates power/heat/coolant trade-offs; it does not settle Phoenix reactor response times.

Selected power rules already define flow into local System batteries, fullness-dependent loss, charging/depletion heat and limited sustainable capacity. The provisional complexity ladder groups allocation and delegates more at lower depth. Do not replace those decisions with a new direct-output multiplier or require every player to operate a reactor. The remaining choice is whether reactor generation itself is a controllable operating state distinct from distribution.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Authored reactor profiles with real capacity/condition limits. Decide whether generation automatically follows demand or supports a commanded output with finite response. |
| Ongoing activity | Listen to expected consumption and maintain useful reserves/thermal margin; ordinary reactor AI handles delegated output decisions. |
| Active decisions | Prepare for a high-demand operation, reduce exposure/thermal load or recover from a damaged supply path. Distribution remains a separate operating responsibility. |
| Complexity and AI | Minimum presets delegate detailed operation under priorities; deeper presets can expose output if selected. Preserve existing individual System AI and no unique temporary Backfill policy. |
| Interactions and communication | Other operators communicate expected work; Engineering prepares supply and local reserves. Forecasts use known loads and stated assumptions, not hidden future actions. |
| Risks | Another slider always left at maximum, unnecessary repeated ramping, output control confused with allocation, or abrupt demand changes defeat the promised crisis margin. |
| Dependencies | G021/G055/G060/G074/G114, selected battery/thermal model and S032–S039. Exact output curve, ramp rate, thermal/signature costs and fuel scope remain open. |

### Resolved choice: is reactor output itself a player-controllable operating state?

| Option | Form | Trade-off |
|---|---|---|
| A — Automatic demand following | Generation follows available demand within equipment limits; humans mainly allocate flow and manage cooling/reserves. | Keeps attention on distribution without a separate reactor-output task. |
| B — Commanded output with ramping | Select an output/posture that takes time to reach; higher generation has meaningful authored operating costs. Ordinary AI handles it when delegated. | Rewards preparing supply and managing margin, but needs useful reasons to choose less than maximum. |

**Selected: B, with shared System mechanics for reactor heat, coolant and capacitor reserve.** Reactor output is commanded with finite response. The reactor has its own heat, coolant and capacitor reserve, which it can draw on when power draw is set high. Use the same reusable mechanics/code as other Systems, preserving individual reactor AI, ordinary communication and limited knowledge. The capacitor is the reactor's local reserve, not an automatically added second energy meter. Exact generation/output curves, reserve recharge, sustainable/burst limits, ramp rates, fuel involvement and costs remain open. Keep stable operation forgiving and authored reactor profiles as sidegrades under G074.

## S032 — Battery / energy reserve

**Inclusion recommendation: retain local energy buffers and resolve whether a separate main battery is needed.** [The Current baseline](../gdd/mechanics/power-resource-network.md) has a ship battery buffering excess demand and surplus generation. The redesign already selects local batteries per System and S031 now gives the reactor its own capacitor reserve. S006 rejects an additional shot-energy capacitor layer on top of weapon-local storage. Do not introduce redundant reserves by carrying every historical meter forward without a distinct purpose.

The reactor reserve can be the main supply buffer feeding consumers, while their local reserves support immediate operation. Alternatively, a separate main battery can sit alongside the reactor and have a distinct fitting/damage role. Either uses the same storage mechanics; this choice concerns the default equipment topology rather than networking or personal complexity.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Reactor and consumer reserves already selected. Decide whether the default model additionally includes a separate main battery System. |
| Ongoing activity | Prepare useful reserves, monitor known depletion/headroom and avoid inefficient permanent fullness. Ordinary AI executes delegated reserve policy. |
| Active decisions | Spend reserve for a timed operation, preserve margin for recovery or reprioritise flow when supply is damaged. |
| Complexity and AI | Simplified presets expose actionable margins; deeper tools show relevant stores/flows without adding fictional capacity. Stable reserves provide crisis wriggle room. |
| Interactions and communication | Engineering prepares supply and communicates endurance under stated load assumptions; other operators describe intended consumption. No hidden prediction of their future actions. |
| Risks | Three storage layers with indistinguishable jobs, permanent full-charge chores, accidental free energy, or one empty store causing an unexplained global lock. |
| Dependencies | Selected fullness-loss/thermal rules, S006/S022/S023/S031 and S033/S036/S038. Exact capacities, flow limits, charging and exhaustion effects remain open. |

### Resolved choice: is there also a separate main ship battery?

| Option | Form | Trade-off |
|---|---|---|
| A — Reactor reserve is the main buffer | Use the reactor capacitor plus consumer-local batteries as the default model, without another mandatory ship-wide store. | Fewer overlapping meters and a clear supply-to-consumer relationship. |
| B — Separate main battery too | Add a distinct main battery System alongside reactor-local and consumer-local reserves. | Adds fitting and damage redundancy, but needs a distinct operating purpose. |

**Selected: A.** The reactor capacitor is the default main supply buffer alongside consumer-local batteries, without another mandatory ship battery. Use the shared storage mechanics. Additional storage remains an authored equipment possibility, not a selected default third layer. Exact capacities, charging/flow limits, optional equipment connections and exhaustion effects remain open; preserve useful prepared crisis margin.

## S033 — Power distribution

**Inclusion recommendation: retain allocated flow and the user's complexity ladder; resolve allocation inside coarse groups.** [The Current baseline](../gdd/mechanics/power-resource-network.md) has Helm/Weapons/Shields levels; Plan C4 adds network delivery. The redesign instead selects flow into local batteries, with Engineering owning allocation and other operators communicating needs. The user's ladder proposes dozens of fine groups at maximum depth, several groups at middle depth, priority-ordered groups with AI allocation at minimum, and full AI operation when delegated.

Coarse groups must still feed individual physical System reserves. Grouping is a control abstraction, not a different ship simulation or new physical connection. Decide whether a group's flow splits by fixed authored proportions or whether ordinary distribution AI allocates within its budget using observed consumption and desired reserves. Human-controlled individual flows remain explicit at fine depth.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain power distribution, authored Station complexity presets and Engineering's final allocation authority. Resolve how a grouped budget maps to constituent System flows. |
| Ongoing activity | Listen to current/upcoming work, prepare reserves and balance useful flow against fullness loss and thermal capacity. |
| Active decisions | Ration scarce supply, prioritise a critical operation or prepare a reserve before demand rises. |
| Complexity and AI | More detailed presets expose finer controls; minimum uses the selected priority-list concept. Ordinary distribution AI handles delegated allocation with known demand/intent, not hidden future player actions or a new central planner. |
| Interactions and communication | Operators request resources and explain upcoming work; Engineering adjusts budgets/flows. Any automatic internal changes must be legible when consequential. |
| Risks | A coarse group strands energy in unused Systems, automatic allocation defeats human intent, presets change physical outcomes, or a low-depth user must tune many hidden exceptions. |
| Dependencies | G002/G003/G011/G021/G114, selected power/thermal model, S031/S032 and S034/S038/S039. Exact grouping, reserve policy, flow limits and scarcity behaviour remain open. |

### Resolved choice: how should a coarse group's power budget be split?

| Option | Form | Trade-off |
|---|---|---|
| A — Fixed proportions | Divide group flow using authored proportions among its Systems. | Predictable and simple, but can waste capacity when only part of the group is active. |
| B — Adaptive within budget | Ordinary distribution AI divides the assigned budget using available demand, reserve goals and priorities. | Keeps coarse controls useful as activity changes, but needs clear intent and feedback. |

**Selected: B.** Ordinary distribution System AI divides assigned group budgets using available demand, reserve goals and priorities. Fine-depth human allocations remain theirs; Engineering retains final allocation authority. Grouping does not alter physical reserves/network rules or add per-System complexity overrides. Advance preparation still depends on communicated intent. Exact grouping, reserve policy, flow limits and scarcity handling remain open.

## S034 — Coolant allocation / thermal circulation

**Inclusion recommendation: retain consumable coolant and resolve the provisional recovery loop.** [Plan C4](../gdd/mechanics/power-resource-network.md) provides thermal/coolant foundations; [R3's EmptyEpsilon reference](research/README.md) motivates allocation against heat. G055 explicitly selects consumable coolant, while the user proposed recycling used coolant at a loss as a provisional idea. No claim is made that the reference game supplies Phoenix's proposed recycling model.

Reactor and consumer Systems use the same thermal/coolant mechanics under S031. Coolant operation should extend useful output and recovery time while drawing down real supplies. A recycling System could reclaim some used stock at finite throughput and operating cost, leaving net loss. It must not create coolant or make waste heat vanish; heat rejection is reviewed separately at S035.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain finite coolant and delegated allocation. Decide whether suitable equipment can recover used coolant at a loss. Exact collection/storage and cooling equations remain open. |
| Ongoing activity | Balance cooling against stocks and expected endurance, listening to upcoming high-output work. Ordinary AI handles routine allocation and, if selected, recycling. |
| Active decisions | Support a hot System, conserve coolant, recover from a fault or arrange resupply. Recycling can offer endurance at the cost of other operating capacity. |
| Complexity and AI | The proposed minimum-depth no/low/high usage control delegates detailed allocation; higher depth exposes grouped/finer choices. No mandatory manual circulation or recycling-batch clicks. |
| Interactions and communication | Engineering supports thermal margin; other operators report sustained/burst needs; Inventory supplies coolant and recovery buffers where fitted. Clear feedback explains stock loss and bottlenecks. |
| Risks | A perpetual coolant loop, recycling confused with heat rejection, too many inventories, cooling as repetitive maintenance or instant recovery after every burst. |
| Dependencies | G055/G060/G065/G111, selected thermal model, S031/S033 and S035/S038/S039. Exact yields, throughput, energy costs, buffers and damaged-loop effects remain open. |

### Resolved choice: should suitable ships recycle used coolant?

| Option | Form | Trade-off |
|---|---|---|
| A — Expend and resupply | Cooling consumes stock without onboard recovery; endurance depends on conservation and resupply. | Simpler stores and fewer interacting Systems, but fewer ways to trade resources for endurance. |
| B — Lossy recycling equipment | Suitable ships recover a fraction of used coolant through finite-capacity equipment with operating costs; net consumption remains. | Adds an endurance/power/capacity trade-off, but needs clear recovery and heat-rejection rules. |

**Selected: B.** Suitable ships can fit equipment that recovers a fraction of used coolant at finite throughput and operating cost, with net consumption remaining. Ordinary recycling System AI runs routine work automatically without batch-click chores. Recovery does not itself reject waste heat. Exact collection/storage, yield, throughput, power/thermal costs and damaged-loop effects remain open. This adopts the previously provisional recycling idea from G055.

## S035 — Heat rejection / radiators

**Inclusion recommendation: retain thermal dissipation and consider optional controllable radiator equipment.** [Plan C4's baseline](../gdd/mechanics/power-resource-network.md) includes thermal recovery/dissipation and authored reactor sidegrades. R2 collects thermal operating constraints, but does not establish the exact deployable-radiator mechanism proposed here. G021 selects observable operating signatures; S026/S027 select emission-based exposure and advisory human emission profiles.

S034 separates reclaiming coolant from rejecting waste heat. Thermal recovery can remain an authored passive rate, or suitable ships can also operate radiator equipment that trades stronger heat rejection against exposure and vulnerable hardware. Exact heat transport is not settled by selecting radiators, and this entry does not require a fluid simulation.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain authored thermal dissipation. Decide whether suitable ships also have controllable radiator deployment/output. |
| Ongoing activity | Maintain thermal margin under actual workloads while ordinary AI handles delegated cooling posture. Stable operation must not require radiator cycling by hand. |
| Active decisions | Accept heat for a burst, improve cooling before sustained work, reduce emissions or protect exposed cooling equipment. |
| Complexity and AI | Simple presets delegate cooling posture; deeper controls expose the meaningful equipment choice. All depths use the same thermal and damage rules. |
| Interactions and communication | Engineering communicates recovery limits; Sensors/Comms explain exposure where supported; Helm can protect vulnerable aspects; other operators plan sustained versus burst workloads. |
| Risks | An always-deployed obvious setting, too many cooling meters, radiators granting free heat deletion, or unsupported claims of exact enemy detection. |
| Dependencies | G021/G055/G057/G074, selected thermal rules, S016/S026/S027/S031/S034 and S039. Exact heat transport, dissipation rates, deployment, signatures and damage consequences remain open. |

### Resolved choice: should suitable ships have controllable radiators?

| Option | Form | Trade-off |
|---|---|---|
| A — Passive dissipation | Heat rejection is an authored equipment property; players manage workload and coolant without separate radiator operation. | Fewer controls and a clearer cooling workload, but fewer exposure/recovery decisions. |
| B — Controllable equipment too | Suitable ships can deploy or adjust radiators for stronger rejection, trading thermal recovery against signature and exposed hardware. | Connects endurance, concealment and protection, but needs useful alternatives to permanent maximum cooling. |

**Selected: A.** Heat rejection is a passive authored equipment property. Players manage workload, coolant and recovery without controllable radiator deployment/output. The earlier controllable-radiator recommendation is not selected. Ordinary heat signatures and damage to fitted equipment still follow shared rules. Exact dissipation rates, condition effects and thermal transport remain open.

## S036 — Capacitors / weapon energy buffers

**Retained through existing selected decisions; no new energy layer.** [R2's capacitor/resource comparisons](research/functional-ship-customisation.md) supply reference mechanisms. S006 already selects weapon-local batteries without an additional shot-energy capacitor; S031 gives the reactor a reserve using the same mechanics; S032 selects that reserve as the main supply buffer. Equipment-specific timed preparation and discharge behaviours remain possible. Ordinary AI manages delegated readiness under known intent, actual flows and reserves.

Preparing and spending margin supports listening, timing and crisis tolerance. Additional meters would duplicate an already resolved decision, so no new question is required. Exact capacities, charge/discharge limits and equipment preparation remain open for tuning rather than a second storage model.

## S037 — Overpower / overcharge control

**Inclusion recommendation: consider equipment-supported overdrive and resolve whether operation can exceed normal ratings.** [Plan C4](../gdd/mechanics/power-resource-network.md) and [R1 #27/R3](research/README.md) include improved output at thermal cost. Selected distribution rules differ from Current direct power multipliers: incoming flow fills local batteries, while equipment consumes stored energy. Increasing flow alone must not silently boost a weapon or engine's output.

S031 selects commanded reactor output and a reserve for high draw; S035 selects passive dissipation. An overdrive mode could make a supported consumer do more work per unit time while drawing more energy and accumulating heat or authored stress. Alternatively, burst capability can consist only of sustaining normal-rated output from reserves.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Decide whether suitable equipment supports operation above normal sustainable ratings. This is distinct from charging its reserve faster or increasing reactor supply. |
| Ongoing activity | Maintain thermal/reserve margin and monitor supported stress; ordinary System AI operates within its selected risk policy. |
| Active decisions | Authorise a useful burst, stop before damaging conditions escalate or accept a supported risk to complete an operation. |
| Complexity and AI | Simple presets delegate supported modes; deeper operators choose output/risk without mandatory extra sliders for every System. Same physical limits and conditions apply to AI and humans. |
| Interactions and communication | The owning operator controls equipment output; Engineering supplies flow/cooling and reports endurance. Requests and forecasts connect burst timing to the rest of the crew. |
| Risks | Always-on upgrades, power-flow sliders becoming magic output boosts, opaque random damage, or punitive wear during ordinary healthy operation. |
| Dependencies | G057/G059/G060/G114, selected storage/thermal model, S006/S031/S033–S035 and S038. Exact supported modes, costs, stress thresholds, shutdown and recovery remain open. |

### Resolved choice: can suitable Systems operate above normal ratings?

| Option | Form | Trade-off |
|---|---|---|
| A — Equipment-supported overdrive | Suitable Systems offer higher output with increased energy draw, heat and authored stress/risk. | Adds consequential burst decisions, but needs clear limits and meaningful costs. |
| B — Rated output only | Reserves sustain normal output during supply shortfalls; equipment cannot exceed its normal ratings. | Simpler capability limits, with bursts expressed through endurance rather than stronger output. |

**Selected: A.** Suitable Systems can exceed normal ratings through equipment-supported overdrive with increased energy draw, heat and authored stress/risk. The owning System controls output; Engineering supplies flow/coolant. Flow alone does not directly boost performance. Ordinary individual System AI follows selected risk policy. Exact modes, multipliers, costs, stress thresholds, shutdown and recovery remain open; no universal random damage roll or compulsory overdrive mode is selected.

## S038 — Presets, load shedding and emergency recovery

**Inclusion recommendation: retain prepared operating intent and local scarcity feedback; resolve empty-battery recovery.** [The Current baseline](../gdd/mechanics/power-resource-network.md#exhaustion-and-recovery) applies a global allocation lock at battery exhaustion. Plan C4 adds local degradation. S032 selects reactor and consumer-local reserves, and S033 selects adaptive allocation within coarse budgets. Do not carry a global lock into this model merely because it exists in Current.

Authored Station complexity presets remain distinct from operating postures. Operating intent can guide normal AI and priorities without replacing a human's local authority or restoring every past setting after Backfill. This entry resolves whether an empty local battery causes a generic restart lock, or equipment continues/restarts as actual energy permits.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain clear priorities, reserve forecasts and supported operating postures. Decide the default consequence of local battery exhaustion, without changing equipment-specific preparation or thermal shutdown. |
| Ongoing activity | Monitor current/expected demand and recovery while ordinary AI sheds lower-priority delegated work under normal policy. |
| Active decisions | Reprioritise flow, stop costly work, choose an affordable alternative or prepare recovery before resuming a demanding operation. |
| Complexity and AI | Simple presets delegate detailed allocation; deeper operators retain relevant flow/output choices. Normal AI hysteresis can prevent pointless cycling; no unique temporary takeover policy. |
| Interactions and communication | Affected Systems report energy-limited capability; Engineering reports recovery under known load assumptions; other operators adjust plans rather than discover an unexplained global lock. |
| Risks | Empty batteries create free-energy loopholes, all controls lock together, cheap functions fail behind arbitrary thresholds, or equipment flickers rapidly on/off. |
| Dependencies | G003/G009/G011/G114, selected storage model, S031–S037 and later equipment review. Exact action costs, thresholds, reserve policies and feedback remain open. |

### Resolved choice: what happens when a local battery runs empty?

| Option | Form | Trade-off |
|---|---|---|
| A — Energy-limited operation | Perform only affordable work: continuous functions reduce/stop as needed and discrete actions wait for enough energy. No generic restart lock. | Keeps scarcity local and grounded in actual capability, but needs clear status and stable operation. |
| B — Restart reserve required | Exhaustion shuts the System down until its battery reaches an authored recovery threshold. | Makes exhaustion a strong commitment, but adds downtime beyond the energy cost of the next action. |

**Selected: A.** Continuous functions reduce or stop as energy permits; discrete actions wait until affordable. No generic exhaustion restart lock or automatic global allocation lock is added. Ordinary System AI uses normal priorities/hysteresis and reports supported capability/recovery. Equipment-specific startup, preparation and thermal shutdown can still apply. Exact action costs and equipment recovery rules remain open.

## S039 — Containment / resource conduits and buffers

**Inclusion recommendation: retain authored resource dependencies and resolve spatial conduit damage.** [Plan C4](../gdd/mechanics/power-resource-network.md) represents connected generation/storage/consumption. G057's example makes each reactor-to-System link a component that can become worn, intermittent, EMP-disabled or destroyed. S028 selects fitted switching and team-built physical bypasses; S016 selects hit paths through the repair map. These do not yet specify whether links occupy spatial routes or only have endpoint components.

The model can use logical links damaged at assigned connection equipment, or authored route segments that passing damage can sever. Both preserve real capacity and connectivity without detailed circuit/fluid simulation. Coarse segments can represent meaningful shared routes without modelling every metre of cable or pipe.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Authored links, capacity and supported isolation/redundancy. Decide whether physical conduit routes participate in the damage map. |
| Ongoing activity | Monitor known delivery faults and remaining capacity; ordinary AI operates supported routing and delegates repair. |
| Active decisions | Isolate a dangerous segment, use an installed alternate route or assign a feasible bypass/restoration task. |
| Complexity and AI | Simple presets show affected Systems and useful alternatives; deeper tools expose relevant network detail. No manual connection puzzle or different physical topology by player depth. |
| Interactions and communication | Engineering explains lost service; repair teams restore relevant components/segments; affected operators ration local reserves while work proceeds. |
| Risks | Every grazing hit severs half the ship, invisible shared dependencies, duplicated repair targets, or unlimited rerouting around destroyed physical links. |
| Dependencies | G057/G058/G059/G070/G075, S016/S028/S031–S038. Exact routes, shared trunks, capacities, condition effects and bypass recipes remain open. |

### Resolved choice: do conduits occupy damageable routes on the ship map?

| Option | Form | Trade-off |
|---|---|---|
| A — Logical links / connection components | Model connectivity and damageable connection equipment, without spatially tracing conduits through the hull. | Simpler map and damage reasoning, but less relationship between a breach location and service loss. |
| B — Coarse physical routes | Author conduit segments on the map; hits can damage routes and affect the Systems they supply. | Connects breaches, redundancy and repair location, but needs readable routes and manageable shared failures. |

**Selected: B.** Author coarse physical conduit segments on the ship map. Hits can damage routes and interrupt the Systems they supply; fitted redundancy and physical bypass work follow actual topology/capacity. Ordinary individual System AI operates under known conditions. Use meaningful segments without detailed cable, circuit or fluid simulation. Exact routes, shared trunks, capacities, condition effects and bypass recipes remain open.

## S040 — Life support

**Inclusion recommendation: retain a damageable life-support consumer and resolve whether survival pressure is compartment-based.** [Plan C4's baseline](../gdd/mechanics/power-resource-network.md) includes authored ship-wide or compartment survival clocks, degraded crewed capability and evacuation pressure. [R1 #57](research/feature-gap-analysis-master.md) offers life support as managed equipment, not a validated workload model. G071 permits explicitly lethal personnel events; S029 selects coarse compartments and S039 physical supply routes.

Life support should be routinely viable under ordinary delegation and become consequential through damage, isolation or exceptional demands. Decide whether loss creates a shared ship-wide survival margin or degrades actual affected compartments, allowing teams to relocate and protected areas to remain useful. Detailed pressure/atmosphere modelling stays for S102.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Damageable life-support equipment using shared resources and condition rules. Decide the geographic scope of survival margin and degraded crewed operation. |
| Ongoing activity | Maintain ordinary viability automatically; observe supported reserve/condition warnings. No routine hunger, sleep or individual oxygen chores. |
| Active decisions | Restore supply, prioritise a safe area, relocate teams or evacuate when conditions become unsafe. Consequential sacrifice follows normal authorisation rather than unannounced AI choices. |
| Complexity and AI | Simple presets delegate maintenance and expose clear risk; deeper tools show affected areas and support limits. Same physical conditions apply to human/AI-controlled Systems. |
| Interactions and communication | Engineering/repair restore equipment and conduits; team operators manage access/relocation; affected Stations report impaired crewed capability; evacuation uses actual available craft/equipment. |
| Risks | A generic timer ignores local damage, healthy life support becomes constant busywork, instant casualties deny prepared margin, or compartment details force individual survival simulation. |
| Dependencies | G009/G057/G065/G068/G070/G071, S029/S038/S039 and later medical/evacuation/S102 review. Exact safe/degraded/lethal conditions, reserves, rates and relocation rules remain open. |

### Resolved choice: is life-support failure local to compartments?

| Option | Form | Trade-off |
|---|---|---|
| A — Ship-wide survival margin | Failure reduces a shared survival reserve and crewed capability, with a clear ship-level warning/deadline. | Simple to understand, but offers fewer options based on isolation and relocation. |
| B — Compartment survival margins | Actual affected compartments become unsafe over time; intact supported areas remain usable and teams can relocate where access permits. | Connects containment, repair and evacuation, but needs readable local status. |

**Selected: B.** Affected compartments become unsafe over time; intact supported areas remain usable and teams can relocate where access permits. Ordinary individual System AI handles routine viability and reports known limits. Preserve prepared intervention margin rather than applying an automatic all-ship countdown. Exact reserves, safe/degraded/lethal conditions, rates, relocation, atmosphere simulation and casualty rules remain open.

## S041 — Main-drive propellant / fuel supply

**Inclusion recommendation: retain finite authored propulsion resources and resolve normal-drive consumption scope.** [Plan C4's baseline](../gdd/mechanics/power-resource-network.md) includes main-drive propellant as bulk stock, with later warp fuel in Plan C6. [R1 #57](research/feature-gap-analysis-master.md) offers fuel management as a reference candidate. Current scenario-local transfers do not establish universal fuel depletion. G065/G067 already select stock targets, timed replenishment and supported port restocking.

The redesign can retain propellant consumption for normal propulsion, or focus finite fuel on long-distance travel while normal movement consumes electrical energy only. This is equipment/resource scope, not a networking or capital-ship movement change. Exact thrust, travel mechanics and any fuel-free equipment variants remain for their relevant reviews.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Authored physical stocks and consumption where selected; decide whether normal propulsion participates as well as long-distance travel. |
| Ongoing activity | Monitor projected endurance under stated activity assumptions; ordinary AI handles routine metering and local replenishment. |
| Active decisions | Preserve withdrawal reserve, choose a route or resupply stop, support another ship or accept a costly pursuit. |
| Complexity and AI | Simple presets show actionable endurance/reserve warnings; deeper tools expose relevant consumption and targets. No manual fuel-pumping chore. |
| Interactions and communication | Helm/Navigation communicates movement intent, Engineering/Inventory explains supply constraints and Comms arranges supported resupply. Forecasts do not know future manoeuvres. |
| Risks | Routine movement constantly needs refuelling, fuel costs discourage useful combat manoeuvres, stranded players have nothing to do or separate resources duplicate identical decisions. |
| Dependencies | G027/G038/G044/G065/G067/G090/G108, S031/S039 and later propulsion/travel review. Exact fuel categories, consumption curves, reserves, transfers and recovery alternatives remain open. |

### Resolved choice: does normal propulsion consume finite propellant?

| Option | Form | Trade-off |
|---|---|---|
| A — Normal propulsion too | Normal drives consume authored propellant as well as operating energy; long-distance travel can have its own equipment-dependent costs. | Makes pursuit, withdrawal reserves and resupply consequential, but needs generous routine endurance. |
| B — Long-distance fuel only | Normal movement uses electrical energy; finite fuel constrains long-distance travel rather than ordinary manoeuvres. | Keeps tactical movement freer, but narrows propulsion logistics mostly to travel. |

**Selected: B.** Ordinary movement consumes electrical energy rather than finite propellant. Finite fuel constrains long-distance travel. This differs from the Plan C4 main-drive propellant candidate and is a proposal choice, not a Current implementation change. Ordinary AI meters supported consumption/restocking; exact travel-fuel types, consumption, reserves, transfers and recovery alternatives remain open.

## S042 — Repair materials and fabrication stock

**Inclusion recommendation: retain typed repair resources and resolve cannibalisation of usable onboard equipment.** [Plan C4's baseline](../gdd/mechanics/power-resource-network.md) includes bulk repair material. G058 selects typed parts and team work, G064 permits most component replacements, G065 selects timed stock replenishment and S014 permits some shared fabrication/repair inputs. G046 already allows salvage of surviving useful components/materials; that does not yet authorise dismantling working equipment aboard the player's ship.

Cannibalisation could supply a compatible part or recoverable material at the cost of losing donor capability. It should follow real equipment compatibility, condition and work/access limits rather than turn any component into any other. Decide whether this deliberate sacrifice belongs in the proposal.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain finite typed parts/materials and automated stock targets. Decide whether usable onboard equipment can be stripped for compatible repair resources. |
| Ongoing activity | Preserve useful repair/fabrication reserves while ordinary AI tracks stock and executes routine replenishment. |
| Active decisions | Choose a costly restoration, arrange resupply or sacrifice a less-needed donor component if cannibalisation is selected. |
| Complexity and AI | Simple presets expose feasible work and donor consequences; deeper tools show compatibility and supported recovery. Ordinary AI executes approved work rather than silently removing another human's active equipment. |
| Interactions and communication | Repair/Inventory identifies supply options, the donor System's operator explains lost capability, and mission coordination weighs the sacrifice against the restoration. |
| Risks | Universal magic conversion, infinite dismantle/rebuild yield, automatic stripping of essential gear, or an inventory recipe puzzle becoming routine repair work. |
| Dependencies | G046/G058/G064/G065/G072/G073, S014/S025/S028 and later salvage/logistics review. Exact donor compatibility, recovered condition/yield, access, timing and approval controls remain open. |

### Resolved choice: can crews cannibalise usable onboard equipment for repairs?

| Option | Form | Trade-off |
|---|---|---|
| A — Allow deliberate cannibalisation | Strip suitable components for compatible parts/materials, sacrificing actual donor capability and consuming team time. | Adds recovery options under scarcity, but requires clear consequences and bounded recovery rules. |
| B — Stocks and external salvage only | Repairs use carried supplies, fabrication, resupply or recovered wreck equipment; working onboard gear cannot be a donor. | Simpler capability ownership and repair choices, but fewer emergency alternatives. |

**Selected: A.** Crews can deliberately strip suitable usable onboard equipment for compatible parts/materials. Actual donor capability is lost, and recovery needs team work, access and time. Ordinary System AI executes approved work with explicit donor consequences; do not silently strip another human's active equipment. Compatibility and actual recovery limit repairs rather than universal component conversion. Exact donor controls, yields, recovered condition and recipes remain open.

## S043 — Internal logistics and delivery routes

**Inclusion recommendation: retain automated timed replenishment and resolve route dependence.** G065 already selects hold stores, manually set local stock targets, automatic replenishment over time and optional manual transfers. [R2/R4's Cosmoteer references](research/README.md) motivate physical delivery constraints; [R6's PlateUp! comparison](research/comparable-games-and-falling-frontier.md) motivates buffers and readable throughput without restoring automated chores. Do not re-ask whether players must move every item.

S029 selects compartment boundaries/access, G070 gives teams map-based routing and S039 gives resources damageable conduit routes. Inventory delivery can likewise use reachable paths, or remain an abstract transfer timer unaffected by internal access. Route dependence does not require simulating every carried item or individual porter.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Automatic delivery to configured stock targets. Decide whether map connectivity and blocked access govern those transfers. |
| Ongoing activity | Monitor relevant stocks, delivery times and bottlenecks while ordinary logistics System AI schedules routine replenishment. |
| Active decisions | Change stock targets/priorities, reopen a blocked path or pre-position supplies before isolating an area. Manual transfers remain optional. |
| Complexity and AI | Simple presets show low stock and actionable blocked deliveries; deeper tools expose relevant routes/throughput. Ordinary AI finds supported alternatives without manual path clicks. |
| Interactions and communication | Operators report expected consumption, repair/containment operators explain access changes and Inventory reports delivery consequences. Local buffers provide time to react. |
| Risks | Every stock item becomes a moving actor, doors require constant babysitting, routine replenishment silently fails or route detail makes automation ineffective. |
| Dependencies | G065/G070/G111, S004/S014/S029/S039/S042 and later cargo/team review. Exact handling capacity, routing, safety constraints and physical delivery representation remain open. |

### Resolved choice: do internal supply deliveries depend on map routes?

| Option | Form | Trade-off |
|---|---|---|
| A — Abstract timed transfers | Available stock moves after an authored delay, without checking internal compartment routes. | Fewer interacting constraints, but containment and local supply are largely independent. |
| B — Route-aware delivery | Transfers need a reachable internal path; blocked or hazardous routes can delay/prevent delivery, with supported alternatives chosen automatically. | Connects stock preparation, containment and repair, but needs clear bottleneck feedback. |

**Selected: B.** Deliveries need reachable paths through the authored ship map. Blocked or hazardous routes can delay/prevent supply; ordinary logistics System AI chooses supported alternatives automatically. G065's manually set stock targets, timed replenishment and optional manual transfers remain. Exact handling capacity, safety constraints and delivery representation remain open; individual porter simulation or per-item movement tracking is not selected.

**Architecture refinement — AD58:** teams and internal deliveries use authored compartment connections, including explicit links between decks. Reuse topology and route queries with activity-specific permissions, safety, costs and travel rules; ordinary controllers choose work and independent jobs execute it. Known conditions guide planning, while actual access governs execution and encountered blockages produce ordinary reports. No detailed movement inside rooms is required. Resource conduits and damage geometry remain separate. AD59 selects aggregate batches for stock in transit. See [internal route topology](./architecture/index.html#internal-route-topology).

**Architecture refinement — AD59:** dispatch moves a resource quantity out of its source stock into an aggregate batch with route stage/progress; arrival moves it into destination stock. Interrupting or cancelling work preserves actual in-transit stock rather than refunding or deleting it. Supported waiting, rerouting, recovery and physical hazards use the ordinary mechanisms and knowledge limits. Routine replenishment remains automated; batches require neither per-item nor individual-porter simulation. AD60 reserves compatible receiving space at dispatch. See [internal delivery state](./architecture/index.html#internal-delivery-state).

**Architecture refinement — AD60:** dispatched batches reserve compatible receiving capacity; other ordinary loading respects the claim and undispatched requests reserve no space. Reuse stage-start commitments tied to a delivery and destination. Arrival converts the claim into stock occupancy; refusal, diversion, cancellation or loss must reconcile claims consistently with actual batch state. Damage or isolation can still prevent receipt. Ordinary logistics AI handles routine work; exact arbitration and invalidation/recovery contracts remain open. See [receiving-capacity reservations](./architecture/index.html#delivery-capacity-reservation).

**Architecture refinement — AD61:** routine internal deliveries use the logistics System's authored throughput and concurrent-work limits, affected by fitted requirements, resources, conditions and routes. Shared capacity and job mechanisms bound the work; ordinary logistics AI schedules it without assigning a personnel team to each batch. Existing batches retain their actual state when handling capability changes. Repair/security teams and the shared framework for explicitly modelled workforce remain. Exact rates, concurrency accounting and interruption rules remain open. See [internal handling capacity](./architecture/index.html#internal-handling-capacity).

## S044 — Main engines / thrust and reverse

**Inclusion recommendation: retain Helm-owned propulsion and resolve the default throttle meaning.** [The Current movement baseline](../gdd/mechanics/movement.md) separates Helm actuators from Navigation destinations and advisory Tactical bearing requests. G027 retains planar capital movement with limited vertical collision avoidance, G032 selects Match Speed without automatic steering, and S041 selects electrical ordinary propulsion. Those choices do not settle whether the normal throttle commands desired speed or direct thrust.

Both interfaces can use the same acceleration, energy, damage and recoil physics. Assisted speed control commands actuators to reach a desired speed; direct thrust requires the player to manage acceleration/braking. Complexity differences must not change the actual ship's capabilities or give human/AI controllers different physics.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Helm owns propulsion; retain authored forward/reverse capability and real limits. Decide the default capital-ship throttle interface. |
| Ongoing activity | Maintain useful speed/position while listening to weapon arcs, protection, recovery and route needs. |
| Active decisions | Accelerate, brake, reverse or commit an approach/withdrawal within available reserve and thermal capacity. |
| Complexity and AI | Beginners can retain steering/basic targeting; normal assistance may manage actuator execution. Ordinary AI handles delegated travel using known intent, not new flight physics. |
| Interactions and communication | Navigation communicates destinations, Weapons requests useful aspects, Shields reports weak facings and Engineering explains sustainable propulsion. Helm retains movement authority. |
| Risks | Manual counter-thrust dominates attention, assistance ignores real braking limits, or direct power allocation secretly determines speed instead of battery-supported propulsion work. |
| Dependencies | G027/G032/G049/G114, S007/S009/S031/S037/S041 and S045–S048. Exact speed/reverse limits, braking response and control variants remain open. |

### Resolved choice: what should the default capital-ship throttle command?

| Option | Form | Trade-off |
|---|---|---|
| A — Desired speed | Set a speed; ordinary assistance accelerates/brakes toward it within actual actuator and resource limits. | Leaves attention for positioning and crew coordination, while still requiring anticipation of braking distance. |
| B — Direct thrust | Set forward/reverse thrust; players manage acceleration and counter-thrust to reach or hold useful speed. | Adds flight-control depth, but can consume attention during coordinated bridge work. |

**Selected: A.** Default capital-ship throttle commands desired speed. Ordinary assistance accelerates/brakes within actual actuator, resource and damage limits; braking distance remains consequential. Helm retains steering, with no automatic relative-position, range or aspect holding. Human and AI controllers use shared physics. Exact reverse/speed limits, response and advanced control variants remain open.

## S045 — Steering / yaw and attitude control

**Inclusion recommendation: retain Helm-owned steering and resolve the default manual steering interface.** [The Current baseline](../gdd/mechanics/movement.md) supplies yaw and hull-authored axes. G027 limits capital vertical movement to collision avoidance; full-3D small craft remain distinct. S044 selects desired speed; G032 explicitly leaves steering to the player during Match Speed. Do not introduce automatic target-relative orientation as part of speed matching.

Manual steering can mean holding left/right to command a turn, or choosing an absolute heading that assistance reaches and maintains. In either case the human chooses the direction, actual turning limits apply and no target is automatically tracked. Broadsides and exposed shield/armour aspects remain coordinated decisions under S007/S022/S025.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Capital planar yaw with authored steering capacity. Decide whether the default manual interface commands turn direction/rate or desired absolute heading. |
| Ongoing activity | Maintain useful aspect and watch relative geometry while listening to requests from other operators. |
| Active decisions | Choose a turn for weapons, protection, route access or an external operation; accept competing demands rather than obey every request automatically. |
| Complexity and AI | Both interfaces can use ordinary assistance without different physics. Full delegated steering remains ordinary whole-System AI; selecting a manual heading does not create a new AI planner. |
| Interactions and communication | Weapons supplies arc/penetration-angle advice, Shields reports weak facings, Navigation supplies route intent and Engineering explains actual steering limits. Helm chooses direction. |
| Risks | Assistance automatically chases a target, turns ignore momentum/damage, constant correction becomes rote or heading control is confused with an automatic route/range hold. |
| Dependencies | G027/G032/G114, S007/S009/S016/S044 and S046–S048. Exact yaw response, heading display, input takeover and alternate controls remain open. |

### Resolved choice: what is the default capital-ship steering input?

| Option | Form | Trade-off |
|---|---|---|
| A — Turn commands | Hold/use left-right steering to command a turn, with ordinary actuator stabilisation when released. | Direct and familiar, but precise large turns require ongoing input. |
| B — Desired heading | Choose an absolute heading; assistance turns toward and maintains it within physical limits. | Leaves attention for geometry and crew requests, but needs clear requested/current heading feedback. |

**Selected: B.** Helm chooses an absolute heading; ordinary assistance turns toward and maintains it within real steering, resource and damage limits. This does not automatically track targets, follow routes or maintain broadside alignment. G032 still leaves heading decisions with Helm during Match Speed. Exact yaw response, heading feedback and alternate controls remain open.

## S046 — Lateral / vertical manoeuvring thrusters

**Inclusion recommendation: retain hull-authored manoeuvring capability and resolve lateral combat scope.** [The Current movement baseline](../gdd/mechanics/movement.md) supports lateral/vertical actuators where authored. G027 restricts capital vertical movement to limited ship-to-ship collision avoidance, while small craft retain full 3D. S044/S045 select desired speed and heading. Do not reopen full-3D capital combat or add automatic position holding here.

Lateral thrust remains a separate choice: it can be a low-speed alignment aid or a tactical capability where fitted, permitting sideways movement without changing facing. Real strength, acceleration, reserves and hull differences must preserve the value of approach angles and main drives.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Hull-authored lateral capability and G027-limited capital vertical avoidance. Decide whether lateral thrust can be used tactically or only for close alignment. |
| Ongoing activity | Maintain useful geometry with actual movement limits while listening to weapon, shield and operation needs. |
| Active decisions | Translate for clearance, alignment, an exposed component angle or avoiding a threat, where equipment supports it. |
| Complexity and AI | Simple presets can delegate manoeuvring execution; deeper controls expose supported lateral choices. No different physics by depth or requirement that every hull have powerful lateral thrust. |
| Interactions and communication | Weapons/defence request useful aspects, external-operation operators report clearance and Engineering supplies energy/cooling. Helm retains actuator authority. |
| Risks | Every capital becomes an omnidirectional fighter, lateral movement eliminates broadside positioning, low-depth controls become overcrowded or vertical avoidance becomes unrestricted tactical flight. |
| Dependencies | G027/G032, S007/S009/S016/S044/S045 and S047–S049. Exact lateral interface, strength/speed limits, vertical avoidance and assistance remain open. |

### Resolved choice: can fitted lateral thrusters support tactical strafing?

| Option | Form | Trade-off |
|---|---|---|
| A — Close alignment only | Use lateral thrust for low-speed docking/operation alignment, without a tactical strafing role. | Keeps capital combat centred on heading and main propulsion. |
| B — Tactical use where fitted | Suitable hulls can translate sideways during combat within actual thruster/resource limits. | Adds equipment-dependent positioning choices, but needs handling differences that preserve capital-ship character. |

**Selected: B, with visible direct controls.** Suitable hulls support tactical lateral thrust within actual thruster/resource limits; G027's capital vertical restriction remains. Provide a HUD toggle, not a hidden settings option, for direct steering, thrust and lateral control. S044/S045's desired-speed/heading assistance remains the default; both interfaces use identical actuators, physics, damage and resources. This is an operating input mode, not a per-System complexity preset override. Exact interface, ratings, mode handoff and Match Speed interaction remain open.

## S047 — Boost / impulse burst

**Inclusion recommendation: retain equipment-dependent tactical boost and resolve how a burst ends.** [The Current movement baseline](../gdd/mechanics/movement.md#impulse-and-boost) separates short tactical boost from charged high-speed impulse transit. S037 selects equipment-supported overdrive with energy, heat and stress costs; S041 excludes ordinary manoeuvre propellant. Boost should use these shared constraints rather than automatically introducing another fuel or abstract charge meter.

The key operating choice is whether Helm controls the duration within resource limits or activation commits a fixed-duration burst followed by recovery. Ending thrust does not erase acquired momentum; acceleration/braking remain shared physics. Long-distance travel and its fuel are distinct from this tactical capability.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional authored tactical boost, respecting fitted propulsion capability. Decide whether duration is operator-controlled or a committed equipment-defined burst. |
| Ongoing activity | Maintain propulsion reserve/thermal margin and observe actual remaining capability; ordinary AI handles delegated use. |
| Active decisions | Spend reserve to close, escape, avoid a threat or support recovery; stop thrust or commit the whole burst according to the selected model. |
| Complexity and AI | Simple presets delegate boost policy; deeper/manual Helm controls expose meaningful use without repeated cooldown-clicking. Shared resource/physics rules apply to direct and assisted inputs. |
| Interactions and communication | Helm communicates a burst; Weapons/Shield operators adapt geometry and Engineering supports energy/cooling. Ordinary AI follows intent and risk limits without hidden future threats. |
| Risks | Another always-press-when-ready button, boost instantly changes velocity, resource costs duplicate fuel, or every hull has identical escape capability. |
| Dependencies | S007/S031/S037/S041/S044–S046 and later transit review. Exact boost strength, limits, recovery, steering compatibility and stacking remain open. |

### Resolved choice: who determines tactical boost duration?

| Option | Form | Trade-off |
|---|---|---|
| A — Operator-controlled | Sustain/stop boost within actual reserve, thermal and equipment limits. | Makes reserve spending flexible and links use to the shared operating model. |
| B — Committed burst | Activation commits an authored duration, with equipment recovery before another burst. | Creates a clear timing commitment, but offers less control over resource spending. |

**Selected: A.** Helm sustains/stops tactical boost within actual reserve, thermal and equipment limits. No universal fixed-duration burst is required; ending extra thrust does not erase momentum. Ordinary individual propulsion AI follows normal intent/risk limits when delegated. Specific preparation/recovery can remain equipment-authored. Exact strength, costs, limits, steering compatibility and transit interaction remain open.

## S048 — Relative-motion assist / match, hold, intercept

**Inclusion recommendation: retain G032's Match Speed, and resolve stale observations without adding automatic relative geometry.** [The broader Plan C3 baseline](../gdd/mechanics/movement.md) includes relative-motion intentions; the redesign specifically selects matching the selected contact's speed until throttle input or another toggle click. Steering remains Helm's responsibility. No automatic position/range/aspect hold or intercept is selected by this entry.

S044/S045 supply desired speed/heading assistance; G019 preserves observed motion and uncertain lost tracks without following unseen manoeuvres. Match Speed must therefore use available contact evidence rather than hidden live velocity. Decide whether missing fresh updates leaves the last observed speed as a flagged setpoint or cancels the matching intent.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Persistent speed matching as already selected, with explicit stale/infeasible status. Resolve behaviour when observations stop; do not re-ask automatic steering. |
| Ongoing activity | Helm chooses direction and watches actual geometry/capability while assistance maintains the supported speed setpoint. |
| Active decisions | Enable/cancel matching, use the throttle, respond to stale information or choose another approach when matching becomes infeasible. |
| Complexity and AI | Assistance uses ordinary available observations and real actuators. Direct controls remain accessible through the S046 HUD toggle; exact mode-switch handoff is not selected here. |
| Interactions and communication | Sensors/Navigation supplies available contact evidence, relevant operators report geometry needs and Engineering explains actual propulsion limits. |
| Risks | Hidden perfect tracking through concealment, sudden unexplained braking on contact loss, matching interpreted as formation/range hold or constant reactivation during brief missed updates. |
| Dependencies | G018/G019/G027/G032, S026/S038/S044–S047 and later sensing review. Exact stale thresholds, track expiry, reacquisition and infeasibility presentation remain open. |

### Resolved choice: what happens when Match Speed loses fresh contact updates?

| Option | Form | Trade-off |
|---|---|---|
| A — Hold last observed speed | Keep the last evidence-based speed setpoint with clear stale status; resume updates when the same contact is observed again. | Avoids unnecessary toggling, but users must understand they are no longer following live changes. |
| B — Cancel matching | End the matching intent and retain the last speed setpoint as ordinary throttle intent; require reactivation after reacquisition. | Makes loss of live matching explicit, but adds another action after temporary observation gaps. |

**Selected: A.** Hold the last observed speed with clear stale feedback; resume updates when the same contact is observed again. Never follow unseen manoeuvres. Throttle input or another toggle click still cancels matching and Helm chooses heading. This resolves missing updates only, not automatic contact substitution or indefinite track retention. Exact expiry and reacquisition rules remain open.

## S049 — Docking / capture and mating apparatus

**Inclusion recommendation: retain physical docking as part of crew-owned external operations.** [The Current baseline](../gdd/mechanics/external-operations.md) already gives docking its own System, power, damage, authority and ordinary Backfill. Helm docks and Engineering operates the separate umbilical; actual resource transfers determine outcomes. Station servicing remains a distinct planned capability. Whole-System AI can handle approach and mating using normal knowledge and physical constraints; manual approach remains available.

G061 already distinguishes recovering deployed personnel/equipment from simply releasing a tractor. G067 selects routine allied-port replenishment and manually initiated trade where required. Neither resolves the mechanical commitment of a physical dock: emergency departure could be an immediate release or require equipment-dependent separation. The proposal should make that commitment readable without making every routine dock a checklist.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Compatible physical docking/capture equipment and readable approach, mating, connected and separation states. Services and transfers remain separate Systems. |
| Ongoing activity | Maintain viable geometry, power and access during external work; routine monitoring can be delegated. |
| Active decisions | Choose an approach, establish a connection, coordinate work and decide when to depart or abandon an operation. |
| Complexity and AI | Whole-System ordinary AI handles delegated docking. Human control exposes consequential approach/departure decisions rather than compulsory latch-by-latch actions. |
| Interactions and communication | Helm communicates departure intent; transfer/team operators report unfinished work and recovery needs; Engineering supplies actual machinery limits. No central controller silently completes other Systems' work. |
| Risks | Instant departure erases physical commitment; long mandatory rituals create waiting; breakaway invents arbitrary damage or silently abandons personnel. |
| Dependencies | G032/G061/G065/G067/G070, S028/S039/S043–S048 and later transfer/team review. Exact compatibility, capture geometry, separation times and emergency hardware remain open. |

### Resolved choice: how should physical undocking work under pressure?

| Option | Form | Trade-off |
|---|---|---|
| A — Immediate release | The docking connection releases immediately when requested, subject to actual machinery availability. | Keeps departure simple and puts the main commitment in unfinished transfers and deployed teams. |
| B — Equipment-dependent separation | Normal separation takes a short, visible period determined by equipment/state; suitable emergency-release hardware can shorten it with explicit consequences. | Makes docking a positioning and protection commitment, but needs brief, readable execution and competent AI. |

**Selected: B.** Normal separation takes a short, visible period determined by equipment/state; suitable emergency-release hardware can shorten it with explicit consequences. Ordinary docking AI executes delegated separation, while players coordinate departure, unfinished transfers and personnel recovery. No universal damage penalty, compulsory manual checklist or new authority over other Systems is selected. Exact compatibility, timing and emergency mechanisms remain open.

## S050 — Warp drive / inter-system transit

**Inclusion recommendation: retain long-distance travel, and choose its spatial form before designing its operating work.** [Plan C6](../gdd/mechanics/navigation-relative-motion.md#plan--c3-relativephysical-space-c6c8-transitorbital) specifies discrete inter-system warp and normally abundant but finite warp fuel. [R1 item 11](./research/feature-gap-analysis-master.md) instead proposes a continuous warp-factor speed control with power draw. These are different candidate designs, not two descriptions of the Current implementation. Current charged impulse transit and S047 tactical boost are separate capabilities.

S041 already selects long-distance fuel while excluding ordinary manoeuvre propellant. G053 selects equipment-specific interdiction with counterplay, not a universal combat departure lock. The next decision is whether warp connects distinct local spaces or extends continuous movement; exact preparation, travel duration, arrival placement and opportunities for activity follow that choice.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Compare discrete inter-system transitions with continuous high-speed travel through connected space. Keep local movement and tactical displacement distinct. |
| Ongoing activity | Prepare known fuel, energy, thermal and equipment margins. Whether transit itself contains operating work remains unresolved. |
| Active decisions | Choose destination and departure timing, assess known route/arrival risks and respond to actual interdiction or equipment limits. |
| Complexity and AI | Ordinary individual drive AI can execute supported travel intent and preparations using available evidence. It cannot bypass other Systems' authority or acquire hidden destination information. |
| Interactions and communication | Navigation contributes route/arrival evidence; Helm handles departure geometry; Engineering reports readiness; mission and team operators communicate unfinished commitments. Final station composition remains open. |
| Risks | Long empty travel, repetitive departure chores, strategic travel replacing local manoeuvre, omniscient arrival knowledge or departures instantly erasing pursuit without physical counterplay. |
| Dependencies | G019/G053/G082/G086/G090, S031/S041/S044–S049 and S051–S053. Fuel costs, preparation, travel duration, arrival geometry and interruption rules remain open. Networking and individual-System AI architecture remain unchanged. |

### Resolved choice: what form should inter-system warp take?

| Option | Form | Trade-off |
|---|---|---|
| A — Discrete inter-system transit | Prepare and commit a transition between local spaces, following the existing C6 direction. | Concentrates play around departure, route choice and arrival; transit duration/activity still needs design. |
| B — Continuous high-speed travel | Operate warp speed and course through connected space, following the R1 warp-factor candidate. | Makes travel an ongoing movement activity, but needs meaningful space between destinations and careful coexistence with impulse. |

**Selected: A.** Inter-system warp uses discrete transitions between local spaces, following C6's spatial model. S041's long-distance fuel remains selected. Current impulse transit and S047 tactical boost remain distinct. This does not select instantaneous arrival or a mandatory idle wait; preparation, duration/activity, arrival geometry and interruption rules remain open.

## S051 — Jump drive / tactical displacement

**Inclusion recommendation: consider a separate local jump only as optional equipment with a distinct tactical purpose.** [R1 item 21](./research/feature-gap-analysis-master.md) actually describes a charged **strategic FTL** jump, with Engineering affecting preparation. It does not establish a separate local tactical teleport. The catalogue's tactical interpretation is therefore an adaptation to evaluate, not a researched game's confirmed mechanic or existing Plan. S050 already supplies discrete inter-system travel.

A local jump could create unusual approach, escape or rescue geometry, but also bypass broadside positioning, projectile travel and physical pursuit. Decide whether it earns a separate place in the redesign before specifying destination constraints or costs. No new networking or AI architecture is needed: an adopted drive would be an ordinary System subject to the same authority, knowledge and physical resource rules.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Compare leaving jumps within S050 inter-system travel with adding optional fitted short-range displacement within a local space. No universal jump drive is proposed. |
| Ongoing activity | If adopted, maintain real readiness and resource margins through ordinary System operation, without a recurring charge-click chore. |
| Active decisions | Choose destination geometry and timing for a meaningful manoeuvre, accepting known preparation and arrival constraints. |
| Complexity and AI | Whole-System ordinary AI operates delegated capability using available observations and supported orders; manual operation exposes meaningful choices rather than required countdown babysitting. |
| Interactions and communication | Navigation/Sensors contributes destination evidence; weapon and defence operators explain aspect needs; Engineering supplies readiness. Observable preparation can give opponents earned warning under G113. |
| Risks | Teleportation invalidates positioning, becomes a universal escape button, reveals hidden safe destinations or adds another drive with no distinct player decision. |
| Dependencies | G019/G027/G053/G105/G113, S007/S016/S031/S041/S044–S050. Inclusion comes first; exact range, preparation, costs, arrival constraints, momentum and counterplay remain open. |

### Resolved choice: should local tactical jumping be a separate capability?

| Option | Form | Trade-off |
|---|---|---|
| A — Keep jumps inter-system | Use S050 for transitions; local positioning uses propulsion, boost and terrain. | Preserves the value of manoeuvre with fewer overlapping drive Systems. |
| B — Optional local jump equipment | Some hulls/loadouts can make a prepared short-range displacement, with meaningful constraints and readable counterplay. | Adds distinctive ship behaviour, but needs limits that preserve ordinary positioning and pursuit. |

**Selected: B.** Some hulls/loadouts support prepared short-range local displacement as optional equipment, with meaningful constraints and readable counterplay. This remains separate from S050 inter-system transit. Whole-System ordinary AI can operate delegated use under normal knowledge and authority. Destination requirements, preparation, costs, momentum, arrival and interruption rules remain open.

### Resolved S051 follow-up — Does a local jump require a destination anchor?

The inclusion choice leaves two different activities possible: selecting a spatial destination from available navigation/sensor evidence, or establishing a receiving beacon/anchor before jumping. The latter creates deployment and protection work but restricts spontaneous positioning and escape. This is a proposal choice, not a mechanic established by R1's strategic-jump description.

G019's uncertain observations still apply: a coordinate selection cannot reveal whether an unseen ship occupies the destination. Neither human nor AI gains hidden clearance information from the jump interface. A beacon could support navigation without guaranteeing an empty arrival volume. Exact obstruction handling and arrival accuracy require a later decision rather than an implicit promise of safety.

| Option | Form | Trade-off |
|---|---|---|
| A — Select a spatial destination | Choose a point within equipment limits using available information; no receiving anchor is universally required. Beacons may assist particular equipment. | Supports flexible tactical manoeuvres and makes destination evidence valuable, but arrival uncertainty must be legible. |
| B — Require a destination anchor | Jump only to a compatible established beacon/anchor. | Creates deployment, protection and denial objectives, but confines the capability to prepared locations. |

**Selected: A, with a shared drive-profile arrival model.** Select a spatial destination without a universal receiving-anchor requirement. Local versus inter-system capability depends on two equipment properties: jump accuracy and sensitivity to gravity wells. Resolve arrival in this order:

1. Randomise a point within an accuracy radius around the intended destination.
2. Push that point outward from large masses along their gravity wells, according to the drive's gravity sensitivity.

Inter-system drives have poor accuracy and strong gravity displacement; local drives handle gravity and/or accuracy better. This is not a complexity-dependent change in physics or a requirement for every ship to carry both capabilities. It supplies common arrival principles without selecting identical range, fuel or preparation rules for all equipment. Normal limited knowledge remains: destination selection does not reveal hidden clearance information.

Navigation/Sensors can contribute known mass locations and arrival estimates; Engineering reports fitted drive capability and readiness. Exact scatter distribution/radius, gravity strength/falloff, combination of multiple masses, preview uncertainty and obstruction handling remain open. Static orbital bodies under G031 do not acquire orbital movement from this jump calculation.

### Resolved S051 follow-up — What happens to motion across a local jump?

The arrival model resolves position but not velocity. S044–S047 retain real acceleration, braking and momentum; a jump that automatically stops the ship would also become a braking capability. Decide whether that is intended before assuming a drive's arrival behaviour. This question concerns local jumps; mapping velocity between inter-system reference frames remains separate.

| Option | Form | Trade-off |
|---|---|---|
| A — Preserve velocity | Arrive with the same local-space velocity vector; Helm must plan braking and the next manoeuvre. | Preserves movement commitments and creates approach/exit coordination, but needs a readable projected arrival motion. |
| B — Reset velocity | Arrive at rest in the local navigation frame. | Simplifies arrival control, but makes jumping a powerful emergency brake as well as displacement. |

**Selected: B.** Local jumps reset velocity: the ship arrives stationary in the local navigation frame. Jumping therefore also provides emergency braking, governed by the drive's actual preparation, resource and operating limits. Arrival position still follows the selected scatter-then-gravity-displacement model; being stationary does not guarantee clearance or safety. Heading changes and inter-system velocity mapping remain unresolved.

## S052 — Navigation chart / waypoint planner

**Inclusion recommendation: retain Navigation's chart and shared destination, and resolve route-planning scope.** [The Current baseline](../gdd/mechanics/navigation-relative-motion.md) has a world-anchored chart and one authoritative shared waypoint, either fixed or entity-anchored. Browsing a contact does not issue commands. Navigation proposes a destination; Helm decides how to fly, and ordinary AI Helm follows permitted navigation intent. This authority boundary remains useful under the redesign.

Terrain, traffic, uncertain contacts and the selected jump gravity model can make approach planning meaningful. The question is whether Navigation communicates a single destination plus spoken advice, or can also publish a multi-leg route. A route must remain advice for human Helm rather than automatically taking steering authority. Moving anchors must use available evidence under G019, not silently expose unseen live movement from the Current baseline's entity-following mechanism.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain a readable chart, local browsing and shared destination. Compare one active waypoint with optional multi-leg route planning. |
| Ongoing activity | Watch changing traffic, known hazards, contact uncertainty and whether the intended approach remains useful. |
| Active decisions | Propose or revise a destination/approach, explain a diversion and coordinate arrival or jump positioning. |
| Complexity and AI | Simple Station presets can expose the next destination; deeper presets can expose route planning if adopted. Ordinary Navigation AI proposes supported intent; individual propulsion AI executes only its own permitted work. |
| Interactions and communication | Navigation explains route reasons and uncertainty; Helm reports handling/geometry needs; Sensors supplies evidence; Comms supplies access/traffic context. Captain has no personal map and can receive deliberate Viewscreen sharing. |
| Risks | Route editing becomes busywork, plotting is compulsory for simple movement, navigation silently takes Helm control or an anchored waypoint grants perfect hidden tracking. |
| Dependencies | G016/G018/G019/G022/G027/G031/G032, S044–S051 and S053. Exact route interface, leg advancement and moving-anchor loss handling remain open. This does not add player-authored mission-log annotations rejected by G026. |

### Resolved choice: should Navigation support a multi-leg route?

| Option | Form | Trade-off |
|---|---|---|
| A — One shared destination | Retain one active waypoint; Navigation explains intermediate approach advice through communication. | Keeps plotting simple, but changing a planned approach requires repeated waypoint updates. |
| B — Optional multi-leg route | Publish a sequence of waypoints with the current leg clear; a single destination remains sufficient for simple travel. | Makes terrain/approach planning tangible, but needs clear revision and leg handling without compulsory route editing. |

**Selected: B.** Navigation can publish an optional multi-leg route with the current leg clear; one destination remains sufficient for simple travel. Human Helm retains steering decisions. Publishing a route does not automatically engage autopilot or move authority between Systems. Ordinary AI uses normal knowledge, orders and individual-System authority. Exact editing, leg advancement and moving-anchor loss handling remain open.

## S053 — Autopilot and formation / escort controller

**Inclusion recommendation: retain ordinary delegated movement, and resolve the scope of coordinated formations.** [The Current baseline](../gdd/mechanics/navigation-relative-motion.md#helm-boundary) already permits AI Helm to pursue a shared waypoint under supported orders. [Current external operations](../gdd/mechanics/external-operations.md) also include a target-authored tractor formation response; that is not a general fleet formation controller. S052 adds optional multi-leg routes without automatically engaging AI control.

Normal individual movement AI remains competent within available knowledge and real actuators. This entry does not create a separate temporary autopilot behaviour or central fleet mind. The unresolved addition is whether escorts merely pursue an objective near a protected actor, or can receive explicit relative formation slots. Human Helm still chooses its own movement unless it delegates the relevant System; G032's Match Speed itself acquires no automatic position or aspect hold.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Retain supported route following and escort intent. Compare loose escort with optional explicit relative-position formation orders. |
| Ongoing activity | Delegated AI adjusts movement within knowledge, capability and normal risk/mission context; human operators monitor relevant geometry rather than babysitting steering corrections. |
| Active decisions | Choose an escort/formation role, communicate an intended manoeuvre, revise the order or break away for a competing objective. |
| Complexity and AI | Each ship's ordinary individual movement AI executes its own orders through normal authority. Fleet members receive requests through supported Comms; no remote direct actuator control or hidden shared perception. |
| Interactions and communication | Leaders communicate manoeuvres; escorts report infeasible geometry or competing tasks. Weapons/defence explain useful arcs and cover, Navigation supplies route evidence and Engineering reports movement limits. |
| Risks | Perfect rigid formations ignore obstacles/damage, orders grant omniscient leader tracking, constant correction becomes busywork or formation assistance removes human Helm decisions without delegation. |
| Dependencies | G019/G022/G024/G032/G039/G044/G114, S007/S044–S052. Exact slot representation, safe deviation, order acceptance, observation loss and breakaway policy remain open. Normal AI context/hysteresis applies. |

### Resolved choice: should coordinated movement support explicit formation slots?

| Option | Form | Trade-off |
|---|---|---|
| A — Loose escort intent | Order ships to escort/protect an actor; each decides useful positioning without an assigned relative slot. | Fewer formation controls and more independent positioning, but less deliberate fleet geometry. |
| B — Optional formation slots | Also support relative-position assignments that ordinary movement AI tries to maintain within real limits. | Enables coordinated cover and approach geometry, but slots must remain breakable/infeasible rather than rigid constraints. |

**Selected: B, with formation jumping.** Support optional explicit relative-position slots, maintained where feasible by each ship's ordinary movement AI when delegated. Slots do not force human Helm to relinquish control or promise perfect station-keeping. Match Speed alone gains no position/aspect hold. Ships can also jump in formation **using the best participating drive stats**. Coordinate through normal communication and individual-System authority; no central fleet mind, hidden shared perception or remote direct actuator control is introduced.

Formation jumping interacts with S051's accuracy/gravity model and gives mixed drive capabilities a useful fleet role. Exact eligibility, combination of best stats across drives, shared arrival displacement, readiness, membership bounds and interruption behaviour remain open. Using best stats does not itself settle whether members need their own operating drives or whether one drive can carry other hulls.

### Resolved S053 follow-up — Does every formation-jump member need its own working drive?

| Option | Form | Trade-off |
|---|---|---|
| A — Each ship participates with a drive | Every member needs an operating drive capable of the jump type; formation coordination improves the effective stats using the best participating drives. | Preserves individual machinery/readiness and makes protecting a disabled ship meaningful. |
| B — A drive can carry other ships | Suitable formation-jump equipment can transport members without operating drives, within explicit capacity/range limits. | Adds jump-tender and evacuation roles, but requires carrying limits and rules for disabled/drive-less members. |

**Selected: A.** Every member needs its own working drive capable of the jump type. Formation coordination uses the best participating drive stats but does not carry ships without operating drives. A separate carrying capability is not adopted by this choice. Exact effective-stat combination, membership bounds, shared arrival displacement and readiness-loss handling remain open.

### Resolved S053 follow-up — Is arrival error shared across the formation?

S051 resolves arrival by scattering around the intended point, then pushing outward along gravity wells. For a formation this could produce one displacement applied to the whole group, or separate displacement for each ship using the same best drive stats. This choice determines whether the jump preserves relative positions or creates regrouping work. Neither option promises obstacle clearance or reveals hidden occupants.

| Option | Form | Trade-off |
|---|---|---|
| A — Shared displacement | Resolve scatter and gravity displacement for a formation reference point, then apply the same translation to participating ships, preserving their relative offsets. | Makes formation jumping a coherent coordinated arrival, while the whole group's destination remains uncertain. |
| B — Independent displacement | Resolve each member's arrival separately using the formation's effective drive stats. | Can disperse the group and create recovery decisions, but weakens planned arrival geometry. |

**Selected: A.** Resolve scatter and gravity displacement once for a formation reference point, then translate participating ships together, preserving their relative positions. The whole group can arrive off target. This does not snap members into perfect slots, guarantee clearance or reveal hidden occupants. Exact formation extent, reference point/gravity evaluation, obstruction handling, effective-stat combination and departure membership/readiness rules remain open.

## S054 — Helm radar

**Inclusion recommendation: retain a movement-oriented view of available contact evidence; resolve its combat warning scope.** [The Current movement baseline](../gdd/mechanics/movement.md) makes Helm responsible for motion while Navigation and Tactical provide advice. G016 already selects basic detected returns on relevant operational radars, G018/G019 retain uncertainty and G025 leaves danger discovery with ordinary player tools. This entry is a presentation/use of available observations, not automatic deep identification or a new omniscient sensor.

Helm needs local clearance and relative motion to act. An additional choice is whether its view also projects observed incoming ordnance as possible intercepts with the ship's motion. This could make evasive manoeuvres a direct activity without requiring Tactical to narrate every projectile, while leaving prioritisation, interception capacity and deeper interpretation with the relevant operators. Prediction must use observed motion and uncertainty, never hidden guidance changes or unrevealed enemy intent.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Local movement/clearance radar using available returns, known terrain and readable uncertainty. Compare proximity warnings alone with optional observed-ordnance motion warnings. |
| Ongoing activity | Watch closing geometry, actual handling and crew requests; automatic tracking avoids maintaining every return manually. |
| Active decisions | Turn, brake, translate or accept exposure to protect an operation or keep useful weapon/shield geometry. |
| Complexity and AI | Authored Station presets simplify display/interpretation; ordinary individual movement AI uses the same available evidence. No automatic evasion takes authority from human Helm. |
| Interactions and communication | Tactical reports interception priorities/capacity, Sensors reports uncertain threats, Engineering reports handling margins and Helm communicates intended manoeuvres. Warnings do not replace those facts or Captain coordination. |
| Risks | Screen clutter, perfect dodge instructions, duplicating deep Tactical/Sensors functions or requiring spoken narration of every obvious local collision threat. |
| Dependencies | G016/G018/G019/G025/G027/G105/G113, S003/S008/S044–S053 and later sensor entries. Exact prediction horizon, warning thresholds, filters and uncertainty presentation remain open. |

### Resolved choice: should Helm see predicted incoming-ordnance danger?

| Option | Form | Trade-off |
|---|---|---|
| A — Movement hazards only | Show clearance, terrain and relative contact motion; Tactical communicates projectile danger. | Keeps the display focused, but makes immediate evasion more dependent on relayed warnings. |
| B — Also observed ordnance predictions | Show possible intersections from observed projectile tracks and own motion, with uncertainty. | Gives Helm direct evasive decisions, but requires filtering and must not disclose unseen guidance or exact future hits. |

**Selected: B.** Helm radar shows possible intersections from observed projectile tracks and own motion, with uncertainty. Predictions do not reveal unseen guidance changes or exact future hits. Helm chooses evasive motion; Tactical communicates interception capacity and priorities. Ordinary movement AI uses the same evidence and no automatic evasion overrides human control. Exact horizon, warning thresholds and filters remain open.

## S055 — Tactical radar / Combat Lock

**Inclusion recommendation: retain Tactical's engagement picture and primary offensive target, with G048's separate weapon-System assignments.** [The Current baseline](../gdd/mechanics/targeting-weapons.md) has one ship Combat Lock and a separate advisory Science Target. The redesign already extends this through G048's individual weapon-System assignments, S015's prepared groups and individual overrides, and G105's equipment-specific acquisition/support. Do not reintroduce a universal lock requirement or imply the Current single-lock restriction still defines the proposal.

The remaining policy choice is how delegated offensive weapons without an explicit individual assignment use the primary target. They could engage only that target, or prefer it while normal doctrine permits suitable secondary targets. Point defence already has its interception role under S008; this question concerns offensive allocation. Explicit assignments/overrides remain authoritative and group changes do not overwrite an active individual override.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Readable primary target, group/System assignments and actual acquisition/readiness reasons. Decide whether unassigned delegated offensive weapons can engage secondary targets under doctrine. |
| Ongoing activity | Watch engagement geometry, ammunition/reserves, contact confidence and what delegated weapons are actually engaging. |
| Active decisions | Change primary target, concentrate or split fire through supported assignments, prioritise support and coordinate angles or resource commitments. |
| Complexity and AI | Simple presets support primary-target play; deeper presets expose prepared groups and System overrides. Each weapon's ordinary AI uses its own knowledge, equipment limits and permitted targeting policy. |
| Interactions and communication | Sensors supplies findings/designations; Tactical explains desired angles and priorities to Helm; Engineering supplies resources; defensive operators report competing interception demand. No station obtains another's authority by selecting a contact. |
| Risks | Constant target administration, weapons silently contradicting concentration orders, target thrashing, hidden perfect acquisition or a primary target that has no meaningful effect. |
| Dependencies | G018/G019/G048/G049/G105/G107/G114, S003/S008/S015/S016/S054 and S056 onward. Exact doctrine controls, ranking, acquisition capacity and override rejoining remain open. |

### Resolved choice: how do unassigned delegated offensive weapons use the primary target?

| Option | Form | Trade-off |
|---|---|---|
| A — Primary target only | Engage the primary target; attacking another requires an explicit supported assignment. | Makes concentration predictable, but may leave weapons idle when another target is viable. |
| B — Primary preference with permitted alternatives | Prefer the primary target; doctrine can permit secondary engagements when appropriate. Explicit assignments still win. | Makes delegated weapons more adaptable, but needs visible intent and a way to order strict concentration. |

**Selected: B.** Unassigned delegated offensive weapons prefer the primary target, with suitable secondary engagements permitted by doctrine. Explicit individual assignments take precedence. Support a strict concentration policy, hysteresis and visible current targets; neither alternative targeting nor group/primary-target changes erase active individual overrides. Point defence retains its separate interception role. Exact doctrine controls, ranking and acquisition capacity remain open.

## S056 — Sensor radar / Science Target

**Inclusion recommendation: retain Sensors' investigation view and advisory designation, and resolve how players express investigation intent.** [The Current baseline](../gdd/mechanics/sensors-epistemics.md) separates radar browsing, a Science Target, scan readings and evidence. A Science designation advises Tactical without replacing its Combat Lock. G017 already selects automatic routine broad assessment with optional deeper enquiry; G020 selects separately powered broad and focused Systems operating concurrently, superseding the baseline Plan's single wide/focused mode trade-off.

This entry concerns choosing what to investigate, not the focused scanner's later hardware/capacity review. The player could select only an entity and receive a general assessment, or also choose a supported question such as identifying an unknown contact, locating an engine or assessing a suspected cargo discrepancy. Ordinary System AI could execute the relevant observation work without a manual scan-click sequence. Questions are requests for evidence, not guaranteed answers or access to hidden truth.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Sensor-oriented radar, contact/evidence inspection and advisory designation. Compare entity-only targeting with optional supported investigation aims. |
| Ongoing activity | Watch changing observations and contradictions; broad assessment continues independently of focused work. |
| Active decisions | Decide which uncertainty matters, prioritise an enquiry, assess the resulting evidence and communicate implications. |
| Complexity and AI | Simple presets can select a subject for general assessment; deeper presets may expose supported aims if adopted. Ordinary individual scanner AI operates whole Systems within knowledge, equipment and authority. |
| Interactions and communication | Helm/Navigation requests route evidence; Tactical asks about capabilities or useful attack angles; Comms supplies claims to check. Sensors reports findings/confidence rather than silently deciding other operators' actions. |
| Risks | Repeated scan-click chores, mandatory investigation menus for routine contacts, questions becoming omniscient queries or the Science Target overriding weapon assignments. |
| Dependencies | G017–G020/G023/G048/G106/G109, S016/S050–S055 and S057 onward. Exact enquiry vocabulary, scheduling, scanner concurrency and result presentation remain open. |

### Resolved choice: should Sensors select an investigation aim as well as a subject?

| Option | Form | Trade-off |
|---|---|---|
| A — Subject only | Select the contact; supported scanning produces its general available assessment. | Simple operation, but weaker control over which question gets attention first. |
| B — Optional investigation aim | Also choose a supported question/priority; general assessment remains the simple default. | Gives Sensors purposeful choices and crew requests, but needs a concise set of equipment-relevant aims. |

**Selected: B.** Sensors can choose an optional supported investigation aim as well as a subject, with general assessment retained as the simple default. Ordinary individual scanner AI executes supported observation work. Aims are equipment-dependent requests for evidence, not guaranteed answers; insufficient evidence or capability can leave an enquiry unresolved. Exact aim vocabulary, scheduling and presentation remain open.

## S057 — Wide survey / search scanner

**Inclusion recommendation: retain independent automatic broad assessment, and resolve whether ordinary contact count consumes assessment capacity.** G017 selects routine assessment without contact-by-contact processing in fleet battles; G020 supplies a separately powered broad System that focused scanning does not directly degrade. [The baseline knowledge ladder](../gdd/mechanics/sensors-epistemics.md#plan--c2c5-knowledge-ladder) provides detection through tactical assessment. Available information remains observation-based, with obfuscation and sensing limits; broad assessment does not expose protected internals.

[R6's sensor research](./research/comparable-games-and-falling-frontier.md) supplies ideas about exposure, coverage and uncertainty, but does not settle Phoenix's per-contact capacity rule. Directional search and passive reception have their own later entries, S098 and S060. Here the question is whether adding ordinary unobfuscated contacts slows broad assessment through a shared update budget, or whether each contact's quality is governed by observation conditions without a count-based budget.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Automatically maintain broad observations within actual coverage, power and interference limits. Decide whether contact count alone spreads a finite assessment/update budget. |
| Ongoing activity | Watch consequential changes, uncertainty and blind areas; routine contact assessment runs without repeated selection. |
| Active decisions | Respond to changed coverage/quality, request resources or direct the independent focused System toward an important uncertainty. |
| Complexity and AI | Ordinary broad-System AI performs routine assessment. If a finite update budget is adopted, allocation must also be automated rather than requiring a queue of manual scans. |
| Interactions and communication | Sensors explains what is known and what remains uncertain; Engineering supports separate sensor demands; Helm/Navigation can change geometry. |
| Risks | Fleet battles become contact maintenance, harmless traffic unintentionally blinds a ship, or automatic assessment is mistaken for unlimited range/perfect truth. |
| Dependencies | G017–G021/G106/G109, S056/S058/S060/S098. Exact range, fidelity, update cadence, clutter/interference and power curves remain open. No numerical capacity is selected here. |

### Resolved choice: should ordinary contact count dilute broad assessment?

| Option | Form | Trade-off |
|---|---|---|
| A — No count-based assessment budget | Assess contacts automatically according to observation conditions; more ordinary contacts alone do not dilute each one's update allocation. | Preserves fleet awareness without track administration; concealment, range, interference and equipment still matter. |
| B — Finite shared update budget | More contacts spread broad assessment capacity; ordinary AI prioritises updates and shows resulting age/uncertainty. | Creates sensor saturation and prioritisation choices, but needs safeguards against fleet-scale stale-data overload. |

**Selected: A.** Broad assessment has no contact-count-based update budget. Ordinary contact count alone does not dilute each contact's assessment; actual coverage, range, equipment, power, concealment, clutter and interference can still limit observations. Routine assessment is automatic, with no per-contact scan administration. Focused sensing remains independent and separately powered. Exact fidelity, update cadence and sensing curves remain open.

## S058 — Focused scanner / structural scan

**Inclusion recommendation: retain focused enquiries and resolve concurrency per fitted scanner.** [The Current baseline](../gdd/mechanics/sensors-epistemics.md) supports infrastructure readings whose fidelity depends on observation conditions. The redesign adds independent focused sensing under G020, optional shield probing under G050, scanned equipment compartment membership and attack-angle advice under S016/AD48, and optional investigation aims under S056. Broad assessment continues under S057; focused work does not take its capacity.

Focused work should make choosing what to learn consequential. A fitted scanner could pursue one active enquiry, with ordinary AI handling a priority queue, or divide its finite capability among simultaneous enquiries. This is equipment concurrency, not a per-player change in physics. Retained findings remain evidence with provenance/age; changing attention does not erase them or guarantee that old conditions remain current. Intrusive permission/exposure is reviewed separately in S059.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Equipment-supported deeper enquiries, including structural/condition information and optional weakness probing. Decide one active enquiry per scanner versus shared concurrent capacity. |
| Ongoing activity | Maintain useful observation geometry and resources; watch whether findings still support current decisions. Ordinary AI handles observation execution. |
| Active decisions | Prioritise or interrupt an enquiry in response to a crew request, choose the useful subject/question and communicate confidence or remaining uncertainty. |
| Complexity and AI | Simple presets can delegate enquiry scheduling; deeper presets expose priorities and supported aims. Same fitted scanner limits apply to all controllers. |
| Interactions and communication | Tactical requests component/weakness evidence, Helm supports geometry, Engineering supplies separate power/cooling and other operators request mission-relevant findings. |
| Risks | Too many simultaneous perfect answers, constant target switching, progress loss that rewards babysitting or a queue that blocks urgent work without explanation. |
| Dependencies | G018–G020/G050/G109/G114, S016/S056/S057/S059. Exact enquiry duration/progress retention, geometry, hardware count, interruption and exposure remain open. No universal deep scan is required to fire weapons. |

### Resolved choice: how many enquiries can one focused scanner pursue at once?

| Option | Form | Trade-off |
|---|---|---|
| A — One active enquiry per fitted scanner | Ordinary AI works a prioritised queue; multiple separately fitted scanners can work concurrently where supported. | Creates clear attention and equipment choices without continuous capacity sliders. |
| B — Shared concurrent capacity | A scanner can pursue multiple enquiries, splitting its finite observation capability under priorities. | More flexible scheduling, but harder to communicate completion rates and resource allocation. |

**Selected: A.** Each fitted focused scanner pursues one active enquiry, with ordinary scanner AI managing a prioritised queue. Multiple separately fitted scanners can work concurrently where supported. This is equipment capability, not a player-complexity change in physics. Broad sensing remains independent and separately powered. Exact enquiry duration, interruption/progress retention, observation geometry and hardware limits remain open.

## S059 — Intrusive / deep scanner

**Inclusion recommendation: retain equipment-supported intrusive analysis with observable exposure and contextual consequences; resolve whether it uses focused-scanner capacity.** [Plan C5](../gdd/mechanics/sensors-epistemics.md#plan--c2c5-knowledge-ladder) describes scanning protected internals, cargo, personnel or vulnerabilities, with a detectable signature and possible authored diplomatic/tactical reactions. This is a Plan baseline, not a Current capability. More time spent observing externally does not necessarily reveal protected information.

S056 supplies supported investigation aims and S058 limits each fitted focused scanner to one active enquiry. Intrusive work could be an operation of suitably capable focused equipment, competing for that enquiry slot, or a separately fitted System with its own capacity and ordinary AI. Neither choice makes every scanner intrusive or guarantees that a protected question can be answered. Obfuscation, shielding and equipment limits still matter.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional equipment-supported deep enquiries that emit a detectable signature and can provoke contextual reactions. Compare focused-scanner operation with separate dedicated hardware. |
| Ongoing activity | Maintain observation geometry/resources and watch exposure or reaction; ordinary AI performs supported scan execution. |
| Active decisions | Decide whether the information justifies intrusion, seek cooperation where useful, prioritise the enquiry or stop when circumstances change. |
| Complexity and AI | Same knowledge, equipment and authority rules for humans/AI. Ordinary AI follows supported orders/doctrine; protected scenario choices retain their existing authority rather than creating a universal confirmation ritual. |
| Interactions and communication | Comms supplies relationship/permission context, Sensors explains the question and exposure, Helm supplies geometry and Engineering supports demand. Others react using evidence they can actually observe. |
| Risks | Intrusion becomes a universal scan button, automatic diplomatic punishment ignores context, deep results expose hidden truth without acquisition or a separate System duplicates the same work without a useful distinction. |
| Dependencies | G018/G020/G021/G024/G039/G040/G109, S016/S027/S056–S058 and S060–S063. Exact detectable signature, resistance, consent/reaction rules, duration and resource costs remain open. |

### Resolved choice: does intrusive scanning use a focused scanner's enquiry slot?

| Option | Form | Trade-off |
|---|---|---|
| A — Operation of capable focused equipment | Intrusive analysis occupies that fitted focused scanner's active enquiry slot; only suitable equipment supports it. | Makes the opportunity cost clear and reuses investigation controls, but competes with other focused work. |
| B — Separate intrusive System | Fit dedicated deep-scanning hardware with its own capacity, resources and ordinary AI. | Allows concurrent focused and intrusive work, but adds a separate fitting and operational surface. |

**Selected: A.** Intrusive analysis is an operation of suitably capable focused equipment and occupies that scanner's one active enquiry slot. Not every focused scanner supports it. Detectable emissions and contextual consequences remain part of the selected direction; evidence and equipment still constrain results. Ordinary scanner AI follows supported orders and normal authority. Exact exposure, resistance, consent/reaction rules, duration and costs remain open.

## S060 — Passive receivers / signal analysis

**Inclusion recommendation: retain passive reception and signal interpretation; resolve whether reception shares the broad scanner's operating state.** [R6's HighFleet/Wolfpack research](./research/comparable-games-and-falling-frontier.md) distinguishes receiving emissions from actively illuminating a subject and from solving its position/motion. G021 already selects operating signatures; G024 selects shared human/AI Comms channels, encryption and interception. Passive reception supplies evidence about observable signals, not automatic decryption, exact identity or a perfect position.

The equipment could be a passive operating mode of a capable broad scanner, or a separately fitted receiver System that can listen while active scans continue. This choice concerns the ship's game equipment, not networking or AI architecture. Detailed bearing-only localisation and target-motion estimation remain S097/S099. Listening does not transmit a scan signal; other operating ship Systems can still expose the ship.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Equipment-supported reception, automatic filtering/correlation and confidence-bearing observations of available emissions. Compare broad-scanner mode with independently operating receivers. |
| Ongoing activity | Monitor unusual signal changes and possible associations; ordinary AI filters routine reception without manual frequency sweeping. |
| Active decisions | Prioritise an unexplained signal, compare hypotheses, ask Comms to interpret traffic or seek active confirmation at an exposure cost. |
| Complexity and AI | Whole-System ordinary AI receives and correlates available evidence. Simple presets summarise useful changes; deeper presets expose supported priorities/hypotheses without compulsory signal puzzles. |
| Interactions and communication | Sensors interprets equipment/signature clues, Comms interprets supported traffic content and Navigation/Helm can improve observation geometry. Findings retain provenance and uncertainty. |
| Risks | Passive reception becomes free omniscience, listening guarantees stealth, routine filtering becomes a chore or encrypted signals reveal their contents automatically. |
| Dependencies | G018–G024/G106/G109/G113, S018/S019/S027/S056–S059/S063/S097/S099. Exact signal types, sensitivity, classification, localisation and resource costs remain open. |

### Resolved choice: can passive receivers operate independently of broad active sensing?

| Option | Form | Trade-off |
|---|---|---|
| A — Broad-scanner operating mode | Capable broad equipment switches between passive reception and active sensing. | Fewer fitted Systems, but choosing active observation replaces that equipment's passive operation. |
| B — Independent receiver System | Separately fitted receivers have their own resources and ordinary AI, listening concurrently with active sensing where equipped. | Supports complementary observations and equipment variation, but adds a fitting/resource decision. |

**Selected: B.** Passive receivers are independently fitted Systems with their own resources and ordinary individual AI, able to listen concurrently with active sensing. Reception uses available emissions with provenance and uncertainty. It neither guarantees stealth nor automatically identifies sources or decrypts traffic; other ship Systems can still expose the ship. Detailed localisation/motion estimation remains S097/S099. Exact signal types, sensitivity, classification and costs remain open.

## S061 — Probes and remote sensor network

**Inclusion recommendation: retain physical remote sensors, and resolve whether suitable probes can move on survey orders.** [Plan C3](../gdd/future/roadmap-baseline.md) includes remote sensor probes; [R1 item 58](./research/feature-gap-analysis-master.md) proposes a deployed-probe map and viewing through linked sensors. These describe extended coverage, not unlimited shared knowledge. S019 already requires deployables to use ordinary ship entity composition, so a probe has actual sensors, power/storage, communications and appropriate controllers rather than a bespoke omniscient map marker.

G022/G024's communication and G019's lost-observation rules apply to remote reports. The receiving ship knows what has reached it, with age/provenance; a disconnected probe is not a live window into hidden space. Local probe behaviour uses ordinary individual-System AI and knowledge. The remaining scope choice is stationary/ballistic deployment only versus equipment-supported powered survey movement.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Deployable sensor entities with finite equipment/endurance and supported communications. Compare stationary/ballistic probes with optional powered survey craft. |
| Ongoing activity | Monitor coverage, report freshness and equipment/endurance; ordinary AI handles approved observation and, if fitted, movement. |
| Active decisions | Choose deployment geometry, task an area/route, request a useful observation or recover/replace equipment. |
| Complexity and AI | Whole-System ordinary AI operates probe sensors, communications and any propulsion. Simple presets can task supported work without controlling each manoeuvre; no special probe-only AI architecture. |
| Interactions and communication | Navigation/Helm supplies deployment and survey geometry, Sensors defines information needs, stores manages finite equipment and Comms supplies usable report paths. |
| Risks | Disposable omniscient scouts, probe babysitting, infinite endurance, automatic knowledge through jammed links or a second craft-control game. |
| Dependencies | G019/G022/G024/G061/G065/G109, S017/S019/S043/S052/S056–S060/S063. Exact links/relay support, endurance, recovery, fitting and task interfaces remain open. No human piloting requirement is introduced. |

### Resolved choice: should suitable probes support powered survey movement?

| Option | Form | Trade-off |
|---|---|---|
| A — Stationary/ballistic deployment only | Coverage depends on where probes are launched or placed; there are no powered survey-route orders. | Simple deployment choices and fewer craft tasks, but coverage cannot adapt without new deployment. |
| B — Optional mobile survey probes | Propulsion-equipped variants can execute approved survey routes using ordinary movement AI; stationary variants remain useful. | Adds reconnaissance/recovery choices and equipment variation, but requires clear endurance and communications limits. |

**Selected: B, with EVE-inspired triangulation.** Propulsion-equipped probe variants execute approved survey routes using ordinary individual movement AI; stationary variants remain useful. Probes retain ordinary ship/System composition, finite equipment/endurance and actual communication paths. The user identifies EVE's probe triangulation as inspiration for gaining more data through deployment geometry; [R9](./research/eve-probe-scanning.md) records the verified reference and proposed transfer. Exact fusion, task controls, recovery and contributing-source rules remain open. Routine execution does not require manual piloting or arithmetic.

### Resolved S061 follow-up — Can allied ships contribute to triangulation?

Useful observations can come from separated probes, but player and AI ships use the same underlying sensor Systems. Extending the proposed fusion to compatible ship observations could make fleet positioning and selective data sharing consequential. This is a Phoenix proposal, not a verified EVE feature. Only received, sufficiently relevant observations can contribute; old reports, repeated copies and incompatible readings do not magically produce independent evidence.

| Option | Form | Trade-off |
|---|---|---|
| A — Probe network only | Triangulation combines compatible probe observations; ships receive its results. | Keeps the feature's scope narrow, but treats otherwise similar ship sensors differently. |
| B — Any compatible shared sensor source | Probes and ships can contribute compatible observations through supported information sharing. | Adds fleet reconnaissance coordination, but needs clear provenance, age and geometry limits. |

**Selected: B.** Compatible shared sensor observations from ships and probes can contribute to triangulation. Actual report delivery, observation age, provenance and useful geometry constrain contributions; repeated copies are not independent evidence. Players can coordinate viewpoints and selective sharing without automatic faction-wide knowledge or a networking/AI architecture change. Exact fusion and supported information types remain open.

## S062 — Database / Intelligence correlation tools

**Inclusion recommendation: retain known-reference retrieval and evidence correlation; resolve whether routine analysis needs a processing-time resource.** [Plan C6](../gdd/mechanics/comms-commitments.md#plan--c6-intelligence-c7-personnelsurrender) gives Comms the primary Intelligence surface, with station-specific projections of known records and shared evidence. G023 already selects automatic routine reference attachment and explicit player finding/flagging of unusual records. G106 supports meaningful competing hypotheses; neither source turns a record into proof of a subject's actual state.

The remaining question is whether searching and correlating already available information is responsive tool use, or a timed processing job competing for finite computer capacity. Actual acquisition, sensor observation, decryption and remote communication can have their own real constraints; this entry concerns routine lookup/correlation of information the ship already holds. Final Station composition remains open rather than assigning all intelligence tasks anew here.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Searchable known references, related observations, source/time and highlighted contradictions. Preserve G023's unusual-record exception and G026's factual history without compulsory prose annotation. |
| Ongoing activity | Notice mismatches between reports, reference patterns and current evidence; ordinary AI retrieves routine matches and relevant changes. |
| Active decisions | Search an unusual lead, attach a relevant unusual record, compare hypotheses and request evidence that distinguishes them. |
| Complexity and AI | Simple presets surface routine matches; deeper presets expose search/comparison. Ordinary AI uses only available records/evidence, with no world-global truth or extra knowledge. |
| Interactions and communication | Comms supplies claims/history, Sensors supplies observations and other operators explain what uncertainty would change their decisions. Correlation supports advice rather than selecting other Systems' actions. |
| Risks | Database work becomes waiting for progress bars, automatic matching solves every investigation, outdated records appear current or manual copying replaces meaningful reasoning. |
| Dependencies | G018/G023/G024/G026/G106/G109, S056/S061/S063 and later Comms entries. Exact record acquisition, search interface, hypothesis presentation and access rules remain open. |

### Resolved choice: does routine database correlation consume timed processing capacity?

| Option | Form | Trade-off |
|---|---|---|
| A — Responsive information tool | Routine retrieval/correlation of held information has no additional gameplay processing timer or capacity budget. | Keeps attention on interpretation and missing evidence; acquisition still has its own constraints. |
| B — Timed analysis jobs | Routine correlation consumes finite processing capacity, with ordinary AI scheduling jobs. | Adds prioritisation and equipment choices, but risks creating wait-based work that duplicates scanner enquiries. |

**Selected: A, with AI reading time.** Routine retrieval/correlation of held information is responsive tool use, without an additional gameplay processing timer or capacity budget. AI-controlled Stations must simulate reading time before using relevant text rather than digesting it instantly; this is controller behaviour, not an analysis-computer resource. Interpretation, unusual-record attachment and choosing missing evidence remain activities. Acquisition, observation, decryption and communication retain their own constraints. Exact reading-time rules, search, access and presentation remain open.

## S063 — Contact designation and information transfer

**Inclusion recommendation: retain advisory designation and selective evidence sharing; resolve what a restored shared feed delivers.** [The Current baseline](../gdd/mechanics/sensors-epistemics.md#science-target-and-designation) separates Science designation from Tactical authority. Plan C2 adds per-ship observation stores and communicated evidence. G022 already selects Navigation/Sensors/Tactical feed sharing, preserves source/age/confidence and lets recipients decide what to do. S061 lets compatible received observations contribute to triangulation; it does not give faction-wide automatic knowledge.

One-off assessments and supported feeds should use existing communication capabilities and limited knowledge. This is a gameplay information rule, not a transport redesign. If a selected feed loses its in-fiction link, the recipient's last evidence ages normally. When communication resumes, should the feed automatically send relevant retained findings from the gap, or only new live updates unless history is explicitly requested? Neither option supplies information the source did not observe or retain.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Advisory contact designation, selected live feeds and supported evidence reports with provenance/observation time. Resolve restoration of relevant retained findings after a communication gap. |
| Ongoing activity | Track consequential changes and report freshness; routine transmission for selected sharing can be automated. |
| Active decisions | Choose audience/feed, flag an important assessment, request clarification and explain implications rather than manually copying every contact. |
| Complexity and AI | Ordinary communication/sensor AI transmits permitted available information. Recipients retain their own System authority, knowledge and interpretation; human control does not require repetitive refresh actions. |
| Interactions and communication | Sensors shares evidence, Comms provides usable channels/context and fleet operators coordinate viewpoints or action. Captain still receives reports and deliberate Viewscreen sharing, with no personal map. |
| Risks | Report copies inflate confidence, old information looks live, automatic sharing spreads beyond the selected audience or transient link gaps produce routine resend chores. |
| Dependencies | G018/G019/G022–G026/G109, S055/S056/S061/S062 and later Comms entries. Exact feed scope, retention, delivery limits and revocation remain open. Observation time stays distinct from receipt time. |

### Resolved choice: what happens when an enabled shared feed reconnects?

| Option | Form | Trade-off |
|---|---|---|
| A — Automatic relevant catch-up | Send relevant retained findings from the gap as communication permits, preserving their observation time and source; resume live updates. | Avoids routine resend work, but needs relevance/retention rules and clear historical presentation. |
| B — Live updates only | Resume new observations; missed historical findings require an explicit request or report. | Makes historical exchange deliberate, but important information can be missed after brief interruptions. |

**Selected: A, clarified as normal history availability.** Probe/sensor views generally show historical data. Retained findings from a communication gap become available through that same history when sharing resumes as the channel permits; no separate catch-up interface or gameplay System is needed. Preserve source and observation time, then resume live updates. Historical findings do not overwrite newer evidence, become live tracks or count as independent observations when copied. Selected audience/feed limits remain. Exact retention, delivery and revocation remain open.

## S064 — Environment / resource / anomaly scanner

**Inclusion recommendation: retain observation of environmental and resource subjects, and resolve predictive assistance.** [The baseline sensor design](../gdd/mechanics/sensors-epistemics.md#plan--c2c3-fields-and-hazard-knowledge) plans field observations and bounded forecasts; Current infrastructure scanning exposes supported condition/capacity readings. G029/G030 select contextual hazard opportunities and bounded causal field evolution; G045 adds selected route-changing anomalies. Resource prospecting remains a candidate equipment/activity variant, not universal Current capability.

S056–S059 provide enquiries and ordinary scanner execution. Environmental subjects should use those evidence principles rather than a separate obligatory minigame. The unresolved activity question is whether players receive only measurements/history and infer future conditions, or also receive conditional forecasts supported by observations. Forecasting must not read undiscovered field parameters, hidden script events or guaranteed future actor choices.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Equipment-supported readings of fields, anomalies and useful resources; focused enquiries can seek deeper properties. Decide whether to add bounded forecasts with explicit assumptions/uncertainty. |
| Ongoing activity | Monitor changing intensity, source behaviour and observation age; ordinary AI maintains supported measurements. |
| Active decisions | Choose a useful sample/viewpoint, identify a potential safe window or recovery opportunity, and ask the crew to change course or intervene at a source. |
| Complexity and AI | Simple presets highlight consequential trends; deeper presets expose supported enquiries/forecast assumptions. Ordinary AI works from the same evidence, not authoritative hidden future state. |
| Interactions and communication | Sensors explains likely conditions, Helm/Navigation weighs routes/windows, Engineering estimates endurance and recovery operators weigh material value against exposure. |
| Risks | Perfect future knowledge, every anomaly becoming a progress bar, environmental work being isolated from the crew or forecasts concealing assumptions as certainty. |
| Dependencies | G018/G019/G028–G031/G045/G046/G109, S056–S063 and later recovery/resource entries. Exact equipment, observable properties, sampling, forecast models and resource extraction remain open. |

### Resolved choice: should environmental scanning provide conditional forecasts?

| Option | Form | Trade-off |
|---|---|---|
| A — Measurements and history | Show observed conditions/trends; players infer future windows and risks. | Keeps prediction with players, but can demand repetitive mental estimation. |
| B — Evidence-based forecasts | Also project supported changes/windows with visible uncertainty and assumptions. | Helps players plan and communicate choices, but requires clear limits when evidence is weak or causes change. |

**Selected: B.** Environmental scanning provides evidence-based forecasts with visible uncertainty and assumptions. Players choose which risk/opportunity to act on and what further observation to seek. Ordinary AI estimates from the same available evidence; undiscovered parameters, hidden scripted events and guaranteed future actor choices are not revealed. Forecasts can be incomplete or wrong when conditions/causes change. Exact models, observable properties, equipment and sampling remain open.

## S065 — Hail / Comms transceiver

**Inclusion recommendation: retain contextual generic requests, authored conversations and human media; resolve concurrent conversation scope.** [The Current baseline](../gdd/mechanics/comms-commitments.md) has chronological dialogue threads and physical reachability, separating failure to deliver from refusal by a recipient. G022 already selects generic requests for every ship, contextual to relationship/objectives and separate from plot Comms; a human-controlled recipient uses text chat. Human text/audio/video remains supported, while AI communicates through text with urgency sounds rather than voices.

G024's public chatter/encryption and S063's selected sharing remain separate supported activities. The next decision is whether operational conversations are limited to one active counterpart or can remain open concurrently. Concurrency does not give AI instant comprehension: S062's simulated reading time still applies where relevant. Normal authority, range, interference and knowledge remain unchanged.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Hails, relationship/objective-contextual requests and supported human media. Compare one active operational conversation with concurrent persistent threads. |
| Ongoing activity | Listen/read for changing needs, relevant chatter and responses; routine triage can be delegated to ordinary Comms AI. |
| Active decisions | Choose whom to contact, prioritise a request, discuss terms with the crew and respond through existing authority. |
| Complexity and AI | Simple presets surface urgent/relevant threads; deeper presets expose more conversation management. Ordinary AI uses limited knowledge and simulated reading time, with no mandatory voice acting. |
| Interactions and communication | Other operators request information, access or actor action; Comms conveys requests and responses without directly controlling recipient machinery. |
| Risks | Message overload, one counterpart blocking all external coordination, simultaneous audio becoming unintelligible or AI replying instantly to unlimited text. |
| Dependencies | G010/G022/G024/G039–G044/G079/G109, S018/S062/S063 and S066 onward. Exact media focus, urgency, thread presentation and generic request vocabulary remain open. |

### Resolved choice: can several operational conversations remain open at once?

| Option | Form | Trade-off |
|---|---|---|
| A — One active counterpart | Switch the active operational conversation between ships; other traffic remains in its supported feeds/inbox. | Clear focus, but several simultaneous requests require more switching. |
| B — Concurrent threads | Maintain multiple operational text/dialogue threads, with clear urgency and selected media focus. | Supports fleet coordination without waiting on one counterpart, but needs strong triage and readable presentation. |

**Selected: B — like WhatsApp.** Maintain concurrent persistent operational text/dialogue threads, with clear urgency and selected audio/video focus. Generic contextual requests remain separate from plot dialogue; human-controlled recipients use text chat. Ordinary Comms AI triages supported work with limited knowledge and simulated reading time. This does not require simultaneous playback of every media conversation; AI voices remain excluded. Exact thread/media presentation and request vocabulary remain open.

## S066 — Interception / frequency search receiver

**Inclusion recommendation: retain shared-channel interception and resolve listening coverage.** G024 already selects the same Comms channels for AI/player fleet targeting and chat, including interceptable enemy text in team PvP, encryption levels and low-power public chatter. S060 supplies independently operating passive receivers. S065's conversation model does not make an intercepted conversation equivalent to membership or provide messages that were never received.

The remaining choice is whether suitable receivers collect every receivable channel automatically, or have finite concurrent listening/search coverage allocated by ordinary AI and operator priorities. In both cases public chatter can remain an automatic feed; interception reveals only signals available under actual reach, interference and equipment. Decoding protected content is S067, not a guaranteed consequence of reception. Exact interception of human audio/video remains open.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Equipment-supported discovery and monitoring of available transmissions, with source/time and uncertainty. Compare automatic all-channel collection with finite monitored coverage. |
| Ongoing activity | Follow consequential chatter/traffic changes; ordinary receiver AI searches and filters supported signals. |
| Active decisions | Prioritise a transmitter/channel/area, investigate an unusual burst and decide which captured traffic merits interpretation or decoding. |
| Complexity and AI | Simple presets delegate search/listening policy; deeper presets expose meaningful priorities. No compulsory manual frequency sweep, code-entry puzzle or deliberately incompetent AI. |
| Interactions and communication | Comms interprets messages, Sensors correlates available emission evidence and other operators explain which information could change their actions. Captured information does not grant authority over another ship. |
| Risks | Enemy chat becomes universally visible, channel hunting becomes rote, lost reception silently produces complete history or encryption is bypassed by the interception UI. |
| Dependencies | G018/G019/G022/G024/G103/G109, S060/S063/S065/S067/S097. Exact spectrum model, coverage, discovery, resources and audio/video scope remain open. Networking and admission remain unchanged. |

### Resolved choice: should interception have finite listening coverage?

| Option | Form | Trade-off |
|---|---|---|
| A — Collect all receivable channels | Suitable equipment automatically captures all transmissions within its actual reception limits. | Keeps operation simple, with choices concentrated on interpretation and decoding. |
| B — Prioritised finite coverage | Equipment has limited concurrent listening/search coverage; ordinary AI allocates it under priorities, alongside routine public chatter. | Adds meaningful reconnaissance priorities and equipment variation, but needs readable gaps without manual tuning chores. |

**Selected: B.** Interception equipment has finite concurrent listening/search coverage, allocated by ordinary receiver AI under supported priorities; routine public chatter remains automatic. Players choose consequential priorities without compulsory manual frequency sweeping. Coverage gaps and missed intervals must be readable. Reception remains physically/epistemically constrained and does not decrypt protected content or recover transmissions never captured. Exact coverage, discovery, costs and audio/video scope remain open.

## S067 — Cipher / decoding suite

**Inclusion recommendation: retain encryption levels and supported automated decoding; resolve whether time alone can defeat every level.** G024 selects encryption alongside low-power public channels and interceptable AI/player fleet traffic. S066 supplies finite listening coverage; captured encrypted traffic is evidence of a transmission, not its plaintext. G103 excludes compulsory cipher/code puzzles. This is fictional game information access, not a change to Phoenix transport security or admission.

Decoding can be ordinary whole-System AI work whose priorities reflect operational value. The key scope choice is whether enough analysis time always reveals a captured message, or whether some protection exceeds the available equipment/knowledge and needs better capability or acquired keys. In either case the player decides whether the information is worth pursuing; no fabricated plaintext may substitute for an unresolved result.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Encryption levels, usable authorised keys and equipment-supported analysis of captured traffic. Decide whether capability/key limits can make a message currently undecodable. |
| Ongoing activity | Ordinary AI executes supported analysis and monitors applicable access; Comms follows relevant traffic and reading time remains distinct from decoding work. |
| Active decisions | Prioritise traffic, obtain a source/key/capability or abandon analysis whose cost/value is poor. |
| Complexity and AI | Same capture, equipment, knowledge and key rules for AI/humans. Simple presets delegate scheduling; deeper presets expose useful priorities rather than manual cryptography. |
| Interactions and communication | Comms interprets results, Engineering supplies actual resources and mission/team work may acquire relevant intelligence where authored. No universal key-hunting side mission is required. |
| Risks | Encryption is only a meaningless delay, an unbreakable level removes all counterplay, progress bars replace decisions or protected player chat leaks without valid access. |
| Dependencies | G023/G024/G103/G109, S060/S062/S065/S066 and later information-acquisition work. Exact levels, costs, analysis granularity, key scope/rotation and partial-information rules remain open. |

### Resolved choice: can enough time alone decode every captured transmission?

| Option | Form | Trade-off |
|---|---|---|
| A — Time always succeeds | Encryption levels chiefly increase analysis time/resource requirements. | Predictable progress, but every protected channel eventually becomes readable by waiting. |
| B — Capability and key limits | Some protection exceeds current decoding capability; better equipment, acquired keys or other supported intelligence may be needed. | Makes protection and intelligence acquisition meaningful, but needs clear limits and available counterplay. |

**Selected: B.** Some protection exceeds available decoding equipment/knowledge; time alone does not guarantee plaintext. Better equipment, acquired keys or other supported intelligence may be needed. Ordinary decoding AI executes feasible work under priorities, with no compulsory cipher puzzle or transport-security change. Exact levels, costs, analysis granularity, key scope/rotation, partial information and acquisition opportunities remain open.

## S068 — Dossier / commitment interface

**Inclusion recommendation: retain evidence-based actor dossiers and content-specific commitment records; resolve predictive commitment warnings.** [The Current baseline](../gdd/mechanics/comms-commitments.md#commitments) records explicit promises, their terms and settlement, with actor dossiers and campaign continuity. G041 limits formal commitments to authored plots or PCG side missions; ordinary generic requests do not automatically become contracts. G023/G109 preserve provenance and distinguish reference claims from current evidence.

S062 already selects responsive correlation with simulated AI reading time, so routine record retrieval needs no new processing job. The dossier can show formal terms and supported status, or additionally flag a commitment as at risk when known deadlines, observed conditions or reported capabilities conflict with fulfilment. Such a warning is an evidence-based assessment, not authoritative failure, hidden world knowledge or an automatic plan for other Stations.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Actor identity/claims, dated evidence, unusual-record attachments and formal content-specific terms/status. Compare record-only tracking with conditional at-risk warnings. |
| Ongoing activity | Notice changed claims, relevant contradictions and commitments affected by current events; ordinary AI maintains supported records. |
| Active decisions | Challenge a claim, seek confirmation, explain a threatened promise to the crew or renegotiate through supported Comms. |
| Complexity and AI | Simple presets surface consequential changes; deeper presets expose evidence/terms. Ordinary AI uses available knowledge and reading time; protected choices retain normal authority. |
| Interactions and communication | Comms explains obligations/claims; Navigation, Sensors, stores and other operators provide relevant known evidence. They retain responsibility for their own actions and assessments. |
| Risks | Omniscient promise status, every request becoming a contract, alert spam, player prose bookkeeping or automated warnings silently choosing the crew's response. |
| Dependencies | G023/G026/G040/G041/G079/G109, S062–S067 and later resource/mission Systems. Exact dossier projections, risk criteria, warning cadence and renegotiation content remain open. |

### Resolved choice: should dossiers flag formal commitments that appear at risk?

| Option | Form | Trade-off |
|---|---|---|
| A — Terms and supported status only | Show the promise, evidence and actual settlement; players recognise emerging fulfilment risks. | Keeps interpretation with the crew, but requires more manual cross-checking. |
| B — Conditional risk warnings | Also highlight likely problems supported by known deadlines, observations or reported capability, explaining the evidence. | Helps Comms bring consequential changes to the crew, but warnings must show assumptions and avoid noise. |

**Selected: A — Comms can judge for themselves.** Dossiers show terms, evidence and supported status without an automatic commitment-risk judgement layer. Human Comms interprets emerging fulfilment risks and raises concerns with the crew. Ordinary AI can maintain/summarise records and reasons within normal knowledge/authority when controlling Comms; that does not add a universal at-risk warning to human dossier views. Formal commitments remain content-specific under G041. Exact projections and supported renegotiation content remain open.

## S069 — Civilian / allied order terminal

**Inclusion recommendation: retain contextual requests through normal Comms, and resolve group addressing.** [The Current traffic baseline](../gdd/mechanics/navigation-relative-motion.md#civilian-traffic-and-orders) separates delivery, acceptance, refusal and subsequent compliance. G022 extends generic requests to every ship, contextual to relationship/objectives and separate from plot dialogue; S065 selects WhatsApp-like concurrent threads. This entry should be a supported request surface in that experience, not another mandatory console or a way to directly actuate another ship.

Individual recipients retain their own objectives, willingness, authority, knowledge and capacity. Human-controlled Comms uses text; AI-controlled recipients respond through ordinary communication and individual-System behaviour. A remaining fleet-scale choice is whether a supported request can address a selected group at once or must be sent separately to each actor. Group delivery must not imply unanimous acceptance or reveal hidden recipient readiness.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Supported contextual requests such as hold, divert, assist or retreat through ordinary Comms. Compare individual-only addressing with optional group addressing. |
| Ongoing activity | Follow delivered responses and observed behaviour; distinguish silence, refusal and changed circumstances. |
| Active decisions | Choose recipient(s), ask for a useful outcome, clarify authority/context and adapt when actors cannot or will not comply. |
| Complexity and AI | Ordinary Comms AI can issue/triage supported requests under intent; each recipient's individual Systems execute only their own authorised work. No central fleet controller or guaranteed obedience. |
| Interactions and communication | Navigation/Tactical supplies useful geometry/targets, Comms conveys context and recipients communicate their own choices. Humans still coordinate interpretation and competing objectives. |
| Risks | Fleet requests become repetitive clicking, broadcasts force all recipients into one action, one acknowledgement falsely represents the group or a new order terminal duplicates Comms. |
| Dependencies | G022/G024/G039/G040/G044/G063/G079/G112, S052/S053/S065/S068. Exact supported verbs, group membership, response presentation and authority rules remain open. |

### Resolved choice: can a request address several selected ships at once?

| Option | Form | Trade-off |
|---|---|---|
| A — Individual requests only | Send each actor its own request/conversation. | Keeps negotiation personal, but makes fleet coordination repetitive. |
| B — Optional group requests | Send a supported request to a selected group/channel; each recipient decides and responds independently. | Supports fleet-scale coordination, but needs readable recipient responses and clear audience. |

**Selected: B, with fixed groups.** Group audiences are **own fleet, per faction, and all ships**. Individual contact remains supported; arbitrary custom group construction is not selected. Each recipient independently decides/responds using its own authority, willingness, objectives, knowledge and capacity. Actual channel reach/security still applies. Group requests remain normal Comms, not remote actuator control or guaranteed coordinated execution. Exact verbs and response presentation remain open.

## S070 — Captain status / objectives dashboard

**Resolved from established decisions; no new Captain authority is proposed.** Retain mission/objective information and supported reports as aids to coordination. The earlier confirmed Captain decisions and G011/G025 establish very few direct controls, no private map, no duplicate specialist controls and no automatically complete tactical briefing. Crew reports, discussion and deliberate Viewscreen sharing remain necessary. Existing alert-state and AI-stance controls support the role, without making every routine action require Captain approval.

Ordinary AI colleagues report within limited knowledge and normal communication; the dashboard cannot silently read all their hidden state. Final Station composition and exact presentation remain open. S071/S072 review the named alert/stance capabilities in order rather than reopening the Captain's whole role.

## S071 — Alert posture / Red Alert

**Inclusion recommendation: retain explicitly declared alert posture, and resolve the number of meaningful readiness states.** [The Current Captain console](../../wiki/entities/captain-console.md) operates binary Red Alert through normal System authority. G025 already selects danger discovery through ordinary player tools and Captain-set alert conditions. Ordinary AI uses alert state alongside orders, missions and observed situation; no automatic danger-discovery or alert escalation is added here.

The open question is whether binary routine/combat posture is sufficient or whether a small authored set can express intermediate readiness. Any added level must change supported operating intent with meaningful resource/readiness consequences, not grant free buffs, seize human controls or become ceremonial button work. Individual Systems still execute their own policies under the existing AI structure.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Explicit ship alert condition; compare binary Red Alert with a small authored readiness ladder. |
| Ongoing activity | Listen to reports and assess whether current readiness suits the situation. Local operators maintain real resources and capability. |
| Active decisions | Declare/change posture and explain why preparedness or restraint matters now. |
| Complexity and AI | Coarse controls remain simple at every Station depth. Ordinary individual AI interprets posture within its knowledge/orders; humans retain local authority. |
| Interactions and communication | Sensors/Comms reports support judgement; Engineering explains reserve/thermal costs; weapon/defence operators explain readiness. Alert alone does not communicate the whole plan. |
| Risks | Free combat bonuses, automatic threat-driven escalation, every crisis requiring repetitive toggling or alert policy silently overriding human operators. |
| Dependencies | G009/G011/G025/G039/G114, S027/S031–S038/S070/S072. Exact states, System responses, costs and precedence remain open. |

### Resolved choice: should alert posture remain binary?

| Option | Form | Trade-off |
|---|---|---|
| A — Binary routine/Red Alert | Retain two clear states; finer intent comes through existing orders and stances. | Very simple, but offers no shared intermediate readiness condition. |
| B — Small readiness ladder | Add a few distinct states, such as routine, heightened readiness and combat, where each earns a useful operating difference. | Supports preparedness before combat, but needs meaningful policies without extra ritual or free buffs. |

**Selected: B.** Use a small, explicitly selected readiness ladder with distinct useful operating policies and actual System resource/readiness consequences. No exact count or state names are chosen. Levels must not grant free buffs, require ceremonial work or seize human controls. Ordinary individual AI interprets posture alongside orders, missions and observed situation; no automatic danger-discovery/escalation is added. Exact policies and precedence remain open.

## S072 — Command / AI stance controller

**Inclusion recommendation: retain coarse stance/intent controls for ordinary individual System AI; resolve conflict with explicit orders.** [The Current baseline](../gdd/systems/ai-and-backfill.md) uses authored Station stances alongside per-System policies, selectors and shared coordination. G003/G004 require competent ordinary AI and the same behaviour for temporary absence, G079 permits outcome-first objectives with procedures, and G114 retains viable settings/hysteresis. S071 adds a small alert ladder without changing this structure.

Command should express intent and priorities rather than operate every specialist control. Human-held Systems retain local authority, and no stance grants extra knowledge, resources or physical capability. The remaining precedence question concerns an explicit supported order that conflicts with a general preference, such as a cautious AI receiving a deliberate approach order. Hard permissions, physical eligibility and protected scenario choices remain separate constraints.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Coarse authored stances and supported priorities/orders for delegated work. Decide whether an explicit order can override a conflicting general stance preference. |
| Ongoing activity | Listen to ordinary AI reports and assess whether delegated intent still suits the crew's plan. |
| Active decisions | Give/revise an order, change posture/priority or take control of a System when desired; no mandatory stance micromanagement. |
| Complexity and AI | Stance influences ordinary individual AI; no central planner, alternate temporary Backfill policy or per-System complexity editor. Supported orders still use normal authority/communication. |
| Interactions and communication | Captain communicates intent, specialist operators explain capability and local AI reports action/blocking reasons within available evidence. Orders do not substitute for communicating an unexpressed plan. |
| Risks | Stance changes erase explicit intent, orders silently bypass human authority, every exception needs multiple configuration changes or generic preference overrides real permissions. |
| Dependencies | G003/G004/G006/G011/G039/G079/G114, S027/S055/S070/S071. Exact stance vocabulary, order lifetime, conflict reporting and alert-policy interactions remain open. |

### Resolved choice: does an explicit order override a conflicting stance preference?

| Option | Form | Trade-off |
|---|---|---|
| A — Explicit order takes precedence | A supported authorised order can override general stance preferences; physical limits, permissions and protected choices still apply. | Lets the crew express deliberate exceptions without changing several settings. |
| B — Stance remains a ceiling | AI reports the conflict and waits for the stance to change before following the incompatible order. | Makes standing restrictions stronger, but adds configuration work for deliberate exceptions. |

**Selected: A.** Explicit supported orders take precedence over general AI stance preferences within normal authority, physical limits and permissions. Protected choices remain protected and human-held Systems retain local control. Stances provide default guidance to ordinary individual AI; no central planner or special temporary Backfill behaviour is added. Genuine blockers are reported through normal communication. Exact order lifetime, stance vocabulary and alert-policy interactions remain open.

## S073 — Viewscreen and auxiliary display controls

**Inclusion recommendation: retain deliberate communal presentation, and resolve whether sharing follows private browsing.** [The Current baseline](../gdd/systems/station-experiences.md#shared-viewscreen) treats the Viewscreen as communal: authorised Stations request permitted surfaces, Captain selects supported cameras and the latest valid command wins with visible owner/mode. The confirmed redesign rejects a private Captain map and relies on crew conversation plus putting information on screen. Auxiliary outputs must follow the same evidence/authority boundaries rather than supplying hidden information.

The next choice concerns what an operator publishes. A shared view could continuously mirror their current console browsing, or pin a deliberately chosen subject/surface while they continue local work. Pinning can still show live supported updates to that subject; it need not freeze a stale screenshot. Neither form automatically publishes every finding or gives the Captain a separate information surface.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Deliberate supported communal views with visible source/topic and truthful uncertainty. Compare mirroring local browsing with pinned shared presentation. |
| Ongoing activity | Follow a useful shared picture while specialist operators continue their own work; no dedicated camera operator required. |
| Active decisions | Put a relevant subject/evidence/view on screen, explain it and deliberately replace/clear it when the discussion changes. |
| Complexity and AI | Coarse publication controls at simple depths; ordinary AI can present supported information under authorised intent. No automatic complete briefing or privileged knowledge. |
| Interactions and communication | Sharing supports the operator's explanation and crew discussion. Camera choice and displayed evidence do not move specialist control authority. |
| Risks | Browsing silently changes the crew's discussion topic, shared information appears current when stale, a publication queue creates waiting or auxiliary screens automate away crew reports. |
| Dependencies | G011/G016/G018/G019/G106/G109, S054–S056/S063/S070/S072. Exact surfaces, arbitration, auxiliary layout and publication controls remain open; existing network/display architecture is retained. |

### Resolved choice: does a published view follow the operator's browsing?

| Option | Form | Trade-off |
|---|---|---|
| A — Mirror current browsing | The shared surface follows the publishing operator's local selection/view until sharing ends. | Quick to demonstrate, but routine local work can disrupt the shared topic. |
| B — Pin the published topic | Deliberately publish a subject/surface that keeps supported updates while local browsing remains independent; replace it explicitly. | Preserves shared context, but needs clear local/shared selection indicators. |

**Selected: A.** Shared presentation follows the publishing operator's current browsing/selection until sharing ends. Changing the local view changes the shared view; an independently pinned shared topic is not selected. Publication remains deliberate and limited to supported information, with stale/uncertain evidence labelled. No private Captain map or transfer of specialist control is added. Exact surfaces, arbitration and auxiliary layouts remain open.

## S074 — Mission / officer log

**Resolved from G026: automatic factual mission history only.** Retain source/time and ordinary knowledge limits for supported reports/events. No player-authored officer annotations, compulsory prose or invented reasoning are added. Players can inspect historical evidence to support their own interpretation and commitments; S063's sensor history remains part of normal observation tools. Exact recap, retention and campaign persistence remain open. There is no new log-writing activity to approve.

## S075 — Internal damage-control teams

**Inclusion recommendation: retain located teams repairing components with appropriate parts; resolve whether teams can concentrate on one repair.** [The Current baseline](../gdd/mechanics/damage-repair.md) gives repair teams travel, on-site work and return phases. G057/G058 select component conditions and evidence-led restoration; G064 allows most components to be field-replaced, with some catastrophic large-System losses beyond field repair. G070 gives a System-marked ship map and automatic routing to assigned work; S043 makes material deliveries depend on reachable internal routes.

These decisions already provide triage, access and supply constraints. The remaining choice is whether each component task admits one team, or suitable tasks can accommodate multiple teams to accelerate urgent work. More teams must not bypass access, missing parts or a component's actual working-space/equipment limits. Exact rates and diminishing returns need tuning, not a universal linear bonus.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Teams travel to supported component work and consume appropriate parts. Compare one team per task with equipment/site-limited cooperative repair. |
| Ongoing activity | Maintain useful team/material allocation while monitoring changed capability and travel/access constraints. |
| Active decisions | Prioritise a repair, divert a team, concentrate on urgent restoration or spread teams across failures. |
| Complexity and AI | Ordinary repair AI can manage the whole System under priorities; team routing is automatic. Simple presets delegate scheduling, while deeper control exposes consequential allocation rather than manual walking. |
| Interactions and communication | Local operators report symptoms/capability needs, Engineering prioritises support, stores supplies parts and team controllers report actual access/work status. |
| Risks | Team stacking is always optimal, repair remains a wait-only activity, repeated dispatch becomes a chore or multiple teams repair through missing materials/blocked access. |
| Dependencies | G057–G065/G068/G070/G110/G114, S028/S039/S042/S043 and S076 onward. Exact task capacity, work/progress rules, rates, safety and interruptions remain open. |

### Resolved choice: can several teams cooperate on one component repair?

| Option | Form | Trade-off |
|---|---|---|
| A — One team per component task | Extra teams work elsewhere or wait for the task to finish. | Clear allocation, but limited ability to prioritise one urgent restoration through manpower. |
| B — Task-limited cooperation | Suitable tasks admit multiple teams, with actual space/equipment limits and tuned returns. | Adds concentrate-versus-spread decisions, but needs limits that prevent universal team stacking. |

**Selected: A, with parallel components.** One team works each component task. A System with several damaged components can receive several teams, each repairing a different component, to accelerate overall restoration. Teams do not stack on the same component. Automatic routing, ordinary repair AI and actual access/evidence/parts constraints remain. Exact component work/progress, travel and interruption rules remain open.

**Architecture refinement — AD51:** component repair can proceed while another team restores the compartment, provided access, safety, materials and other real prerequisites permit. The compartment disabling condition blocks operation, not repair by itself. Repaired components remain disabled until structural recovery reaches AD50's threshold. Jobs and their reports retain separate targets and outcomes; structural recovery does not imply component faults were fixed. See the [parallel repair rule](./architecture/index.html#parallel-structural-repair).

## S076 — Diagnosis / tests / damage reports

**Inclusion recommendation: retain evidence-led diagnosis, and resolve whether supported tests can affect operation.** [The Current baseline](../gdd/mechanics/damage-repair.md#information-model) gives local operators detailed System information and Engineering additional diagnosis through on-site teams. Plan C4's symptom-to-order matching is a separate candidate; G058 selects readings, symptoms and reliably detected faults, with tests/on-site investigation for ambiguity rather than mandatory copied codes or a puzzle. Ordinary AI uses the same evidence.

S075 limits each component work task to one team while allowing parallel work across components. Diagnosis need not hide an obvious fault or delay every repair. The open question is whether tests only observe, or whether suitable diagnostic procedures can temporarily change output or interrupt the tested capability—for example cycling a suspect power link. Such a test must have a clear expected operating effect and respect the tested System's normal authority.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Local readings, symptoms, detected faults and supported tests/on-site inspection. Compare observation-only diagnostics with selected operationally disruptive tests. |
| Ongoing activity | Watch symptoms and relevant trends; routine detection/reporting is automated rather than a healthy-System maintenance ritual. |
| Active decisions | Choose which ambiguity matters, obtain a useful test or team inspection and decide when an operational interruption is acceptable. |
| Complexity and AI | Simple presets delegate diagnosis; deeper presets expose meaningful choices. Ordinary individual AI uses available evidence and operates only authorised Systems, without hidden fault knowledge. |
| Interactions and communication | Local operators describe symptoms, repair/Engineering explains test needs and affected operators coordinate timing. A repair order does not silently seize another human's System. |
| Risks | Diagnostic busywork, arbitrary gotchas, deliberately hidden obvious damage, accidental capability loss or tests that always reveal exact hidden truth. |
| Dependencies | G057–G059/G070/G103/G109/G114, S028/S039/S062/S075/S077. Exact tests, evidence outcomes, duration, diagnosis retention and authority/control surfaces remain open. |

### Resolved choice: can diagnostic tests temporarily affect the tested capability?

| Option | Form | Trade-off |
|---|---|---|
| A — Observation-only tests | Diagnostics do not interrupt or deliberately change output; information costs are time, access and evidence quality. | Easier coordination, but fewer choices about testing during a crisis. |
| B — Selected operational tests | Some equipment-supported tests require a clear temporary operating change/interruption; ordinary inspection remains available where supported. | Adds useful timing and coordination decisions, but effects must be explicit and local authority preserved. |

**Selected: A — operational tests would be too complex.** Diagnostics are observation-only: tests do not interrupt or deliberately change the tested capability. Information costs are time, access and evidence quality. Ordinary AI uses the same readings, symptoms and supported investigation, without hidden fault knowledge or compulsory code puzzles. Exact tests, duration, evidence outcomes and diagnosis retention remain open. The proposed operational-interruption activity is rejected.

## S077 — Repair execution / restoration

**Inclusion recommendation: retain condition-appropriate repair and supported replacement; resolve whether emergency patching needs a universal extra mode.** G057/G058 select component conditions and typed-parts restoration; G064 permits field replacement for most components, with some catastrophic large-System losses beyond field repair. G072 allows limited compatible field swaps while major refits require facilities. S075 selects one team per component task, with parallel work across components; S076 keeps diagnosis observation-only.

The remaining choice is whether each condition has its normal supported remedy, or whether repairs generally offer a separate quick temporary patch versus proper restoration. A universal patch mode adds another judgement for every repair and needs deterioration/rework rules. Condition-specific remedies already provide variation: transient EMP resolves without repair, while damaged or destroyed components need appropriate work/materials. No decision here changes actual access, stocks or catastrophic limits.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Component-condition remedies and supported replacement with actual teams/parts/access. Compare normal remedies with a universal quick-patch alternative. |
| Ongoing activity | Maintain useful work/material priorities and monitor restored capability; ordinary AI executes supported jobs. |
| Active decisions | Prioritise a component, choose supported repair/replacement or redirect work as the crew's needs change. |
| Complexity and AI | Simple presets delegate scheduling; deeper controls expose meaningful priorities. No mandatory per-repair minigame or deliberately inferior AI; same work/material rules apply. |
| Interactions and communication | Local operators explain capability needs, repair reports actual work/parts and stores supports delivery. Status changes remain grounded in completed supported work. |
| Risks | Every repair gains another menu, temporary patches create repetitive rework, restoration becomes an unrelated percentage meter or field work silently reverses catastrophic loss. |
| Dependencies | G057–G065/G070/G072/G114, S025/S028/S039/S042/S043/S075/S076. Exact remedies, work/progress retention, parts consumption and fitting limits remain open. |

### Resolved choice: should emergency patching be a universal repair alternative?

| Option | Form | Trade-off |
|---|---|---|
| A — Condition-appropriate remedies | Use the supported repair/replacement for the condition, without a universal quick-patch versus full-repair choice. | Keeps triage focused on capabilities, teams and materials, with less repeated configuration. |
| B — General quick-patch mode | Most repairs also offer faster temporary restoration with a later cost/risk or proper-repair requirement. | Adds crisis trade-offs, but introduces rework and another choice across many repairs. |

**Selected: A, with automatic fallback patches.** Teams perform full repair where possible and only patch when full repair cannot be done. The team automatically chooses the supported remedy; this is not a routine player patch/full-repair selection. A patch lasts indefinitely at reduced effectiveness and has no chance of spreading. No expiry or recurring rework timer is added. Actual access/materials and one-team-per-component limits remain. This does not grant immunity to new damage or reverse an irreparable catastrophic loss. Exact fallback eligibility, effectiveness/materials and later full-restoration behaviour remain open.

## S078 — Proactive inspection / preventive maintenance

**Inclusion recommendation: retain stress-driven wear and its supported maintenance, and resolve whether prevention requires a separate team assignment.** G060 already selects wear from hard use, overheating or adverse conditions, with no inevitable maintenance timer in healthy ordinary operation. G057 represents component conditions; S075/S077 supply team work and automatic supported remedies. S076 keeps diagnosis observation-only. These rules already let crews prevent wear by managing operation and repair worn components through normal work.

The remaining scope choice is whether to add a separate preventive team assignment to components before wear occurs. It could trade team availability for reduced wear under expected hard use, but also risks making every healthy System need a standing maintenance crew. Neither option changes G060's rejection of routine healthy-operation decay or adds compulsory inspection rounds.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Stress-dependent wear, readable evidence and normal condition repair. Compare prevention through operation with an additional pre-wear team assignment. |
| Ongoing activity | Watch thermal/stress margins and known wear; ordinary AI maintains supported operation/work priorities. |
| Active decisions | Accept hard-use wear for a mission need, reduce stress or prioritise known worn components. If adopted, allocate a team to prevention instead of another task. |
| Complexity and AI | Routine execution remains delegated under ordinary individual AI. No mandatory manual maintenance cycles or healthy-component inspection quota. |
| Interactions and communication | Local operators explain expected hard use, Engineering manages resources and repair prioritises actual work. Prevention must not become a requirement to obtain normal rated performance. |
| Risks | Another always-optimal staffing chore, healthy Systems demand repeated service, maintenance becomes an abstract buff or existing repair work is duplicated. |
| Dependencies | G057/G058/G060/G065/G070/G114, S031–S040/S075–S077. Exact stress thresholds, wear effects, evidence and parts requirements remain open. |

### Resolved choice: should teams have a separate pre-wear preventive assignment?

| Option | Form | Trade-off |
|---|---|---|
| A — Operation and normal repairs | Prevent wear through operating choices; maintain known worn components through ordinary repair tasks. | Keeps maintenance within existing decisions and avoids another standing assignment. |
| B — Also preventive team work | Assign teams ahead of expected hard use to reduce supported wear, tying up those personnel. | Adds preparation choices, but needs limits to avoid mandatory staffing of healthy Systems. |

**Selected: A.** Prevent wear through operating choices and maintain known worn components through ordinary repair tasks. No separate pre-wear preventive team assignment is added. G060's hard-use/overheating/adverse-condition wear remains, without an inevitable maintenance timer or healthy-component inspection quota. Ordinary AI executes delegated operation/repair. Exact thresholds, wear effects, evidence and parts remain open.

## S079 — Duty Team roster and assignments

**Inclusion recommendation: retain typed, finite teams and meaningful assignment commitments; resolve whether teams can change speciality between jobs.** [Plan C7](../gdd/mechanics/duty-teams-and-operations.md#duty-teams) supplies anonymous Engineering, Security, Science and Medical teams without simulating every ensign. G037/G070 reuse the team/task/location framework across ships and facilities; G068 lets named Duty Officers add bonuses while leaderless teams remain functional. S075 selects one team per component task and parallel work across components.

The next choice is whether a team's authored competencies remain stable or whether routine reassignment can turn it into another speciality. Stable types create different scarce capabilities; flexible conversion makes the whole crew more interchangeable and needs equipment/training/changeover rules. This concerns team eligibility, not the eventual Station that owns assignment controls. A team cannot simultaneously occupy two jobs or teleport between locations.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Typed finite teams, located assignments and availability. Compare stable authored competencies with between-job role conversion. |
| Ongoing activity | Maintain a useful reserve and follow actual team commitments/travel. Ordinary AI handles delegated staffing and task execution. |
| Active decisions | Assign or recall a team, weigh an away mission against onboard needs and choose which supported task gets scarce capability. |
| Complexity and AI | Simple presets delegate allocation under priorities; deeper presets expose task/team choices. Anonymous members remain aggregate, with no watches, individual walking or mandatory officer leadership. |
| Interactions and communication | Repair, Security, Medical and Operations explain personnel needs and release/recovery constraints; officers add supported bonuses without replacing base competence. |
| Risks | Every team becomes a universal resource, personnel conversion becomes menu work, fixed types strand useful crew or assignment bypasses actual travel/recovery. |
| Dependencies | G037/G061/G068–G071/G114, S043/S075/S080–S088. Exact competencies, eligibility, team counts and assignment authority remain open; final Station composition is deferred. |

### Resolved choice: can teams routinely change speciality between assignments?

| Option | Form | Trade-off |
|---|---|---|
| A — Stable authored competencies | A team's type/skills determine eligible work; authored overlap can support more than one task family. | Preserves distinct personnel shortages without a role-conversion workflow. |
| B — Reconfigurable roles | Available teams can change speciality between jobs using supported equipment/changeover rules. | Makes staffing flexible, but reduces distinct reserves and adds another management layer. |

**Selected: A.** Duty Teams retain stable authored competencies, with authored overlap where useful; routine between-job speciality conversion is not added. Finite eligible teams are committed to actual tasks with travel, availability and recovery constraints. Anonymous members remain aggregate, and named officers add bonuses rather than basic eligibility. Exact competencies, counts, task eligibility and assignment authority remain open pending final Station composition.

## S080 — Duty Officers / expertise assignments

**Inclusion recommendation: retain named expertise bonuses and resolve onboard reassignment timing.** G068 already selects officers assigned to a team, compatible System or away mission, with ordinary teams functional without leaders. [R8's STO follow-up](./research/sto-duty-officers.md) supports historical specialisation/trait variety but does not determine Phoenix's movement or balance. [Plan C7](../gdd/mechanics/duty-teams-and-operations.md#duty-officers) treats off-duty officers abstractly and does not continuously simulate location or watches.

G070's ship map and automatic team routing provide an available model for coarse assignment travel. The remaining question is whether moving an officer between onboard posts uses that model or applies the bonus immediately. Neither option allows an officer committed to an away mission or otherwise unavailable to appear aboard; G061 recovery and G071 casualty constraints still apply. No individual ensign simulation, walking controls or watch schedule is proposed.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Named officers give explicit compatible bonuses to one supported assignment at a time. Compare coarse automatic onboard transfer with immediate onboard reassignment. |
| Ongoing activity | Preserve useful expertise distribution and monitor availability without repeated bonus swapping. |
| Active decisions | Assign an officer to a useful team/System/mission, accept the lost contribution elsewhere or recall them through supported recovery. |
| Complexity and AI | Ordinary AI can manage delegated assignments within normal authority/knowledge. If travel is adopted, routing is automatic using the ship map rather than manual path commands. |
| Interactions and communication | Operators explain where expertise matters; personnel/Operations coordinates actual availability and Medical restores recoverable casualties. |
| Risks | Instant bonuses encourage constant swapping, travel creates tedious waiting, officers become mandatory operators or roster management expands into character simulation. |
| Dependencies | G061/G068–G071/G114, S043/S075/S079/S081 and away-mission entries. Exact bonuses, compatible slots, transfer rules and assignment authority remain open. |

### Resolved choice: should onboard officer reassignment take travel time?

| Option | Form | Trade-off |
|---|---|---|
| A — Coarse automatic travel | Assignment changes route the officer between posts using existing ship-map movement; the new bonus starts on arrival. | Makes expertise placement a preparation choice and discourages instant bonus swapping. |
| B — Immediate onboard reassignment | Eligible onboard officers change posts/bonuses immediately; external deployment and recovery remain physical. | Keeps personnel management simple, but permits rapid situational switching. |

**Selected: A.** Onboard officer reassignment uses coarse automatic travel through the ship map; the new assignment's bonus starts on arrival. Players choose posts and ordinary routing handles movement, without manual walking, watches or individual ensign simulation. Away-mission commitment and recovery remain physical constraints. Exact compatible slots, bonuses, transfer details and assignment authority remain open.

## S081 — Medical / recovery services

**Inclusion recommendation: retain limited treatment and meaningful casualty recovery; resolve whether minor injuries prohibit duty.** [Plan C7](../gdd/mechanics/duty-teams-and-operations.md#medical) supplies treatment capacity, triage and recovery of simulated personnel, with Science support for specialist observations. G071 selects permanent officer loss only in explicitly lethal circumstances and preserves human participation. G110 reserves manoeuvre-constrained surgery for selected mission-critical work, not routine healing. No continuous bodily-needs or medical minigame is implied.

G068/S080 make expertise assignment useful and located. Injury can therefore remove an officer until treatment or leave minor cases available at reduced effectiveness while severe cases are unavailable. Either approach makes treatment capacity consequential; the latter permits an operational choice about using injured expertise. Exact deterioration/risk is not implied by allowing duty and must remain explicitly supported rather than arbitrary per-tick casualty rolls.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Limited medical capacity, casualty states and supported treatment/recovery. Compare all injuries blocking duty with a coarse minor/severe distinction. |
| Ongoing activity | Ordinary Medical AI handles delegated treatment queues; players monitor relevant availability and capacity without tracking every crew member's biology. |
| Active decisions | Prioritise treatment, support rescue/evacuation or choose whether available injured expertise is needed before recovery. |
| Complexity and AI | Simple presets delegate triage under priorities; deeper controls expose consequential patient/support allocation. Same evidence/capacity rules apply to AI and humans. |
| Interactions and communication | Operations/personnel supplies casualty and deployment needs, Science supplies relevant findings and Engineering supports medical capability. Routine treatment does not require Helm to stop manoeuvring. |
| Risks | Medical has only idle timers, injuries create repetitive status management, every hit rolls death or simulated casualty removes a human from useful play. |
| Dependencies | G061/G068–G071/G110, S043/S079/S080/S082–S088/S100. Exact states, treatment duration/materials, casualty checks and persistence remain open. No resurrection/clone mechanic is selected. |

### Resolved choice: can a mildly injured officer remain on duty?

| Option | Form | Trade-off |
|---|---|---|
| A — Injuries block assignment | Injured officers are unavailable until supported recovery restores them. | Simple status handling, but every injury removes expertise entirely. |
| B — Coarse severity distinction | Minor injuries allow duty with reduced effectiveness; severe injuries make the officer unavailable. | Creates triage and expertise choices without requiring detailed bodily simulation. |

**Selected: B.** Minor injuries permit duty at reduced effectiveness; severe injuries make the officer unavailable. Use a coarse readable severity distinction, with ordinary Medical AI executing supported treatment within actual capacity/evidence. This choice does not add automatic deterioration, repeated casualty rolls or routine manoeuvre restrictions. G071's explicit lethal circumstances and useful human participation remain. Exact conditions, treatment, checks and persistence remain open.

## S082 — Internal defence / boarding support

**Inclusion recommendation: retain located team objectives on the shared ship map, and resolve encounter granularity.** [Plan C7](../gdd/mechanics/duty-teams-and-operations.md#boarding-and-defence) makes boarding a supported team operation with objectives such as disabling machinery, rescuing personnel, recovering cargo/intelligence or capture. G070 already selects located teams and System-marked ship maps shared with repair, with automatic routing and legitimate observation of hostiles. G069 retains exact check percentages and contributing factors for supported mission checks.

Located teams need not create a tactical combat game inside the bridge simulator. Encounters can resolve at whole-team/task scale while players choose objectives and destinations, or expose more detailed tactical movement/engagement commands. Ship combat, support and personnel recovery continue either way. This choice does not settle capture authority, deployment eligibility or exact combat formulas.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Finite typed teams move to supported System/area objectives and contest local work. Compare coarse team encounters with detailed tactical team commands. |
| Ongoing activity | Watch legitimate incursion reports, team commitments and threatened capability; ordinary AI executes supported movement/work. |
| Active decisions | Defend a useful location, divert a team, choose a boarding objective, supply support or recover personnel. |
| Complexity and AI | Whole-System ordinary AI can manage delegated defence/boarding under priorities. No first-person control, individual ensign simulation or compulsory route clicking. |
| Interactions and communication | Security explains threatened areas, affected operators report consequences, repair competes for access/resources and Operations coordinates deployment/recovery. Enemy locations remain evidence-limited. |
| Risks | Boarding becomes a separate RTS, map possession reveals all enemies, abstract timers ignore team location or combat removes players from their bridge tasks. |
| Dependencies | G037/G061/G068–G071/G102/G109, S029/S043/S075/S079–S081 and deployment entries. Exact encounters, checks, access, observation, capture and defence orders remain open. |

### Resolved choice: how detailed should internal team combat controls be?

| Option | Form | Trade-off |
|---|---|---|
| A — Whole-team objectives and encounters | Order teams to protect, reach or act on a location/System; resolve local clashes through supported team-level rules/checks. | Keeps attention on bridge-scale allocation and support. |
| B — Detailed tactical commands | Also expose finer team movement/engagement manoeuvres inside those locations. | Adds tactical mastery, but risks a separate combat-management activity and heavier UI. |

**Selected: A.** Players assign whole teams to protect, reach or act on supported locations/Systems; local clashes resolve through team-level rules/checks. Teams route and execute their work automatically, without fine tactical manoeuvre or individual control. Legitimate observations, actual access, personnel commitments and bridge support provide the depth. AD56 selects whole-ship operational takeover after supported capture requirements are met, retaining individual System controllers. Exact encounters, checks, capture requirements and deployment eligibility remain open.

## S083 — Tractor beam / tow and stabilise

**Inclusion recommendation: retain ordinary crew-owned towing/stabilisation, and resolve use against resisting targets.** [The Current baseline](../gdd/mechanics/external-operations.md) has a first-class tractor System with actual power/availability/targeting gates and target-authored coupling responses for towing, stabilisation and formation work. G061 distinguishes releasing a tractor from recovering deployed personnel/equipment: no intrinsic recovery penalty is needed. The target's independent circumstances can still make releasing it consequential.

S018 already selects reuse of relevant weapon geometry—arcs, ranges, turret traversal—for equipment such as tractor beams. Whole-System ordinary AI can engage an approved mission target when eligible; human Helm can remain responsible for steering. The unresolved extension is whether suitable tractors can also influence a resisting vessel, making geometry, propulsion and support relevant in combat, or remain restricted to external service work. No blanket shield, mass or immunity rule is selected by asking this question.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Equipment-supported tow/stabilise coupling with actual geometry/resources. Compare service-only use with optional contested use against resisting targets. |
| Ongoing activity | Maintain viable coupling, reserves and useful movement; ordinary AI handles delegated tracking/execution. |
| Active decisions | Choose a target, engage/release, coordinate a tow or contest a target's movement if equipment supports it. |
| Complexity and AI | Ordinary individual tractor AI operates the whole System with normal knowledge/authority. No required beam-steering minigame or separate combat controller. |
| Interactions and communication | Helm handles geometry, Engineering supplies power/cooling and relevant operators explain rescue, boarding or combat needs. A tractor command does not operate the target's other Systems. |
| Risks | Free immobilisation, infinite tow capacity, arbitrary safe-release penalties, AI following hidden targets or a service System becoming compulsory combat equipment. |
| Dependencies | G027/G032/G053/G061/G105/G114, S018/S031–S038/S044–S053/S082. Exact coupling/force, resistance, shields, mass limits, range and interruption behaviour remain open. |

### Resolved choice: can suitable tractors act against resisting ships?

| Option | Form | Trade-off |
|---|---|---|
| A — External service only | Use tractors for supported towing, rescue, stabilisation and cooperative/disabled-target work. | Keeps the role clear and avoids another combat control effect. |
| B — Optional contested use | Suitable equipment can influence resisting targets under actual operating limits and counterplay. | Adds positioning, escape and boarding-support choices, but must avoid guaranteed immobilisation. |

**Selected: B.** Suitable tractor equipment can influence resisting ships under actual geometry, resources, operating limits and counterplay. No guaranteed immobilisation or universal fit is selected. Ordinary individual tractor AI handles delegated operation using available evidence. G061's release rule remains: tractors have no intrinsic deployed crew/equipment recovery penalty. Exact force/coupling, resistance, shields, mass limits and interruptions remain open.

## S084 — Umbilical / capacity transfer

**Inclusion recommendation: retain actual docked resource transfer; resolve concurrent supply scope.** [The Current baseline](../gdd/mechanics/external-operations.md#target-side-meaning) gives an umbilical one named capacity shared by both endpoints, transferred at an authored rate bounded by source stock and recipient headroom. Helm establishes the dock and Engineering operates the transfer; power, availability and separation govern interruptions. What already moved stays moved. This is a first-class System, not a generic completion timer.

G065/G067 select automatic replenishment against manually chosen stock targets and actual port/trade conditions; S049 makes physical undocking a modest commitment. An expanded umbilical could retain one active resource per connection or support concurrent compatible transfers under finite equipment throughput and priorities. Routine automation should not require a player to manually switch hoses after each stock completes. Neither option bypasses compatibility, actual stocks or permission to transfer.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Actual compatible stock/capacity movement over a viable physical connection. Compare sequential single-resource work with equipment-supported concurrent transfers. |
| Ongoing activity | Maintain viable connection/resources and watch remaining stocks or changed needs; ordinary AI executes authorised replenishment. |
| Active decisions | Choose recipient, allocation/reserve targets or priority; interrupt or depart when another need outweighs further supply. |
| Complexity and AI | Simple presets use target stocks/priorities; deeper controls can expose supported transfers. Ordinary individual transfer AI handles scheduling without manual per-item movement. |
| Interactions and communication | Helm coordinates connection/departure, stores explains demand/reserves, Engineering supplies operation and Comms handles access/trade terms. |
| Risks | Infinite transfer rate, every resource needs a manual restart, departure erases delivered stock or concurrent transfers introduce compulsory allocation sliders. |
| Dependencies | G038/G065/G067/G111, S043/S049/S063/S083 and later supply entries. Exact resource compatibility, throughput, transfer authority and replenishment scheduling remain open. |

### Resolved choice: can an umbilical transfer several compatible resources concurrently?

| Option | Form | Trade-off |
|---|---|---|
| A — One active resource | A connection transfers one compatible resource at a time; ordinary AI sequences authorised jobs. | Clear capacity use with fewer simultaneous flows. |
| B — Equipment-supported parallel transfer | Suitable connections transfer several resources under finite throughput/priorities; ordinary AI allocates the work. | Supports richer supply choices without sequential hose management, but needs readable bottlenecks. |

**Selected: B.** Suitable umbilical equipment supports concurrent compatible transfers governed by finite throughput and priorities. Ordinary transfer AI schedules authorised work with actual stocks/capacities and target-stock replenishment; detailed per-resource rate sliders are not required. Physical connection, power, availability and permission remain constraints, and delivered resources persist after interruption. Exact compatibility, throughput, authority and scheduling remain open.

## S085 — External repair / field team

**Inclusion recommendation: retain deployed repair personnel using the ordinary component/team model; resolve independence from the mothership.** [The Current baseline](../gdd/mechanics/external-operations.md) commits a real repair team and improves target condition while assignment and range hold. The redesign's G057/G058/S075–S077 instead provides actual component work, parts, one team per component and automatic fallback patches. External work should use the same supported rules on ships/facilities, rather than a separate generic healing meter.

G061 already makes deployed crew/equipment a recovery commitment. The open question is whether teams must always remain within mothership support range, or whether suitably equipped teams can continue after delivery while the mothership operates elsewhere. Independent work does not grant remote access to the mothership's stores, continuous reports through a broken link or automatic recovery.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Deploy actual teams/tools/parts for supported work on another entity. Compare mandatory nearby support with equipment-supported independent work. |
| Ongoing activity | Maintain local work/materials and monitor received reports, safety and recovery options; ordinary AI executes approved work. |
| Active decisions | Commit a scarce team, choose useful restoration, deliver supplies or return for recovery when other needs compete. |
| Complexity and AI | Ordinary team/repair/transport Systems retain individual AI and limited knowledge. Humans choose priorities without manually piloting workers or executing each repair step. |
| Interactions and communication | Target operators provide access/capability needs, transport delivers personnel/materials, Comms carries reports and Helm chooses whether to remain nearby or leave if independence is supported. |
| Risks | Range arbitrarily stops a physically equipped team, distant repairs consume unreachable stores, recovery becomes instant or disconnected reports reveal live progress. |
| Dependencies | G035/G037/G061/G064/G065/G070, S043/S075–S079/S084/S086–S088. Exact deployment, local stock access, support equipment, safety, authority and recovery remain open. |

### Resolved choice: can an equipped repair team work after the mothership leaves?

| Option | Form | Trade-off |
|---|---|---|
| A — Nearby support required | External repair requires the mothership to remain within the operation's support range. | Keeps support/recovery concentrated, but ties up the bridge during extended work. |
| B — Equipment-supported independence | Delivered teams can continue using available local tools/parts without nearby mothership support, where equipped. | Enables parallel operations and return/recovery decisions, but needs real supplies and report limitations. |

**Selected: B.** Delivered equipped teams can continue approved work using available local tools/parts after the mothership leaves. Tethered or remotely powered equipment can still require nearby support. Teams use ordinary component/task rules and remain committed until physically recovered. They cannot consume unreachable mothership stores or provide live reports through unavailable communication. Exact deployment, supplies, support equipment, authority and recovery remain open.

## S086 — Shuttle / carried utility craft

**Inclusion recommendation: retain physical AI-piloted utility craft and resolve whether transport stays committed after delivery.** [Plan C7](../gdd/mechanics/duty-teams-and-operations.md#deployment-methods) supplies physical interceptable carried craft for transport, reconnaissance, rescue and specialist work. S017 already selects sortie management rather than human piloting; G027 preserves full-3D small craft. S019's ordinary ship/System composition applies to craft, with real equipment, endurance and communication. S101 later reviews control/telemetry rather than duplicating the craft model here.

S085 now lets suitably equipped deployed teams work independently. Their transport could remain tied to the mission until recovery or become available after delivery when its continued presence is unnecessary. Separating team and transport commitments allows ferry work and return-for-extraction decisions; it does not guarantee a free craft, safe pickup or unlimited Operations capacity. Tasks that actually need onboard tools, shelter or immediate extraction can still require the shuttle to stay.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Physical compatible utility craft, actual cargo/personnel capacity and AI flight. Compare mission-long transport reservation with task-dependent release after delivery. |
| Ongoing activity | Monitor craft endurance, commitments and received reports; ordinary individual AI handles approved flight and equipment operation. |
| Active decisions | Choose transport/team/load, deliver support, redirect an available craft or reserve it for recovery. |
| Complexity and AI | Simple presets delegate sortie execution; deeper presets expose useful mission/load priorities. No human piloting requirement or separate craft physics/AI architecture. |
| Interactions and communication | Operations coordinates deployment/recovery, stores provides cargo, Medical prioritises evacuation and Helm/defence supports launch/arrival geometry and protection. |
| Risks | Every delivery locks a craft into idle waiting, craft is magically freed while still needed, team/craft capacity is double-booked or recovery ignores travel and danger. |
| Dependencies | G027/G061/G065/G068–G071, S017/S019/S049/S079–S085/S088/S100/S101. Exact craft/load compatibility, bay servicing, transport capacity and sortie/task authority remain open. |

### Resolved choice: can a shuttle take another job after delivering an independent team?

| Option | Form | Trade-off |
|---|---|---|
| A — Reserved until mission recovery | The assigned transport remains committed until its team's operation ends and they return. | Guarantees a reserved recovery resource, but can leave useful craft waiting. |
| B — Task-dependent transport commitment | After delivery, release the craft for other work if the team/task does not need it on site; plan a later pickup. | Enables ferry/resupply choices, but recovery competes with other commitments and actual travel. |

**Selected: B.** A shuttle can take another job after delivering an independent team when the task does not need it on site. Tasks requiring its tools, shelter or immediate extraction keep it committed. Team and craft availability remain separate; later pickup is a real transport/recovery task subject to actual travel, capacity and risk. No double booking or automatic recovery.

## S087 — Transporter / remote personnel deployment

**Inclusion recommendation: retain an optional fitted alternative to physical transport; resolve destination requirements.** [Plan C7](../gdd/mechanics/duty-teams-and-operations.md#deployment-methods) proposes faster limited-range deployment/recovery without an interceptable craft. Shields, interference, damage and destination suitability can block it. It does not specify a universal receiving pad or beacon. This is planned capability, not a claim about implemented transporters.

S085/S086 make independent teams and return transport meaningful commitments. Transporters should provide a different operational opportunity without erasing recovery planning or the usefulness of shuttles. Requiring a receiver would make infrastructure preparation central; allowing a sufficiently established suitable destination would support rescue and boarding where no receiver exists. Neither option grants hidden destination knowledge or bypasses actual operating constraints.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional fitted personnel/equipment transport within supported capacity, range and conditions. Compare universal receiver requirements with evidence-supported suitable destinations. |
| Ongoing activity | Maintain readiness/resources and destination evidence; ordinary individual AI handles delegated operation. |
| Active decisions | Choose deployment/extraction, establish a usable destination and coordinate a window when shields/interference or geometry permit. |
| Complexity and AI | Simple presets expose target and feasibility; deeper presets expose useful priorities and constraints. No compulsory manual targeting puzzle. |
| Interactions and communication | Sensors establishes destination evidence, transport operators execute, Engineering supplies resources and teams/Helm coordinate recovery opportunities. |
| Risks | Magical extraction, unknown destinations become visible, transporter makes shuttles obsolete or a universal pad requirement excludes rescue without a deliberate design reason. |
| Dependencies | G061/G069/G070/G105/G109, S019/S056–S064/S079–S086/S088. Exact capacity, destination evidence, shield interaction, interference and interruption behaviour remain open. |

### Resolved choice: must every transporter destination have a receiving pad or beacon?

| Option | Form | Trade-off |
|---|---|---|
| A — Receiver required | Deployment and extraction require compatible receiving infrastructure or a deployed beacon. | Makes infrastructure preparation valuable, but restricts unprepared rescue and boarding. |
| B — Suitable established destination | No universal receiver requirement; supported equipment can use a sufficiently established suitable destination within operating limits. | Supports more rescue/boarding opportunities, but needs readable evidence and operating constraints to preserve transport choices. |

**Selected: B.** No universal receiving pad/beacon requirement. Supported equipment can transport to a sufficiently established suitable destination using available evidence within actual operating limits. Range, capacity, shields, interference, damage and suitability can constrain deployment/extraction. No unknown interior knowledge or guaranteed recovery is granted. Exact evidence requirements and equipment limits remain open.

## S088 — Away Mission Operations systems

**Inclusion recommendation: retain off-screen team operations, including low-key assignments; resolve their routine attention demand.** [Plan C7](../gdd/mechanics/duty-teams-and-operations.md#away-missions) describes timed phases, checks and decision events, with bridge actions modifying exact check odds. G069 already selects exact percentages and contributing factors. The [STO Duty Officer research](./research/README.md) supports examining assignment, personnel commitment and later outcomes as a quieter activity; it does not require every assignment to become an interactive story sequence.

The proposal already supplies real teams, optional officer bonuses, physical transport, local supplies and recovery. An operation can therefore create decisions through those commitments even without a mandatory mid-mission prompt. Selected missions can still offer consequential branches or require bridge support. The question here is whether every away mission must include operator decision stops, or whether routine jobs can proceed under approved instructions with interventions only when warranted.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Off-screen operations using committed teams, equipment and compatible Operations Systems. Preserve exact check odds and ordinary world consequences. |
| Ongoing activity | Track commitments, received reports, supplies and recovery opportunities; delegated Systems execute within their knowledge and authority. |
| Active decisions | Choose mission/team/load, obtain useful scans or access, supply support, respond to meaningful branches and recover personnel. |
| Complexity and AI | Authored Station presets expose suitable planning depth. Whole-System AI handles routine execution under existing authority; selected protected choices retain their boundary. |
| Interactions and communication | Operations requests relevant personnel, transport, evidence, protection or supplies. Communication follows actual dependencies rather than a compulsory prompt quota. |
| Risks | Idle real-time timers, every assignment demands acknowledgements, hidden changes invalidate displayed odds, mandatory support distracts the whole bridge or recovery becomes automatic. |
| Dependencies | G008/G009/G061/G068–G071/G079/G110/G111, S062/S075–S087. Exact check knowledge, phase structure, assignment authority and authored support remain open. |

### Resolved choice: should every away mission require mid-mission operator decisions?

| Option | Form | Trade-off |
|---|---|---|
| A — Decision sequence for every mission | Every away mission includes staged operator choices during execution. | Guarantees attention points, but risks repetitive prompts and distraction during bridge crises. |
| B — Routine autonomy, selected interventions | Routine jobs proceed under approved instructions; meaningful changes or selected mission content introduce decisions and requests for support. | Supports low-key work alongside major operations, but requires missions whose initial commitments and consequences are worthwhile. |

**Selected: B.** Routine assignments proceed under approved instructions. Meaningful developments or selected mission content introduce decisions and requests for bridge support; every mission need not contain staged operator prompts. Assignment, preparation, support and physical recovery remain substantive commitments. Ordinary individual System AI, limited knowledge, exact check presentation and protected-choice boundaries remain applicable.

## S089 — Cargo hold / bulk stores / hazardous containment

**Inclusion recommendation: retain main stores, meaningful consignments and automatic internal replenishment; resolve storage location granularity.** [The resource baseline](../gdd/mechanics/power-resource-network.md) plans named bulk amounts in C4 and later discrete cargo lots without an inventory slot grid. Current external capacity transfers are narrower functionality. G065/G066 already select main hold stores, manually set local stock targets, timed automatic replenishment, aggregated routine supplies and consignments with decision-relevant properties. [The comparable-games research](./research/comparable-games-and-falling-frontier.md) offers physical supply and interception as sources of decisions; it does not require manual crate arrangement.

S039/S040/S043 already use coarse physical infrastructure, compartments and reachable delivery paths. Cargo could remain a single ship-wide capacity pool, or occupy authored storage compartments with relevant compatibility and containment. The latter could make a damaged hold, unsafe route or specialised containment meaningful without tracking individual boxes.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Aggregated routine stocks plus meaningful consignments; compare one pooled hold with coarse authored storage compartments. No inventory grid or individual crate packing. |
| Ongoing activity | Maintain viable reserves, storage and replenishment routes; ordinary inventory AI handles compatible placement and delivery. |
| Active decisions | Choose cargo/load priorities, target stocks, hazardous cargo acceptance, transfer or disposal and responses to damaged storage. |
| Complexity and AI | Simple presets expose stock targets/capacity problems; deeper presets can expose useful storage priorities. Routine loading/replenishment should not require per-item instructions. |
| Interactions and communication | Stores coordinates supply needs, Engineering restores access/containment and relevant specialists assess hazardous consignments. Transport and Comms support actual delivery/access. |
| Risks | Inventory Tetris, mandatory healthy-cargo maintenance, arbitrary loss of every stock from one hit or pooled stores ignoring physically blocked routes. |
| Dependencies | G035/G057/G065–G067/G070/G108/G111, S039/S040/S043/S084/S090–S092. Exact capacities, compatibility, damage exposure, hazard rules and placement priorities remain open. |

### Resolved choice: should cargo occupy particular storage compartments?

| Option | Form | Trade-off |
|---|---|---|
| A — One pooled hold | Ship stores share one capacity pool; hazards and containment are managed at hold level. | Simplifies loading, but gives storage location and local damage less meaning. |
| B — Coarse authored storage | Stocks/consignments occupy compatible authored storage compartments; ordinary AI places and moves them where supported. | Makes containment, access and local damage matter, but needs clear capacity feedback and automatic routine handling. |

**Selected: B.** Aggregated routine stocks and meaningful consignments occupy compatible authored storage compartments. Ordinary inventory AI handles routine placement and supported movement. Storage capacity, containment, local damage and reachable delivery routes matter; players choose loads and priorities without arranging individual crates. Exact capacities, compatibility, hazard effects and placement priorities remain open.

## S090 — Salvage / recovery equipment

**Inclusion recommendation: retain recovery of actual surviving cargo/components; resolve onboard scrap processing.** G046 already selects useful surviving cargo, components and appropriate materials. S042 allows deliberate cannibalisation of compatible onboard equipment, with actual donor loss and work. [The Current external-operations baseline](../gdd/mechanics/external-operations.md) supplies towing, transfer and field repair foundations, not a generic salvage/recycling implementation. [The comparable-games research](./research/comparable-games-and-falling-frontier.md) identifies Avorion's wreck scavenging as an activity reference while cautioning against repetitive harvesting.

The remaining distinction is between recovering usable objects/material stocks and processing otherwise unusable wreck material onboard. Optional fitted processing could extend operational endurance, but must retain actual compatible outputs and finite throughput rather than turning every wreck into any desired ammunition or repair part. S014's selected recipe overlap does not make all finished parts interchangeable.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Recover what actually survives with suitable access/tools. Compare recovery-only equipment with optional fitted scrap processing. |
| Ongoing activity | Maintain a viable work site, available storage and equipment resources; ordinary individual AI executes the approved recovery/processing job. |
| Active decisions | Choose valuable recoverable items, commit teams/craft, protect work, prioritise limited capacity or abandon recovery when circumstances change. |
| Complexity and AI | Simple presets expose useful recovery goals; deeper presets can expose item/material priorities. No manual cutting or repeated collection clicks required. |
| Interactions and communication | Sensors establishes useful wreck evidence, Helm positions, transport/teams recover and stores/Engineering judge needs and compatibility. |
| Risks | Endless mandatory wreck harvesting, destroyed equipment yields intact replacements, universal material conversion or salvage reveals unobserved contents. |
| Dependencies | G046/G057/G061/G064–G066/G109/G111, S014/S019/S042/S075–S089. Exact tools, extraction work, yields, recovered condition, recipes and ownership remain open. |

### Resolved choice: should suitable ships process wreck scrap onboard?

| Option | Form | Trade-off |
|---|---|---|
| A — Recovery only | Recover surviving cargo, usable components and existing material stocks; further scrap processing needs a suitable facility. | Keeps onboard salvage focused and gives ports a distinct role. |
| B — Optional fitted processing | Suitable ships can also process compatible scrap into specified material outputs, using finite capacity, time and resources. | Creates endurance and salvage-fit choices, but must avoid universal conversion or compulsory grinding. |

**Selected: B.** Optional fitted equipment processes compatible wreck scrap into specified material outputs with finite capacity, time and resources. This supplements recovery of actual surviving cargo/components/materials and provides a situational alternative to resupply. Outputs feed supported recipes rather than automatically becoming any missing finished part. Ordinary individual AI executes approved work. Exact tools, yields, recovered condition and recipes remain open.

## S091 — Mining / harvesting tools

**Inclusion recommendation: consider optional fitted extraction as a non-combat operation; resolve whether it exists outside specific mission content.** The catalogue draws on historical R2 EVE/Elite/Endless Sky research. [The expanded comparable-games research](./research/comparable-games-and-falling-frontier.md) also records Avorion's mining/scavenging and EVE's resource activities, while identifying repetitive harvesting and MMO labour as poor defaults for a bridge session. These are research references, not Current Phoenix capabilities.

G088 already supports self-directed Sandbox projects. S014/S042/S090 establish actual compatible material inputs and finite processing; mining could supply those inputs where deposits and equipment permit. That does not require every ship to mine or every scenario to contain extraction. A narrower alternative is to provide extraction only for authored or generated mission objectives, without a general harvesting capability.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Compare mission-specific extraction with optional fitted harvesting of suitable actual deposits. No universal conversion of scenery into useful stock. |
| Ongoing activity | Maintain viable position, energy/thermal margins and storage; ordinary extraction AI handles delegated work within limits. |
| Active decisions | Prospect/select a site, choose material and extraction priorities, commit time or craft and decide when to leave as needs or risks change. |
| Complexity and AI | Presets expose appropriate site/stock priorities; routine extraction need not require repeated aiming or collection clicks. |
| Interactions and communication | Sensors supplies deposit evidence, Engineering/stores explains demand and limits, Helm establishes access and other players assess security or alternative supply. |
| Risks | Mandatory resupply grind, idle waiting for a full hold, identical deposits, extraction crowds out other players or every hull gains indefinite self-sufficiency. |
| Dependencies | G008/G009/G038/G046/G065/G088/G111, S014/S019/S031–S038/S042/S089/S090/S092. Exact deposits, prospecting evidence, tools, extraction rates and processing compatibility remain open. |

### Resolved choice: should mining work independently of a specific mission?

| Option | Form | Trade-off |
|---|---|---|
| A — Mission-specific extraction | Mining/harvesting occurs where authored or generated mission content calls for it. | Keeps activity focused, but limits self-directed supply and exploration choices. |
| B — Optional general capability | Equipped ships can harvest suitable deposits without accepting a mission first; scenarios decide available deposits and needs. | Supports self-directed projects and alternative supply, but must avoid repetitive or compulsory harvesting. |

**Selected: B, with an explicit scope limit.** Equipped ships can harvest suitable deposits without accepting a specific mission first; scenarios determine availability and ordinary AI handles routine work. Mining remains a supporting activity and must not turn Phoenix into a mining game. Avoid compulsory resource grind or an expanding extraction/production loop as the default bridge experience. Prospecting, site choice and operating exposure can provide occasional useful decisions alongside alternative supply. Exact tools, deposits, rates and processing compatibility remain open.

## S092 — Supply / logistics drones

**Inclusion recommendation: retain optional physical delivery craft using the ordinary entity/System model; resolve repeat-delivery authority.** [The comparable-games research](./research/comparable-games-and-falling-frontier.md) draws on Carrier Command 2, Foxhole and Falling Frontier for protected supply and automated routing. G108 already requires physical vessels for active-sector deliveries. S019 applies the ordinary ship model to deployed equipment; S086 provides task-dependent craft commitments. Supply drones therefore need no special physics or AI architecture.

G065 already automates internal replenishment against chosen stock targets. External deliveries could require a fresh dispatch for every sortie, or fulfil an approved standing supply order across multiple sorties. A standing order would authorise a recipient, resource goals and reserve limits, not grant unrestricted authority to supply anyone. Physical cargo, travel, capacity, viable transfer and received reports remain relevant. Datalink behaviour is reviewed at S101.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional cargo-carrying craft with actual stocks, capacity, equipment and ordinary individual System AI. Compare single-sortie dispatch with approved repeat-delivery orders. |
| Ongoing activity | Monitor supply coverage, reserves, committed craft and received progress; delegated AI handles approved routing/loading/delivery. |
| Active decisions | Choose recipient, stock goal, reserves and priority; protect, redirect, suspend or abandon a delivery when circumstances change. |
| Complexity and AI | Simple presets can expose recipient/goal/reserve controls; deeper presets expose useful priorities. No manual flying or compulsory dispatch clicks for every load if standing orders are selected. |
| Interactions and communication | Recipient communicates need, stores defines available stock, craft supplies it and Helm/defence supports safe access. Supply permission is distinct from control of recipient Systems. |
| Risks | Infinite supply, hidden reserve depletion, automatic gifts without authority, double-booked craft, live remote knowledge through lost links or repeated dispatch chores. |
| Dependencies | G038/G061/G065/G067/G108/G109/G111, S019/S043/S084/S086/S089/S101. Exact loading/transfer compatibility, reserve controls, route authority and blocked-job behaviour remain open. |

### Resolved choice: can supply drones fulfil standing delivery orders?

| Option | Form | Trade-off |
|---|---|---|
| A — Dispatch each sortie | The operator approves every delivery trip separately. | Gives direct control over each commitment, but repeats instructions for routine supply. |
| B — Approved standing orders | Authorise a recipient, supply goal and reserve limits; ordinary AI repeats viable deliveries until satisfied, suspended or blocked. | Lets players manage supply priorities rather than trips, but needs clear commitments and reserve feedback. |

**Selected: B.** An approved recipient, supply goal and reserve limits authorise repeated viable deliveries until satisfied, suspended or blocked. Ordinary individual AI handles execution while actual craft, cargo, capacity, travel and transfer conditions remain necessary. This does not grant unrestricted gift authority, control recipient Systems or reveal remote state through unavailable communication. Exact reserve controls, compatibility and blocked-job behaviour remain open.

## S093 — Construction / fabrication / repair drones

**Inclusion recommendation: consider optional physical work craft, but resolve construction scope before adding an infrastructure loop.** The catalogue's historical R2 references include FTL/Reassembly. [The expanded research](./research/comparable-games-and-falling-frontier.md) retains free construction as a boundary rather than an implied Phoenix requirement. S019 already supplies ordinary composed deployable entities; S075–S085 supply component repair, actual parts/access and physical field work. Drone equipment could participate in supported work without becoming a new universal repair model.

The unresolved scope is whether drones only deploy carried equipment and perform supported repairs, or can also construct new predefined entities from materials in the field. Neither choice introduces freeform hull/interior construction. S091's explicit limit against becoming a mining game also argues for considering the cumulative extraction, processing and construction loop rather than accepting each capability in isolation.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional physical work craft with actual equipment/resources. Compare carried deployables and supported repair with additional blueprint-based field construction. |
| Ongoing activity | Maintain work access, available materials and craft commitments; ordinary individual AI executes approved jobs. |
| Active decisions | Choose useful repairs/deployment sites, commit equipment, prioritise scarce capacity and recover or abandon craft as conditions change. |
| Complexity and AI | Humans choose work goals; AI executes supported steps. No manual drone piloting, per-piece assembly or compulsory new Station. |
| Interactions and communication | Sensors/Helm establish viable sites/access, Engineering assesses restoration, stores supplies materials and defence protects vulnerable work. |
| Risks | Drone repairs bypass component rules, infinite deployable production, an expanding base-building economy, mandatory mining or one operator managing a separate RTS. |
| Dependencies | G057/G061/G064/G072/G075/G111, S019/S042/S075–S085/S089–S092/S101. Exact drone work compatibility, personnel relationship, deployment effort and resources remain open. |

### Resolved choice: should drones construct new entities from materials in the field?

| Option | Form | Trade-off |
|---|---|---|
| A — Repair and carried equipment | Drones perform supported repairs and deploy carried equipment; no general field construction of new entities. | Supports bridge operations while keeping the production/infrastructure loop bounded. |
| B — Limited blueprint construction | Suitable drones can also build predefined entities from actual materials, taking time and capacity. | Adds engineering projects and infrastructure choices, but expands the production loop and balancing scope. |

**Selected: A.** Work drones perform supported repairs and deploy carried equipment. General field construction of new entities from materials is outside the baseline; specific construction missions can still be considered separately as content. Physical craft use ordinary entity/System rules with actual equipment, parts, access and commitments. Exact repair compatibility, personnel relationship and deployment effort remain open.

## S094 — Remote shield / energy / repair support emitters

**Inclusion recommendation: consider optional fitted energy/shield support; resolve whether emitters also perform physical repairs.** Historical R2 EVE logistics is the catalogue's reference for specialised support ships. This is a candidate transfer, not Current Phoenix functionality or a commitment to copy EVE mechanics. S018 already suggests reusing relevant weapon geometry/traversal for service equipment; S022–S024 and S031–S038 supply local battery, shield and thermal constraints. S084 separately handles physical connected supply.

Remote support can couple one ship's available resources and positioning to another's endurance. The unresolved boundary is physical repair: S075–S085/S093 already provide component work through teams and equipped craft with parts/access. Adding a repair emitter would require an explicit alternative physical mechanism and compatible work, rather than restoring a generic hull percentage or bypassing destroyed components.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional fitted support using actual range/geometry, source resources and recipient compatibility. Compare energy/shield assistance only with additional remote component-repair equipment. |
| Ongoing activity | Sustain useful support within source reserves, heat and positioning limits; ordinary individual support AI operates an approved target. |
| Active decisions | Choose beneficiary, priority and reserve commitment; move or stop supporting when exposure or competing needs change. |
| Complexity and AI | Presets can expose target/priority/reserve settings. Delegation avoids compulsory beam tracking or repetitive activation. |
| Interactions and communication | Recipient reports need, Engineering judges reserve/thermal cost, Helm maintains viable geometry and defence protects the support ship. No remote command of recipient Systems. |
| Risks | Free energy, permanent support loops, mandatory healer ship, invisible range loss or repair beams bypassing parts, access and component conditions. |
| Dependencies | G057/G058/G063/G111, S018/S022–S024/S031–S038/S075–S085/S093. Exact transfer mechanisms, efficiency, recipient acceptance, geometry and limits remain open. |

### Resolved choice: should remote support emitters repair physical components too?

| Option | Form | Trade-off |
|---|---|---|
| A — Energy/shield support only | Suitable emitters provide energy or shield assistance; physical repairs use teams and equipped work craft. | Preserves distinct support methods and the established component-repair model. |
| B — Include specialised repair emitters | Suitable equipment also performs specified remote component repairs, with explicit material, access and operating limits. | Adds support-fit variety, but requires another repair mechanism and clear limits against bypassing existing work. |

**Selected: A.** Optional fitted emitters provide energy/shield assistance, using actual source resources, recipient compatibility and operating geometry. Physical repair emitters are excluded; teams and equipped work craft retain component restoration. Ordinary individual AI operates approved support. Exact transfer mechanisms, efficiency, acceptance and limits remain open.

## S095 — Repair puzzles and reactivation workflows

**Resolved from G103/S075–S077; no repeat question.** The historical R1 puzzle candidate is superseded by operational interpretation without a separate authored code/pattern/manual puzzle layer. Diagnosis uses available evidence; teams execute supported restoration with actual access, parts and commitments. Automatic fallback patching remains S077. Do not add a compulsory manual reactivation task after every repair. Selected meaningful System procedures can still exist under previously agreed rules.

Players decide what to investigate, restore and resource, and communicate actual capability/support needs. Ordinary individual AI executes delegated work without hidden fault knowledge. This retains complexity through repair priorities and dependencies rather than repeated shape matching or transcription.

## S096 — Fire-control radar / illuminator

**Inclusion recommendation: consider optional specialised engagement hardware; resolve whether it is separate from ordinary sensing.** [The NEBULOUS research](./research/comparable-games-and-falling-frontier.md) distinguishes detection from fire-control/illumination and identifies terrain, electronic warfare and guidance as interacting capabilities. This is a research transfer rather than Current Phoenix behaviour. G105 already rejects a universal deep-scan prerequisite for ordinary weapon use; S057/S058 separately retain broad and focused sensing.

Specialised tracking/illumination could support particular weapon or guidance types with finite equipment capacity. It need not gate every gun or require a human to manually maintain each lock. Alternatively, engagement solutions could remain part of ordinary sensors and individual weapon Systems without another fitted System. Either approach preserves evidence limits and equipment-specific targeting rules.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Integrated ordinary tracking or optional fitted fire-control/illumination for supported equipment; no universal requirement. |
| Ongoing activity | Maintain useful evidence and viable geometry; ordinary individual AI maintains approved solutions within hardware limits. |
| Active decisions | Allocate scarce tracking/illumination capacity, choose exposure and switch priorities when a solution becomes blocked. |
| Complexity and AI | Simple presets delegate allocation against approved priorities; deeper presets expose useful assignments. No mandatory manual lock upkeep or dedicated Station selected. |
| Interactions and communication | Sensors reports evidence/solution limits, weapons specifies guidance needs, Helm adjusts geometry and Engineering supplies resources. Final ownership follows Station composition later. |
| Risks | Every shot needs another player's permission, redundant sensor chores, hidden caps, guaranteed solutions or hardware without meaningful trade-offs. |
| Dependencies | G019–G021/G049/G105, S003/S015/S018/S048/S055–S060/S099. Exact guidance, capacity, emissions, costs and solution-loss behaviour remain open. |

### Resolved choice: should specialised fire-control/illumination be separately fitted?

| Option | Form | Trade-off |
|---|---|---|
| A — Integrated tracking | Ordinary sensors and weapon Systems provide supported engagement solutions; no separate fire-control System. | Keeps the equipment model simpler with fewer allocation demands. |
| B — Optional specialised hardware | Particular weapons/guidance can use separately fitted tracking/illumination with finite capacity and operating constraints. | Adds equipment and coordination choices, but must earn its complexity and avoid gating every weapon. |

**Selected: B.** Optional separately fitted tracking/illumination supports particular weapons or guidance types with finite capacity and operating constraints. It is not a universal weapon-use prerequisite. Ordinary individual AI maintains approved solutions; humans choose priorities, exposure and geometry where these matter. Exact guidance, capacity, emissions, costs and solution-loss behaviour remain open.

## S097 — Bearing-only receivers / ELINT / passive thermal search

**Inclusion recommendation: retain passive spatial evidence under S060; resolve whether every detection grants a position estimate.** [The comparable-games research](./research/comparable-games-and-falling-frontier.md) uses HighFleet/Wolfpack and Falling Frontier's advertised signatures as references for detecting emissions and deciding whether to expose oneself for better information. S060 already selects independent receivers; S061 allows compatible communicated observations from ships/probes to improve localisation. This entry concerns the evidence they supply, not another compulsory receiver System.

G018 separates confidence in position, identity and condition. A receiver could always produce a rough located contact, or sometimes provide only direction/signature until geometry and further evidence establish range. The latter gives Helm movement, probe placement and selective active confirmation useful roles. It must not require manual triangulation arithmetic or force every passive contact through the same sequence; capable equipment and sufficient evidence can still establish position directly.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Passive observations with equipment-dependent evidence; compare always-located rough contacts with possible bearing/signature-only findings. |
| Ongoing activity | Monitor emission changes and evidence age; ordinary receiver/analysis AI correlates available observations. |
| Active decisions | Seek a useful baseline, deploy a probe, request shared observations or accept active-sensing exposure to resolve uncertainty. |
| Complexity and AI | Show direction/uncertainty readably; automate correlation and arithmetic. Deeper presets can expose evidence and investigation priorities. |
| Interactions and communication | Sensors explains what is known, Helm provides geometry, other ships/probes supply compatible reports and Tactical judges whether evidence supports engagement. |
| Risks | A bearing secretly grants exact range, perfect identity from any emission, every contact needs micromanagement or fusion uses unreceived/duplicated reports. |
| Dependencies | G018–G021/G096/G105/G109, S056–S064/S096/S099. Exact bearing display, sensitivity, evidence fusion, signal classification and ranging capability remain open. |

### Resolved choice: can a passive detection lack a usable range estimate?

| Option | Form | Trade-off |
|---|---|---|
| A — Always roughly located | Every detected passive contact supplies an approximate position, with uncertainty refined by further observations. | Keeps a consistent contact interface, but reduces the value of establishing range separately. |
| B — Evidence-dependent localisation | Some detections supply only a bearing/signature; range and position require sufficient equipment/evidence, potentially using movement, probes or active confirmation. | Creates reconnaissance choices, but needs readable partial contacts and automated correlation. |

**Selected: B.** Passive detections can supply only bearing/signature without usable range. Suitable equipment or sufficient evidence can establish position; movement, compatible communicated ship/probe observations and active confirmation can improve localisation. Ordinary AI handles correlation and arithmetic using available evidence. No mandatory investigation sequence applies to every contact. Exact display, sensitivity, classification and fusion remain open.

## S098 — Directional search array / reorienting reconnaissance suite

**Inclusion recommendation: treat directional search as optional sensor equipment geometry, not a third mandatory sensing workflow.** [The comparable-games research](./research/comparable-games-and-falling-frontier.md) proposes sector coverage and redirection as a ship-mounted adaptation of directional reconnaissance references. G020/S057/S058 retain independent broad and focused Systems. Directional equipment must not silently turn focused work into a drain on broad coverage or reintroduce contact-count micromanagement.

Sensors already use the shared turret/fire-arc/tracking code selected for equipment. Hull-fixed or steerable fields of view follow authored mounts and traversal limits. The interview unnecessarily presented this existing rule as a new decision; the user's clarification reaffirms its application to sensors. It does not imply every sensor is directional or steerable.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Optional directional variants of appropriate sensor equipment. Compare hull-fixed direction only with equipment-supported steerable arrays. |
| Ongoing activity | Maintain useful sector coverage and observe blind areas; ordinary individual sensor AI holds approved direction or search policy. |
| Active decisions | Choose a sector/search priority, redirect coverage or ask Helm to change geometry when mount limits/occlusion matter. |
| Complexity and AI | Simple presets delegate search against priorities; deeper presets expose useful sector controls. No repeated manual sweep clicks or extra permanent Station required. |
| Interactions and communication | Sensors explains coverage needs, Helm coordinates heading/position with tactical and service demands, Engineering supplies actual resources. |
| Risks | A redundant third sensor workflow, arbitrary delay timers, focused scans disable broad sensing, blind areas are hidden or steering every array becomes compulsory upkeep. |
| Dependencies | G020/G027/G105/G111, S018/S044–S046/S056–S060/S096/S097. Exact fields of view, sensitivity trade-offs, traversal rates, search patterns and costs remain open. |

### Resolved clarification: directional sensors use the common equipment model

| Option | Form | Trade-off |
|---|---|---|
| A — Hull-fixed only | Directional arrays face their mounted direction; Helm turns the ship to change coverage. | Creates direct heading coordination with simpler equipment controls. |
| B — Equipment-dependent steering | Some arrays are fixed; others steer within authored arcs and traversal limits, with ordinary AI handling approved search. | Supports varied sensor fits and competing geometry needs, but requires readable coverage and mount limits. |

**Selected: B, reaffirming an existing rule.** Sensors use the same turret/fire-arc/tracking code as weapons. Fixed or steerable behaviour follows equipment mounts, arcs and traversal limits; ordinary individual sensor AI handles approved search/tracking. No separate sensor steering model or new mandatory workflow is added. Broad and focused sensing retain independent resources. Exact fields of view, sensitivity and search patterns remain open.

## S099 — Target-motion / firing-solution computer

**Inclusion recommendation: retain automated evidence-based motion estimation; resolve whether it needs another fitted System.** [The Wolfpack research transfer](./research/comparable-games-and-falling-frontier.md) proposes useful uncertainty and observation choices while automating routine plotting/arithmetic for Phoenix. G018/G019 already require confidence and evidence-limited prediction; S061/S097 provide shared observations and bearing-only findings. S096 separately selects optional hardware for particular tracking/illumination needs. These choices do not yet require a separate computing module for ordinary estimates.

Estimation can be a standard function of relevant sensors/fire-control, constrained by their available evidence. Alternatively, a fitted solution-computer System could add its own capacity, power and failure conditions. Either must avoid hidden target motion, a compulsory manual plotting puzzle or treating confidence as guaranteed accuracy.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Automated position/motion estimates and supported firing predictions; compare integrated computation with an additional fitted System. |
| Ongoing activity | Update estimates from available observations and retain uncertainty/age; ordinary AI handles routine calculation. |
| Active decisions | Seek another observation or better geometry, choose whether uncertainty is acceptable and select engagement priorities. |
| Complexity and AI | Simple presets show useful estimates/uncertainty; deeper presets can expose contributing evidence. No compulsory arithmetic, plotting or additional Station. |
| Interactions and communication | Sensors explains evidence limits, Helm helps observation geometry and Tactical chooses whether/how to engage. |
| Risks | Another mandatory support module with little decision value, perfect unseen-motion prediction, manual plotting chores or duplicated confidence/illumination controls. |
| Dependencies | G018/G019/G049/G105/G109, S003/S048/S054/S055/S061/S096–S098. Exact estimator, prediction presentation and evidence fusion remain open. |

### Resolved choice: does motion estimation require its own fitted computer System?

| Option | Form | Trade-off |
|---|---|---|
| A — Integrated estimation | Relevant sensors/fire-control compute estimates from their available evidence without an additional fitted computer. | Keeps decisions about observations and engagement rather than another module to supply. |
| B — Separate fitted computer | A dedicated System computes supported motion/firing solutions with its own resources, capacity and failure conditions. | Adds fitting and redundancy choices, but increases support complexity and overlap. |

**Selected: A.** Relevant sensors/fire-control integrate automated evidence-based motion/firing estimation without an additional fitted computer System. Estimates preserve age and uncertainty and cannot use hidden target motion. Humans seek useful observations/geometry and decide whether evidence supports engagement. Exact estimator, presentation and fusion remain open.

## S100 — Escape-pod beacon, locator and recovery apparatus

**Inclusion recommendation: retain physical rescue/capture and useful survivor participation; resolve distress emissions.** [The Falling Frontier research](./research/comparable-games-and-falling-frontier.md) records advertised rescue/capture as an activity reference. G012 already selects useful post-loss play through escape pods and team management, with eligible vacant Stations another situational option. G071 preserves human participation despite simulated casualties. S019's ordinary entity/System composition applies to pods; recovery uses supported physical methods such as tractors, shuttles or transporters, not automatic respawning.

A pod's distress transmitter can always broadcast while functional, or support a choice to signal openly, use available directed/secure communication or remain quiet. The latter creates a rescue-versus-detection decision, but must obey actual communications equipment, knowledge and reach. It cannot make a silent pod undetectable or grant rescuers its position without evidence.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Physical survivor/pod entities and supported recovery equipment using ordinary rules. Compare compulsory distress emission with selectable signalling policy. |
| Ongoing activity | Track available support, observations and received rescue reports; ordinary individual AI operates delegated equipment. |
| Active decisions | Signal or conceal where supported, choose rescue priorities and recovery method, coordinate a pickup or defer under danger. |
| Complexity and AI | Survivor controls should offer useful choices without survival chores; AI can operate a selected signalling/recovery policy. No special omniscient rescue controller. |
| Interactions and communication | Survivors communicate legitimate information, Comms establishes contact, Sensors localises, Helm/transport executes pickup and Medical handles casualties. |
| Risks | Player elimination, automatic replacement ships, guaranteed pod position, compulsory maintenance timers or silence treated as perfect stealth. |
| Dependencies | G012/G018/G061/G071/G109, S019/S060/S065–S067/S081/S083/S086/S087/S097. Exact pod equipment, capacity, endurance, recovery compatibility and survivor authority remain open. |

### Resolved choice: can survivors choose whether their pod broadcasts distress?

| Option | Form | Trade-off |
|---|---|---|
| A — Automatic distress | A working pod transmitter broadcasts distress automatically, without a signalling-policy choice. | Makes rescue discovery straightforward, but offers less control over exposure. |
| B — Selectable signalling | Where equipment permits, choose open distress, supported directed/secure contact or silence; ordinary AI operates the selected policy. | Gives survivors an exposure/recovery decision, but requires actual reach and detection consequences to be clear. |

**Selected: B.** Actual equipment permitting, survivors choose open distress, directed/secure contact or silence, with ordinary individual AI operating the selected policy. Silence does not guarantee concealment; rescue requires legitimate evidence and physical pickup. Useful human participation remains independent of simulated casualties. Exact pod equipment, endurance, capacity and recovery compatibility remain open.

## S101 — Remote-craft command / telemetry datalink

**Inclusion recommendation: retain gameplay communication limits for ordinary autonomous craft; resolve relay support.** [The Carrier Command 2 research transfer](./research/comparable-games-and-falling-frontier.md) identifies interpretable reports, bounded remote missions and recovery as useful bridge activities. S019/S086/S092 already provide ordinary autonomous entities, approved jobs and real transport commitments. Losing a gameplay link removes unavailable new commands/reports; it does not erase onboard knowledge or replace individual System AI with a special remote-control architecture. Existing orders and ordinary policies/context govern continued behaviour. Exact craft-specific safety policy remains open.

This is simulated communication, with Phoenix's networking unchanged. Gameplay range, occlusion, interference and equipment can govern delivery of orders and observations. The unresolved topology is direct links only versus supported relay equipment on ships/probes/deployables. Relays would carry permitted messages under actual reach, capability and security constraints, not grant fleet-wide knowledge or remote actuator authority.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Gameplay command/report links for ordinary autonomous craft; compare direct-only links with equipment-supported relays. |
| Ongoing activity | Monitor report age and link availability; ordinary AI operates approved communication and craft missions. |
| Active decisions | Reposition, deploy or protect a relay where supported, change mission priorities or recover craft when communication matters. |
| Complexity and AI | Routine supported routing can be automatic; players set purpose and priorities rather than forwarding every message. No special temporary Backfill behaviour. |
| Interactions and communication | Remote operators explain missing reports, Helm/other craft establish coverage, Comms provides available channels/security and Engineering supports resources. |
| Risks | Gameplay restrictions alter actual networking, a lost link freezes all craft, relays reveal hidden state, unlimited hops erase limits or every message needs manual routing. |
| Dependencies | G004/G007/G022–G024/G109/G114, S019/S061/S063/S065–S067/S086/S092. Exact ranges, relay compatibility, security, delay and routing remain open; AD74 excludes message-bandwidth accounting. |

### Resolved choice: can suitable equipment relay remote-craft commands and reports?

| Option | Form | Trade-off |
|---|---|---|
| A — Direct links only | A remote craft must communicate directly with its controlling ship. | Simple coverage model, with repositioning required when direct reach fails. |
| B — Equipment-supported relays | Suitable ships, probes or deployed equipment relay permitted messages over viable gameplay links. | Creates coverage, placement and protection choices, but requires readable routing and real equipment/link limits; AD74 excludes message-throughput budgets. |

**Selected: B.** Suitable ships, probes or deployed equipment relay permitted commands/reports over viable gameplay links. Routine supported routing is automated, with actual reach, interference, equipment resources, security and received-information limits. Remote craft retain ordinary individual System AI and onboard knowledge after link loss. Phoenix networking is unchanged. AD74 excludes message capacity/throughput tracking. Exact compatibility, delay, routing and craft-specific safety policies remain open.

**Architecture refinement — [AD72](./architecture/index.html#remote-order-lifetime):** pending structured remote orders use authored expiry and supersession, with durable mission orders explicitly persistent. Apply replacement only where newer intent is known; receivers still validate authority, validity and feasibility. Undelivered-order expiry does not cancel already accepted onboard work or erase reports/chat history. [AD73](./architecture/index.html#remote-order-receipts) selects automatic delivery and accepted/refused receipts returning over viable in-world links. Delivery, acceptance and completion are distinct; missing return communication leaves the sender uncertain, without exposing authoritative remote state or requiring manual acknowledgements.

**Architecture refinement — [AD74](./architecture/index.html#transceiver-capacity):** do not model message throughput, traffic-class budgets, congestion or quotas. Keep physical link/equipment and knowledge/security constraints. AD72–73 remain for unavailable links and uncertain return communication, not bandwidth contention. S066 listening coverage and AI reading/decoding time retain their separate meanings.

## S102 — Compartment atmosphere / pressure control and emergency pumping

**Inclusion recommendation: retain compartment safety and containment, then bound atmosphere detail.** [The Barotrauma/Stormworks research transfer](./research/comparable-games-and-falling-frontier.md) offers cross-specialist consequences of movement, hazards and local failures. It does not make flooding or a detailed fluid simulation appropriate for every spacecraft. S028/S029 already select physical routes and coarse compartment containment; S040 selects local life-support margins with time to intervene, rather than a universal ship countdown or individual survival chores.

The remaining question is whether atmosphere stays a coarse compartment condition/reserve model, or gains numerical pressure and equalisation across connected spaces. Both can support sealing, venting, restoring supply and relocating teams. The detailed alternative would need additional rules for doors, breaches and transfer rates; it should only be selected if these produce worthwhile bridge decisions beyond the existing containment model. Liquid pumping remains relevant only to justified hulls/hazards, not a universal System requirement.

| Aspect | Proposal or dependency |
|---|---|
| Amount and form | Coarse authored compartments with local safety margins; compare condition/reserve states with numerical pressure/flow modelling. |
| Ongoing activity | Maintain viable supported areas and follow known incidents; ordinary individual life-support/containment AI performs delegated routine operation. |
| Active decisions | Isolate or vent a compromised area, restore supply, prioritise access or relocate teams when local safety conflicts with work. |
| Complexity and AI | Presets expose useful priorities and known safety/access limits. Avoid per-door babysitting or individual oxygen management. |
| Interactions and communication | Engineering communicates safe access/restoration limits, teams report conditions and other operators weigh capability loss, evacuation and competing resource demand. |
| Risks | A separate survival game, healthy-compartment chores, instantaneous lethal failure, redundant pressure controls or adding flooding to every hull. |
| Dependencies | G009/G057/G059/G070/G071/G111, S028/S029/S040/S043/S075–S085. Exact reserves, condition thresholds, protection, restoration rates and hazard-specific equipment remain open. |

### Resolved choice: how detailed should compartment atmosphere be?

| Option | Form | Trade-off |
|---|---|---|
| A — Coarse conditions and margins | Track compartment atmosphere/safety states and reserves; breaches, isolation, venting and supply affect them through bounded rules. | Supports containment and team-access decisions with less simulation and interface complexity. |
| B — Numerical pressure and equalisation | Model pressure and flow between connected spaces, including breaches and door/seal states. | Adds physical interactions and more precise control, but expands simulation and operational complexity. |

**Selected: A.** Use coarse compartment atmosphere conditions and survival margins affected by breaches, isolation, venting and supply through bounded rules. No detailed numerical pressure/flow equalisation in the baseline. Ordinary individual life-support/containment AI operates delegated priorities; local safety, team access and evacuation remain meaningful. Liquid pumping applies only to justified hazards/hulls. Exact reserves, thresholds, protection and restoration rates remain open.

## Station synthesis — six-person reference

The first catalogue pass is complete. Compare candidate activity bundles before selecting a roster. All retain dedicated Captain coordination, specialist authority, independent per-System AI and Station presets. No candidate changes networking, grants Captain a map or makes routine actions require Captain approval. Final names and exact System ownership remain open.

**Assignment invariant clarified by the user:** all System-to-role assignments below are data-driven authored hull choices, not hard-coded game rules. System behaviour cannot require ownership by a particular role name. Each hull has one fixed roster; different hulls can assign capabilities differently while retaining single ownership, normal AI and information limits. Crew-count layout variants, player-defined bundles and live reconfiguration are not selected.

### Common candidate work

| Candidate seat | Sustained activity | Event-driven decisions and communication |
|---|---|---|
| Captain | Listen to reports, reconcile competing goals and track mission priorities. | Coordinate changing objectives, alert/stances and competing resource commitments; depends on crew interpretation and deliberate on-screen sharing. |
| Helm | Maintain useful movement and awareness of geometry. | Choose approaches, exposure, escape, docking/jumps and observation baselines in response to specialist needs. |
| Tactical | Manage engagement assignments, defensive coverage and weapon readiness through chosen delegation. | Commit fire, interception priorities, guidance support and combat positioning requests. |
| Engineering | Balance flows, batteries, heat and stocks while maintaining known capability margins. | Ration scarce resources and prioritise restoration, containment and support under changing demands. |

### Resolved choice: separate information seats or a dedicated Operations seat?

| Option | Remaining two seats | Where deployment work goes | Main concern to test |
|---|---|---|---|
| A — Separate Sensors and Comms | Sensors investigates/assesses; Comms handles conversations, requests and commitments. | Assign coherent operation types to relevant specialists: candidate examples are technical field work to Engineering, boarding/sorties to Tactical and survey work to Sensors. No final assignment selected. | Deployment commitments must not overload specialists; routine execution and selected interventions should reduce attention demand. |
| B — Combined information, separate Operations | One seat handles Sensors/Comms; Operations owns most deployment, personnel and logistics planning. | Dedicated Operations coordinates craft/team commitments and recovery, requesting evidence and resources from colleagues. | Simultaneous contact investigation and consequential conversations can overload the combined seat; Operations must have worthwhile work without compulsory assignment churn. |

**Selected: A as the first test candidate.** Six reference seats are Captain, Helm, Tactical, Engineering, Sensors and Comms. Deployment work is distributed into coherent specialist bundles rather than a dedicated Operations seat. Exact System ownership and complexity presets remain open. Validate combat, non-combat and recovery workload before treating this as balanced; later 7/8-person arrangements follow that work.

### Tactical's non-combat activity — candidate scope

The six-person candidate must give Tactical consequential non-combat work without manufacturing threats or turning weapons into a compulsory maintenance chore. Already selected security teams, carried craft, physical recovery and autonomous missions provide possible work. S017 retains AI-piloted sortie management; S086 makes utility-craft commitments task-dependent. This does not require a new drone-control architecture or manual flying.

One candidate keeps Tactical primarily weapons, defence and security, leaving rescue/utility transport with mission-owning specialists. Another gives Tactical a broader security-and-flight-operations bundle, including rescue/evacuation sorties. Sensors would still own investigation and survey intent, Engineering technical field work, and Comms negotiations/access; exact shared-bay authority is not yet selected. A broader bundle need not make Tactical approve every other specialist's deployment.

| Situation | Broader Tactical candidate activity | Required cooperation |
|---|---|---|
| Combat | Prioritise interception/fire, supported guidance and combat sorties. | Sensors supplies evidence, Helm geometry, Engineering resources and Captain overall priorities. |
| Civilian rescue | Choose pickup/protection priorities, commit suitable rescue craft and plan recovery. | Comms establishes needs/access, Sensors locates survivors/hazards, Helm approaches and Engineering supplies equipment/resources. |
| Investigation | Manage supported escort/security or available transport commitments when needed. | Investigation decisions remain with Sensors and relevant specialists; no compulsory security task for every enquiry. |
| Recovery after an incident | Recover committed craft/personnel and reassess readiness. | Actual resources, available transport, received reports and competing goals remain constraints. |

### Resolved choice: should Tactical include rescue/utility flight operations?

| Option | Form | Trade-off |
|---|---|---|
| A — Weapons, defence and security | Tactical owns combat sorties/security; mission-owning specialists manage their own utility transport and rescue work. | Keeps transport close to mission context, but Tactical may have fewer non-combat decisions. |
| B — Security and flight operations | Tactical also owns rescue/evacuation sortie planning and general utility-craft commitments; specialists retain their mission expertise. | Gives Tactical a stronger non-combat role, but simultaneous combat/rescue needs require useful delegation and clear ownership. |

**Selected: B.** Tactical includes security and flight operations: combat sorties, rescue/evacuation sorties and general utility-craft commitments alongside weapons/defence. Specialists retain their mission decisions; routine flight stays AI-operated. Validate simultaneous combat/rescue workload. Exact shared craft/bay authority and logistics ownership remain open; the selection does not itself impose Tactical approval on every specialist deployment.

### Shared craft requests — authority and attention

[The Plan baseline](../gdd/mechanics/duty-teams-and-operations.md#operations-systems) distinguishes operation ownership from remote requests for resources/personnel, without overriding human-held authority. The selected six-seat candidate needs the same distinction for craft: a scientist deciding to survey a site does not thereby acquire direct control of Tactical's craft Systems. Conversely, making Tactical manually approve every routine ferry risks creating a dispatcher chore during combat.

Compare case-by-case acceptance with standing authorisation for bounded specialist requests. A standing policy could reserve craft for emergencies and permit particular mission types within availability/priority limits. Ordinary individual System AI executes only delegated work under this authority. A request is not another actuator writer, an unrestricted command to another Station or permission to cancel an existing deployment. Exact request interface, reservation semantics and scheduling remain to be designed.

| Situation | Proposed standing-authorisation behaviour |
|---|---|
| Compatible craft is free and request meets policy | Delegated ordinary AI can fulfil the authorised request without another acknowledgement. |
| Craft is reserved, committed or unsuitable | Request remains blocked/queued with the known reason; no double booking or implicit cancellation of another mission. |
| A new request conflicts with priorities or exceeds policy | Relevant players coordinate the exception or reprioritisation; Captain can coordinate goals without gaining craft controls. |
| Craft communications fail | Continue ordinary onboard AI behaviour and respect information limits; standing authorisation does not restore a link. |

### Resolved choice: must Tactical accept every specialist craft request individually?

| Option | Form | Trade-off |
|---|---|---|
| A — Case-by-case acceptance | Tactical accepts each requested deployment before execution. | Gives direct control over every craft commitment, but can make routine transport an approval queue. |
| B — Standing authorisation | Tactical sets permitted work, priorities and reserves; qualifying requests can execute through delegated ordinary AI, with exceptions/conflicts brought to the player. | Preserves useful scarcity decisions while reducing routine acknowledgements; policy and commitment feedback must be clear. |

**Selected: B.** Tactical sets permitted work, priorities and reserves. Qualifying specialist requests can execute through delegated ordinary individual AI without case-by-case acceptance; exceptions and competing protected commitments require coordination. Requests neither take over another Station's Systems nor double-book craft or implicitly cancel deployments. Existing ownership/delegation remains; no central planner or unique Backfill behaviour. Exact interface, reservations and scheduling remain open.

### Medical ownership — avoid concentrating every crisis in Engineering

Engineering already faces ongoing power/heat balancing plus incident-driven repair and containment. Assigning treatment capacity and casualty priorities there as well may concentrate simultaneous crisis decisions. [Plan Medical](../gdd/mechanics/duty-teams-and-operations.md#medical) instead provides a distinct patient/triage workload with specialist biological scans requested from Science. S081 retains coarse injury and ordinary Medical AI; G110 restricts movement-sensitive treatment to selected major mission beats. These establish activities, not final Station ownership.

In the selected six-seat candidate, compare Engineering owning both machinery and treatment with Sensors expanding into a Science/Medical bundle. The latter groups biological interpretation and treatment priorities while leaving life-support machinery, atmosphere supply and repairs with Engineering. Tactical retains evacuation transport and Comms establishes rescue needs/access. Neither adds a seventh required seat or makes ordinary healing a manual minigame.

| Aspect | Science/Medical candidate |
|---|---|
| Ongoing activity | Interpret sensor evidence and known conditions; ordinary Medical AI handles authorised treatment within actual capacity/resources. |
| Active decisions | Choose investigation priorities, triage scarce treatment capacity and commit medical expertise to supported missions. |
| Communication | Explain patient/hazard needs, request safe access/resources from Engineering and coordinate extraction with Tactical. |
| Workload risk | Contact investigation and casualties can coincide. Test Station presets and delegation against a rescue during combat; do not assume adding AI removes all judgement demand. |

### Resolved choice: which reference Station owns Medical decisions?

| Option | Form | Trade-off |
|---|---|---|
| A — Engineering | Engineering owns treatment priorities as well as technical repair, life support and resources. | Concentrates restoration work, but may overload the Station during damage and casualties. |
| B — Sensors / Science | The sensor seat also owns Medical assessment and treatment priorities; Engineering retains machinery/life support and Tactical evacuation transport. | Groups scientific/biological judgement and spreads crisis demand, but sensing and triage workload must be tested together. |

**Selected: B.** The Sensors/Science seat owns Medical assessment and treatment priorities. Engineering retains machinery/life support and Tactical evacuation transport. Routine treatment uses ordinary Medical AI where delegated. Validate simultaneous sensing/triage demand; exact team authority and treatment-resource ownership remain open.

### Cargo and supply ownership — connect requests to actual stocks

G065–G067 already select main hold stores, local stock targets, timed automatic replenishment, meaningful consignments and manually initiated trade where required. S089 locates cargo in compatible compartments; S092 permits approved standing deliveries. Ownership must distinguish local consumption/readiness from ship-wide reserves and external supply commitments. A cargo owner does not acquire another Station's local controls or permission to cancel its deployed operations.

Engineering could own the main hold as part of technical resource management. Alternatively, Comms could broaden into communications/logistics: comparing received requests and offers with real stocks, reserving cargo and initiating supported supply/trade. Tactical would still own physical flight commitments and Engineering actual machinery/repair decisions. This does not introduce a market-price simulation or universal formal contracts.

| Activity | Comms/logistics candidate interaction |
|---|---|
| Routine internal replenishment | Local operators set their own stock targets; ordinary inventory AI fulfils reachable authorised deliveries from available hold stores. |
| Shortage or competing demands | Relevant operators explain actual needs; the hold owner adjusts reserves/supply priorities within its authority, with Captain coordinating goals. Exact scarcity arbitration remains open. |
| External request or trade | Comms considers actual cargo/reserves and terms, authorises supported exchanges and requests transport where needed. |
| Damaged storage or unsafe cargo | Engineering handles technical restoration/containment; Science assesses relevant hazards; cargo priorities remain distinct from those specialist decisions. |

### Resolved choice: who owns main hold and external supply priorities?

| Option | Form | Trade-off |
|---|---|---|
| A — Engineering | Engineering manages main stores/reserves and supply priorities; Comms negotiates requests and terms. | Keeps material readiness with technical management, but adds stock/trade demand to Engineering's crisis workload. |
| B — Comms / Logistics | Comms manages main cargo, reserves and external supply/trade priorities; local operators retain local targets and Engineering technical restoration. | Connects conversations to material choices and spreads workload, but Comms must handle simultaneous dialogue and supply decisions. |

**Selected: A.** Engineering owns main hold stores, reserves and external supply priorities; Comms negotiates requests and terms. Local operators retain local stock targets, ordinary inventory AI handles authorised routine movement and Tactical retains physical craft commitments. Test concurrent power, repair and supply demand; exact shortage arbitration and transaction controls remain open. The Comms/logistics table above remains an unselected comparison, not adopted ownership.

### Engineering presets — preserve a playable simple seat

Engineering's selected ownership combines power/heat, repair/containment and main stores. Before numerical tuning, define what the simplest human preset delegates. G002 requires authored Station presets, not separate per-System depth menus. The user's provisional reactor ladder remains the input: maximum has dozens of fine-grained power/coolant groups, middle several individually allocated groups, minimum several priority-ordered groups with AI power allocation and no/low/high coolant usage, and AI manages the whole task. This ladder is not yet validated or adopted as a finished Station preset.

Stock movement already follows targets automatically; repair execution already uses teams with actual parts/access and automatic fallback patching. The unanswered issue is repair job selection at the simplest human preset: must the beginner choose each damaged component, or may ordinary repair AI choose jobs within approved priorities/reserves? Delegating job selection does not create free repairs, an omniscient diagnosis service or authority to cannibalise active equipment without approval.

| Work | Candidate simplest-preset treatment |
|---|---|
| Power/coolant | User's provisional minimum: prioritise groups and choose no/low/high coolant usage; ordinary AI allocates within actual resources. |
| Local/main inventory movement | Retain chosen stock targets and reserve limits with automatic authorised replenishment; exact initial defaults remain open. |
| Repair execution | Teams perform supported component work; no per-step repair puzzle or patch/full-repair menu. |
| Repair job selection | Selected: ordinary AI chooses routine jobs with no required repair preset/policy setup, using existing orders, alert, mission context and known conditions. |
| Exceptional commitments | Scarce external supplies, cannibalisation and major mission choices retain their existing authority/protection boundaries. |

### Resolved choice: can the simplest Engineering level delegate routine repair job selection?

| Option | Form | Trade-off |
|---|---|---|
| A — Policy-based repair selection | Ordinary repair AI selects supported jobs within approved priorities and material reserves; the player changes priorities or intervenes when needed. | Lets a beginner concentrate on broad readiness and communication while repairs remain real commitments. |
| B — Human selects every job | The player chooses each component repair; teams automate execution only. | Keeps each team/material commitment explicit, but adds dispatch demand during power and supply crises. |

**Selected: A, with no required repair presets at the simplest level.** Ordinary individual AI chooses and executes routine repair work using existing orders, alert, mission context, known conditions and normal defaults. The beginner need not configure repair priorities, policies or material reserves. Actual evidence, access, parts and team commitments still constrain work; protected choices retain their boundary. This is ordinary AI behaviour, not a unique beginner controller. The earlier option's player-configured policy is not a prerequisite. Authored Station complexity selection remains distinct from operational repair presets and does not become a per-System setup menu.

### First manual Engineering layer — choose a coherent activity

After defining no-setup routine assistance, the next dependency is which operating problem the first manual layer introduces. It should not simultaneously add power allocation, repair dispatch and cargo administration. The user's provisional reactor minimum offers one candidate: priority-ordered power groups and no/low/high coolant usage, with AI allocation. A repair-led alternative would keep power automatic while introducing restoration priorities/team commitments. These are alternatives for an authored Station level, not individual depth switches the player must configure.

### Resolved choice: what should the first manual Engineering layer emphasise?

| Option | Form | Trade-off |
|---|---|---|
| A — Power/readiness first | Introduce the proposed group priorities and simple coolant demand; routine repairs/stocks stay AI-managed without setup. | Gives an ongoing operating problem and reasons to listen to other Stations, with event-driven repair complexity added later. |
| B — Restoration first | Power stays AI-managed; introduce repair priorities and team commitments as the first human operating responsibility. | Makes damage response tangible, but offers less ongoing demand when the ship is healthy. |

**Selected: A.** Engineering's first manual layer introduces power-group priorities and no/low/high coolant usage with ordinary AI allocation. Routine repair selection/execution and stock handling stay AI-managed without required setup. Additional restoration administration belongs to later authored Station levels; middle/detailed controls and workload remain under review.

### Middle Engineering layer — restoration priorities or component dispatch

The provisional middle reactor model already exposes several power groups with individual power/coolant allocations. Increasing restoration depth at the same level should preserve time to react to a crisis. S075 permits parallel repairs on different components of one System; S077 makes teams choose full repair or fallback patch automatically. These rules do not require the player to dispatch every component manually.

One middle-level candidate lets the operator prioritise restoring named Systems, with ordinary repair AI choosing known component jobs and team assignments. A more detailed candidate exposes individual component/team dispatch immediately. Both retain actual access, parts, team commitments and observation limits. Ordering a System restored does not guarantee every component can be repaired, bypass catastrophe or reveal unknown damage. Detailed map inspection can remain available without making individual dispatch compulsory.

### Resolved choice: how should middle-level Engineering direct repairs?

| Option | Form | Trade-off |
|---|---|---|
| A — Prioritise Systems | Choose which Systems need restoration first; ordinary repair AI selects supported component jobs and assigns teams. | Adds meaningful crisis priorities while leaving execution and dispatch manageable alongside power/coolant controls. |
| B — Dispatch components | Choose individual component jobs and team assignments at the middle level. | Gives precise control earlier, but adds more concurrent detail during damage incidents. |

**Selected: A.** Middle Engineering chooses System restoration priorities; ordinary repair AI selects supported component jobs and assigns teams. Actual evidence, parts, access and team commitments still constrain work. Detailed component/team dispatch is reserved for deeper-level review, not required at middle complexity. The underlying component model remains identical across levels.

### Fire-control ownership — engagement support versus investigation

S096 selects optional specialised tracking/illumination hardware, and S099 integrates motion estimation into relevant sensors/fire-control. G020 preserves independent broad and focused sensing; S098 applies shared turret/arc/tracking code. Shared implementation does not determine Station ownership. Tactical owns engagement priorities, while Sensors/Science interprets evidence and investigates contacts; either could own the specialised illuminator.

Assigning it to Tactical groups scarce engagement-support capacity with the weapons it supports. Assigning it to Sensors creates a direct investigation-versus-engagement-support prioritisation problem for that player, without physically coupling broad/focused capacity. Neither choice makes every weapon require illumination, gives hidden contact information or requires repeated manual lock maintenance. Ordinary individual AI maintains authorised solutions in either bundle.

### Resolved choice: who owns specialised fire-control/illumination?

| Option | Form | Trade-off |
|---|---|---|
| A — Tactical | Tactical allocates specialised engagement-support hardware; Sensors supplies evidence and assessments. | Keeps weapons/support priorities together while communication concerns evidence, geometry and exposure. |
| B — Sensors / Science | Sensors allocates specialised illumination alongside its other information work; Tactical requests support. | Adds cross-Station engagement coordination, but increases Sensors' concurrent combat/Medical workload and can create a support bottleneck. |

**Selected: B.** Sensors/Science owns specialised fire-control/illumination; Tactical requests engagement support. Ordinary individual AI maintains approved solutions under finite capacity, evidence, geometry, emissions and resource constraints. Broad/focused sensing remain independent; illumination is not a universal weapon requirement. Test concurrent investigation, support and Medical workload. Exact request/assignment controls remain open.

### Human-operated illumination — preserve an actual allocation decision

Tactical can request support without taking control of the illuminator. At a human-operated Sensors level, compare persistent explicit assignments with an authorised follow-Tactical policy that reallocates against changing engagement targets. Either can use ordinary AI to track an assigned target; maintaining a solution does not require a human to approve each shot or repeatedly click the same contact. AI-operated Systems retain normal orders, context and hysteresis rather than gaining a unique support controller.

An explicit assignment gives the Sensors player a decision about emissions, scarce tracking capacity and competing targets. Automatic following can reduce routine work but must be deliberately authorised and must not overwrite an active manual assignment. Lost solutions and the exact interruption/reacquisition rules remain equipment-specific open details.

### Resolved choice: what is the default at a human-operated illumination level?

| Option | Form | Trade-off |
|---|---|---|
| A — Persistent explicit assignments | Sensors chooses supported targets; assignments persist until changed or no longer viable, with ordinary AI maintaining tracking. | Preserves deliberate support allocation and communication without approval for each shot. |
| B — Authorised automatic following | Sensors authorises a policy to follow Tactical's changing engagement priorities within limits; manual assignments override it. | Reduces repeated target allocation, but can make support choices less visible unless conflicts/emissions are clear. |

**Selected: A, with a target-authority clarification.** Sensors makes persistent illumination assignments; ordinary AI maintains tracking. Tactical independently chooses weapon targets, and illumination only helps. An illumination assignment does not select a weapon target or grant/veto firing permission; losing it loses assistance, subject to other actual weapon rules. This refines G105's earlier inclusion of illumination among possible prerequisites. Simpler authored Station levels can delegate the whole System under ordinary AI. Exact assistance effects and interruption/reacquisition remain open.

### Shield ownership — protection decisions versus energy supply

S022–S024 provide shield arcs, battery-mediated redistribution and retuning with temporary protection reduction/resource costs. Engineering already owns the flows that supply local batteries and cooling; this does not automatically determine who chooses defensive arc priorities and retuning. Tactical owns weapons/defence, while Sensors supplies evidence about threats and shield/weapon interactions.

### Resolved choice: who operates shield priorities, redistribution and retuning?

| Option | Form | Trade-off |
|---|---|---|
| A — Tactical | Tactical chooses defensive configuration; Engineering provides power/cooling and Sensors relevant evidence. | Groups protection with engagement/exposure decisions and creates resource coordination with Engineering. |
| B — Engineering | Engineering chooses shield configuration alongside supply/thermal management; Tactical requests protection changes. | Groups shield reserves with resources, but adds defensive decisions to Engineering's damage workload. |
| C — Science, supplied by the user | Science chooses shield configuration; Engineering supplies power/cooling, Tactical chooses weapon targets and Helm controls exposure. | Gives Science more direct combat activity; simultaneous sensing, illumination, shields and Medical demand needs testing. |

**Selected: Science (user-supplied alternative C).** Science operates shield priorities, redistribution and retuning to provide more combat activity. Engineering supplies power/cooling, Tactical chooses weapon targets and Helm controls exposure. Ordinary shield AI executes delegated priorities with the same physical/resource rules. Validate concurrent shields, sensing, illumination and Medical demand. The earlier Tactical recommendation is not adopted.

### Comms in combat — signal warfare ownership

Comms already handles fleet/enemy conversations, generic requests, public chatter, interception and decoding. S018 adds geometry-dependent jamming with real friendly interference; S020 supplies finite electronic protection and S021 bounded remote disruption without hostile System takeover. These can be grouped with Comms' signal interpretation or Tactical's engagement work. Ownership is distinct from the shared turret/tracking code or the targeted ship's System authority.

Assigning signal warfare to Comms would make it an operator affecting combat conditions as well as interpreting messages. It must be able to judge the cost of suppressing channels needed for negotiation, friendly reports or recovery. Tactical would still choose weapon targets; Sensors would still investigate and operate shields/illumination. Either ownership uses ordinary individual System AI and leaves networking unchanged.

### Resolved choice: who owns jamming and electronic protection/disruption?

| Option | Form | Trade-off |
|---|---|---|
| A — Comms | Comms operates supported jamming, electronic protection and remote disruption alongside channels/interception. | Connects signal interpretation to combat intervention, but simultaneous conversations and interference decisions need testing. |
| B — Tactical | Tactical operates those capabilities with weapons/sorties; Comms interprets channels and reports communication effects. | Groups offensive effects together, but adds to Tactical's concurrent engagement workload. |
| C — Split attack/protection, supplied by the user | Comms owns jamming and disruption; Science owns electronic protection alongside shields. | Gives both seats combat decisions connected to their information/protection responsibilities. |

**Selected: split ownership (user-supplied C).** Comms owns jamming and remote disruption; Science owns electronic protection alongside shields. These are data-driven reference-layout assignments, not System-to-role hard rules. Existing geometry, resources, evidence, protection granularity and ordinary individual AI remain unchanged. Test concurrent dialogue/interference and Science protection/sensing/Medical demand.

### Crew-size layouts — authored alternatives or a fixed roster

G001 selects designer-authored bundles, G002 simple Station complexity selection and G063 ordinary AI for absent humans. Two humans can occupy selected seats while ordinary AI operates others; this is not a reason to combine manual Stations. Eight humans need eight coherent seats if the scenario/hull supports them. System-to-role mappings can be authored in data, but the interview has not selected whether a reference hull offers alternative authored crew layouts or one fixed roster with additional seats.

| Option | Form | Trade-off |
|---|---|---|
| A — One fixed roster per hull | Author all supported seats for a hull; six humans leave remaining seats AI-operated, eight occupy more of that same roster. | Keeps ownership stable across crew sizes, but six-person activity depends on where important work lives in extra seats. |
| B — Authored layout variants | Offer coherent pre-authored six-, seven- or eight-seat arrangements where appropriate; choose before the mission. | Can redistribute substantial work for each crew size, but requires validating each layout and keeping setup simple. |

### Resolved choice: should a hull support pre-authored crew-layout variants?

**Selected: A for current design scope.** Use one authored data-driven roster per hull; ordinary AI operates vacant seats and existing human-seeking Systems address important human interventions. The user clarifies that the data system technically supports variants by inheriting a ship and overriding Stations, but those variants will not be authored for now. This is a content-scope decision, not an architectural prohibition or missing capability. Different hulls may author different assignments.

### Eight-player support — per-hull capacity or catalogue coverage

With fixed rosters, supporting eight dedicated human Stations requires enough authored seats on the selected hull. Human-seeking Systems provide important interventions to eligible humans but do not by themselves create extra dedicated seats or justify routine filler. The six-seat reference remains useful as an activity design; it does not yet establish whether every hull must offer eight seats or whether larger crews select suitable hulls.

### Resolved choice: must every playable hull provide eight worthwhile human seats?

| Option | Form | Trade-off |
|---|---|---|
| A — Eight-seat capacity on every hull | Every playable hull has at least eight coherent dedicated Stations; six humans leave some AI-operated. | Any crew can use any hull, but the extra roles must earn their place even on simpler ships. |
| B — Capacity varies by hull | Each hull authors an appropriate fixed roster; the game supports eight through suitable hulls, with capacity clear during selection. | Preserves coherent ship-specific bundles, but eight-person crews need an eligible hull/scenario combination. |

**Selected: B.** Retain the user-specified progression: Courier 2, Destroyer 4, Cruiser 6 and Battleship 8. Larger crews select suitable hulls. Vacant seats use ordinary AI; human-seeking Systems handle important human interventions. Exact redesigned bundles and scenario coverage remain to be validated.

**Current asset check:** the Alliance courier/destroyer/cruiser author 2/4/6 non-auxiliary Stations respectively. The current Alliance battleship authors 9 non-auxiliary Stations (Captain, Helm, Tactical, Repair, Sensors, Shields, Navigation, Power, Comms), despite the agreed eight-seat class target. This is baseline evidence, not a change to the selected target; resolve the actual redesigned Battleship roster through activity design. No asset changes were made. Auxiliary human-seeking consoles are not counted as additional dedicated seats here.

### Battleship activity bundles — first eight-seat candidate

Use a separately authored Battleship roster rather than a crew-count variant of the Cruiser. The six-seat reference's largest candidate bundles are Engineering (power, repair, containment, stores), Science (sensors, illumination, shields, electronic protection, Medical) and Tactical (weapons, security, flight operations). Two additional seats should separate substantial decisions, not merely duplicate controls.

### Resolved choice: which bundles should the first eight-seat candidate split?

| Option | Eight-seat candidate | Trade-off |
|---|---|---|
| A — Split Engineering and Science | Captain, Helm, Tactical, Comms, Power, Repair/Stores, Sensors/Medical, Protection. | Spreads power/restoration and sensing/defence; Protection needs worthwhile non-combat responsibilities as well as combat work. |
| B — Split Engineering and Tactical | Captain, Helm, Weapons, Flight/Security, Comms, Power, Repair/Stores, Science. | Separates combat from rescue/sorties, but leaves Science's sensing/protection/Medical bundle broad. |

**Selected: A.** The first eight-seat Battleship candidate is Captain, Helm, Tactical, Comms, Power, Repair/Stores, Sensors/Medical and Protection. Split Engineering and Science rather than Tactical. This is a separately authored hull roster, not a runtime split or Cruiser layout variant. Exact System assignments and workload validation remain open, especially Protection's non-combat activity.

### Protection's non-combat scope — ship safety or external defence only

The selected Protection seat begins with shields and electronic protection. S029/S040/S102 already supply compartment containment, life-support margins and coarse atmosphere decisions; S094 separately supplies fitted remote energy/shield assistance. These are existing proposal capabilities, not reasons to invent a new minigame to fill a seat. The next ownership decision is whether Protection also operates internal safety Systems on the Battleship.

A broader ship-safety bundle can decide which areas to seal/vent, preserve safe access and support evacuation while Repair/Stores restores components and Power supplies resources. Requesting a team does not seize another Station's workforce; exact team authority remains to be assigned. Healthy machinery still requires no compulsory safety chores. Shields only affect environmental hazards where actual equipment/hazard rules support that interaction; this assignment does not make them universal protection.

| Situation | Broader Protection candidate decision | Other Stations retain |
|---|---|---|
| Combat damage | Balance shield/ECCM priorities with known compartment containment needs. | Power allocation, component restoration, weapon targeting and movement. |
| Rescue or hazardous recovery | Preserve safe receiving/working areas and coordinate supported protective equipment. | Medical triage, transport commitments, cargo priorities and scientific hazard assessment. |
| Internal incident | Isolate, vent, restore supported atmosphere operation or request relocation/repair. | Physical repairs and route restoration; no automatic knowledge of hidden faults. |
| Quiet preparation | Check known readiness and establish viable protection settings, then monitor changing conditions. | No invented incident quota, healthy-compartment maintenance loop or manual oxygen babysitting. |

### Resolved choice: should Battleship Protection also own internal safety Systems?

| Option | Form | Trade-off |
|---|---|---|
| A — External defence focus | Protection owns shields/ECCM and suitable external support; Repair/Stores retains internal containment/life-support operation. | Keeps a narrow defence role, but non-combat activity depends more on fitted equipment and scenario hazards. |
| B — Defence and ship safety | Protection also operates containment/life-support/atmosphere priorities; Repair/Stores handles physical restoration and Power supplies resources. | Gives coherent rescue/incident decisions, but simultaneous external defence and internal safety workload must be tested. |

**Selected: B.** Battleship Protection operates containment, life-support and atmosphere priorities alongside shields/electronic protection. Repair/Stores handles physical restoration and Power supplies resources. Reuse the selected coarse safety/component rules and ordinary individual AI; healthy operation adds no compulsory chores. This is a data-driven candidate assignment, not a universal role rule. Validate simultaneous defence and internal-safety demand.

### Non-combat workload trace — disabled passenger ship

This is a hypothetical design exercise, not a new authored scenario or a simulation result. A damaged passenger ship has inaccessible areas, limited safe occupancy and salvageable life-support equipment. It can communicate imperfectly. The bridge can support repair, provide compatible supplies or evacuate people; craft/team commitments and power compete. No attackers are required to give Tactical or Protection work. Actual hazards and deadlines would need authored, disclosed causes and prepared intervention margins.

| Seat | Consequential activity | Communication dependency |
|---|---|---|
| Captain | Coordinate repair versus evacuation and revise priorities as reports arrive. | Needs specialist assessments; no private map or direct operation of their Systems. |
| Helm | Choose a viable support/transfer position and departure options. | Needs transport, sensor geometry and resource constraints. |
| Tactical | Commit rescue craft and plan pickup/recovery against limited available sorties. | Needs locations, safe destinations, patient priorities and access. |
| Comms | Establish actual needs, permission and cooperation; follow other relevant traffic. | Needs feasible options/resource estimates from specialists before making supported requests or commitments. |
| Power | Prepare reserves and ration concurrent sensor, transport, tractor or compatible remote-energy demand. | Needs upcoming use and tolerated interruption; quiet preparation may produce breathing room rather than forced churn. |
| Repair/Stores | Choose useful component restoration and cargo/parts commitments. | Needs diagnosis, access, safe work conditions and transport. |
| Sensors/Medical | Establish hazard/survivor evidence, prioritise treatment and assess whether further investigation changes the plan. | Needs team reports, transport availability and safe receiving space. |
| Protection | Preserve safe receiving/working compartments, decide containment changes and operate applicable protective equipment. | Needs hazard evidence and evacuation/repair intentions; cannot assume remote control of the passenger ship's safety Systems. |

This trace identifies plausible decisions, not proof all eight players stay engaged. Test concurrency, meaningful alternatives and periods of waiting at chosen complexity. External energy assistance is selected at S094, with Power ownership resolved below.

### Resolved choice: who owns fitted remote energy-transfer equipment on the Battleship?

| Option | Form | Trade-off |
|---|---|---|
| A — Power | Power operates external energy transfer alongside generation/distribution; Protection retains remote shield assistance where fitted. | Gives Power a direct service decision and keeps donor energy commitments together. |
| B — Protection | Protection operates both remote energy and shield assistance; Power supplies the requested flow. | Groups external support together, but adds transfer decisions to Protection's safety workload. |

**Selected: A.** Power operates fitted remote energy-transfer equipment; Protection retains remote shield assistance where fitted. Power weighs external help against donor reserves and upcoming local demand. Actual equipment, geometry, recipient compatibility and resources remain constraints. This data-driven ownership decision does not change transfer mechanics or ordinary individual AI.

### Energy-flow boundary — a full local battery

The selected model already makes energy-consuming Systems use local batteries, with fullness-dependent passive loss and heat from charging/depletion. Remote assistance must respect actual receiving equipment and the same storage/operating rules rather than grant free output. Exact receiving topology remains equipment-dependent and open. The proposal explicitly leaves incoming flow at full charge unresolved; settle that common rule before adding special transfer exceptions.

Compare a configured flow that is limited by actual receiving headroom with a committed flow whose excess is wasted when the battery is full. Under either rule, maintaining a full reserve remains inefficient because passive drain increases with fullness and replacing lost energy has the selected thermal cost. Headroom-limited acceptance does not automatically redistribute another System's reserved allocation: normal source controls/AI still govern allocation. Wasting surplus would need an explicit overflow rule; additional overflow heat is not implied by the already selected battery-change heat model.

### Resolved choice: what happens to allocated incoming energy when a local battery is full?

| Option | Form | Trade-off |
|---|---|---|
| A — Headroom-limited intake | Actual accepted flow reduces to supported consumption/loss/headroom; energy that cannot be accepted is not drawn from the source. | Avoids a new overflow penalty while retaining the cost of high reserves and recharge. |
| B — Waste surplus | The configured flow keeps drawing energy; anything beyond usable intake is discarded, with exact disposal/thermal consequences requiring definition. | Makes over-allocation more costly, but adds another failure/waste rule and monitoring demand. |

**Selected: A.** Actual intake is limited by supported consumption, passive loss and available battery headroom. Unaccepted energy is not drawn from the source; no compulsory overflow-waste rule is added. Full reserves still incur fullness-dependent passive loss and replenishment heat. Source output, heat and allocation controls retain their rules; unused acceptance does not automatically redistribute reserved flow elsewhere.

### Courier two-seat activity — use the authored baseline accurately

The [current Alliance Courier data](../../assets/entities/alliance_courier.toml) assigns helm actuators, sensor radar, tactical radar and its blaster to `tactical`, while `captain` owns navigation, Comms, shields, repair, reactor/battery and command/view controls. Its manual overview describes a different split, so the authored System mapping is the baseline here. This inspection does not change assets or claim those overview strings are design truth.

The selected class progression retains two Courier seats. A small ship needs two coherent activities with ordinary AI handling delegated Systems, not two people manually operating six full Stations. The dedicated lightly equipped Captain remains the six-person reference; data-driven small-hull assignments need an explicit design rather than silently assuming identical role names imply identical duties.

### Resolved choice: which two-seat Courier bundle should we test first?

| Option | Form | Trade-off |
|---|---|---|
| A — Command/support plus pilot/weapons | Retain the current broad split: one seat handles communication/navigation/ship support, the other movement and combat. | Builds directly on authored ownership, but both observation and combat movement can concentrate on the pilot. |
| B — Pilot plus mission/systems | One seat owns movement and weapon targeting/execution decisions; the other owns sensing, communication and ship support, with coherent whole-System assistance. | Separates observation/interpretation from physical engagement so the pair exchange useful information in combat and non-combat work. |

**Selected: B, following the user's correction.** One seat operates movement and weapon decisions; the other sensing, communications and ship support, with ordinary whole-System assistance. Helm and tactical radar information can automatically combine in a shared presentation. Preserve available evidence, uncertainty/age and distinct control authority; this does not merge underlying Systems or automatically copy all Sensors findings. The corrected choice supersedes the preceding A and helm/sensors-display suggestion. Exact names, equipment and Station complexity remain open.

### Destroyer four-seat reference — coordination versus a specialist information seat

[The current Alliance Destroyer data](../../assets/entities/alliance_destroyer.toml) provides Captain, Helm, Tactical and Engineering as dedicated seats. Sensors belongs to Captain; Navigation, Comms and Command are auxiliary human-seeking consoles. That is a baseline, not a requirement to preserve this exact role split. The selected class progression retains four dedicated seats; human-seeking and ordinary AI remain available without new behaviour.

The six-person reference has a dedicated lightly equipped Captain with no private map. The smaller-hull question is whether the Destroyer also reserves a seat chiefly for coordination, or uses its fourth seat for Science/Comms with leadership through conversation. A specialist information seat can own its legitimate sensor displays without becoming a Captain map. This does not change the dedicated Captain decision for Cruisers or Battleships.

### Resolved choice: should the four-seat Destroyer retain a dedicated Captain seat?

| Option | Form | Trade-off |
|---|---|---|
| A — Dedicated coordination | Captain, Helm, Tactical and Engineering; information/communication work uses authored auxiliary/human-seeking and AI mechanisms. | Retains an explicit coordination seat, but must prove enough coordination demand and avoid turning Captain into an all-purpose specialist. |
| B — Specialist fourth seat | Helm, Tactical, Engineering and Science/Comms; crew leadership is a social responsibility rather than a separate coordination-only seat. | Gives each of four players a direct operating problem, but the crew must still coordinate competing goals and human-seeking work. |

**Selected: B.** The Destroyer reference has Helm, Tactical, Engineering and Science/Comms. Leadership is a crew responsibility rather than a separate coordination-only seat. Six/eight-seat reference ships retain dedicated Captain roles. Existing data-driven Command/human-seeking mechanisms remain; exact Command hosting and other assignments need authoring and workload validation.

### Destroyer Command hosting — no new Captain controls

The current Destroyer's auxiliary `command` Station authors `host_order = ["captain"]` in [hull data](../../assets/entities/alliance_destroyer.toml). That exact host list cannot serve the selected Captain-less reference roster unchanged. This is an authored assignment to update if the proposal is adopted, not a need for a new human-seeking mechanism. Command remains limited to its own supported orders/stances; it does not give its host specialist controls, a duplicate map or the power to override human-held Stations.

The first preference could be Science/Comms, connecting external communication and reports to the ship's AI posture, or Helm, connecting travel/approach intent to it. Either adds attention to an existing player and requires testing. Remaining eligible hosts and ordinary finite fallback behaviour can use the existing mechanism; exact full order remains open.

### Resolved choice: where do the Destroyer's coordination controls belong?

| Option | Form | Trade-off |
|---|---|---|
| A — Science/Comms first | The information/communications seat is the first authored host preference for Command. | Connects requests and reports to AI orders, but adds demand during investigation/conversations. |
| B — Helm first | Helm is the first authored host preference for Command. | Connects approach/travel intent to AI orders, but adds demand during precise manoeuvres. |

**Selected by the user:** Command on Comms (the combined Science/Comms seat), alert controls on Tactical, camera controls on Helm. These are separate authored assignments, not a bundle of Captain controls handed to one specialist. Existing authority and human-seeking mechanisms remain; exact fallback lists need authoring. Camera controls do not replace/restrict other Stations' deliberate on-screen information publication. Six/eight-seat Captain roles are unchanged by this Destroyer decision.

### Destroyer defensive workload — test the four-seat difference

The current Destroyer places shields under Engineering. The selected Cruiser reference instead places shields and electronic protection under Science. Data-driven ownership permits different choices; the Cruiser decision must not silently become a hard rule on the four-seat ship. Destroyer Science/Comms already combines observation, dialogue and Command, while Engineering has power, repair and stores. Ordinary AI reduces execution demand but does not remove competing judgements.

In a hypothetical encounter, Helm chooses geometry, Tactical chooses weapon targets/defensive interception, Science/Comms interprets contacts and requests while allocating supported illumination, and Engineering balances resources/restoration. A shield configuration change can either join Engineering's resource decisions or Science/Comms' evidence/support decisions. This is a workload hypothesis, not a new authored scenario or validated result.

### Resolved choice: where should Destroyer shield operation live?

| Option | Form | Trade-off |
|---|---|---|
| A — Engineering | Retain Engineering ownership of shield priorities/redistribution/retuning, alongside its power/repair work. | Spreads combat intervention across the four seats and avoids adding defence controls to combined Science/Comms. |
| B — Science/Comms | Put shield operation with sensing/illumination, as on the Cruiser's Science seat. | Groups observation and defensive interpretation, but combines those decisions with conversations and Command. |

**Selected: A (user accepted the recommendation).** Destroyer shield operation remains with Engineering. The smaller hull need not copy the Cruiser mapping. Ordinary repair/stock assistance supports the broader workload; validate concurrent power, shielding and restoration demand. Exact electronic-protection assignment remains open if that equipment is fitted.

### Cruiser Science complexity — one useful beginner activity

The six-seat candidate puts sensing, illumination, shields, electronic protection and Medical with Science. These are owned capabilities, not a requirement for every player to operate them manually. G002/G003 select authored Station complexity levels and competent whole-System assistance. S057 makes broad assessment automatic; S056 offers a scan subject/question; the user already gives basic targeting/steering as examples of worthwhile beginner actions.

The simplest human Science level should expose a coherent activity without requiring setup for every other System. Compare choosing an investigation subject and interpreting/reporting findings with directing protection priorities while investigations are delegated. Other Systems use ordinary individual AI with existing context and knowledge, not a special beginner controller. Protected human-seeking choices retain their existing boundary. More advanced authored levels can add manual illumination/protection/Medical decisions; their precise combination remains open.

### Resolved choice: what should the simplest human Cruiser Science level emphasise?

| Option | Form | Trade-off |
|---|---|---|
| A — Investigation first | Choose what to investigate, inspect findings and explain their meaning; ordinary AI operates protection, illumination and routine Medical. | Gives a clear combat/non-combat activity and useful reporting without several simultaneous control problems. |
| B — Protection first | Choose protection priorities; ordinary AI handles routine investigations, illumination and Medical. | Gives direct defensive agency, but quieter scenarios need supported safety/protection decisions to sustain interest. |

**Selected: A.** The simplest human Cruiser Science level emphasises choosing investigations, interpreting findings and communicating their consequences. Ordinary individual System AI operates protection, illumination and routine Medical without mandatory setup for each delegated System. Existing human-seeking boundaries remain. Exact beginner controls and workload need testing.

### Cruiser Science middle complexity — which additional decision comes first?

Investigation remains the human's core activity. The selected ownership gives Science both defensive operation and illumination support, but introducing every owned System at the next level would undermine the purpose of authored complexity levels. Compare adding a coarse defensive priority decision with adding explicit illumination assignments. Neither changes underlying equipment or gives Science authority over Tactical's weapon targets.

### Resolved choice: what should middle Cruiser Science add to investigation?

| Option | Form | Trade-off |
|---|---|---|
| A — Defensive priorities | Direct shield-arc and electronic-protection priorities; ordinary AI executes those priorities and handles illumination and routine Medical. | Adds a direct survival decision linked to Helm's exposure and Engineering's resources, but can compete with investigation during crises. |
| B — Illumination assignments | Choose contacts for sustained illumination; ordinary AI handles defensive operation and routine Medical. | Adds offensive support linked to Tactical's intentions, but its usefulness depends on fitted equipment and the encounter. |

**Selected: A.** Middle Cruiser Science adds shield-arc and electronic-protection priorities to investigation. Ordinary individual AI executes those priorities and handles illumination and routine Medical. Detailed shield redistribution/retuning remains for deeper-level review. Test simultaneous investigation, defence and communication; this is a selected test candidate, not a validated workload.

### Detailed Cruiser Science — breadth versus specialist depth

The reference seat owns investigation, illumination, shields, electronic protection and Medical. Ownership does not require continuous manual execution. The remaining complexity question is whether the deepest authored level exposes decisions across that whole bundle, or concentrates additional human attention on sensing/defence while Medical remains delegated. Both retain competent ordinary individual AI, the same evidence and physics, and no per-System complexity editor.

### Resolved choice: what should detailed Cruiser Science expose?

| Option | Form | Trade-off |
|---|---|---|
| A — Full owned bundle | Add persistent illumination assignments, detailed shield redistribution/retuning, equipment-supported sensing/protection controls and Medical treatment priorities. | Gives an experienced player broad intervention choices, but simultaneous demands must be tested and ordinary AI must continue delegated work. |
| B — Sensing/defence depth | Add the sensing, illumination and defensive controls, while routine Medical treatment priorities remain AI-managed at every standard Science level. | Concentrates attention on related decisions, but leaves less human agency over competing recovery needs. |

**Selected: A.** Detailed Cruiser Science exposes the full owned bundle. Availability does not require continuous manual operation of every System: ordinary individual AI continues delegated work. No new mandatory clinical procedures, per-shot illumination approval or manual arithmetic are introduced. Detailed controls correspond to selected mechanics; concurrent workload remains an explicit validation risk.

### Simplest Cruiser Tactical — meaningful activity outside combat

Basic human targeting with AI weapon execution is already an agreed beginner example, so it does not need another vote. Tactical also owns flight operations and security in the six-seat reference. A combat-only beginner interface risks leaving that player idle during rescue, survey or transport work. The open question is whether the simplest authored Station level retains goal-level sortie decisions alongside targeting, or delegates those decisions entirely.

Shared craft still follow the selected standing-authorisation model. Qualifying specialist requests need no per-trip approval. Human sortie choices concern goals, competing commitments or exceptions; they do not turn every routine launch into a confirmation prompt. Ordinary individual AI chooses and operates suitable available craft within actual authority, knowledge and resources. Neither option adds direct small-craft piloting.

### Resolved choice: should simplest Tactical retain goal-level flight-operation decisions?

| Option | Form | Trade-off |
|---|---|---|
| A — Targeting plus sortie goals | Retain basic weapon targeting and choices such as which rescue or transport goal gets scarce craft first; ordinary AI handles craft selection and execution. | Provides combat and non-combat agency without detailed bay administration, but adds occasional competing priorities. |
| B — Targeting first | Delegate routine flight-operation decisions to ordinary AI at the simplest level, exposing them at deeper levels. | Keeps the entry interface narrowly focused, but can leave fewer meaningful decisions in non-combat scenarios. |

**Selected: B.** Simplest Cruiser Tactical retains basic weapon targeting; ordinary individual AI manages delegated flight operations. Human sortie decisions appear at deeper authored Station levels. No special beginner AI or mandatory flight-policy setup is introduced. Actual authority, evidence, resources and commitments remain constraints. Non-combat activity at this level needs testing; do not silently restore the rejected manual sortie duties. Security responsibilities and higher Tactical complexity remain open.

### Middle Cruiser Tactical — combat depth versus broader operations

The simplest level now concentrates on basic targeting. The next authored level could deepen the weapon decision through separate group targets and offensive/interception priorities, or retain simple targeting while adding goal-level sortie choices. Existing weapon grouping, defensive interception and flight-operation mechanics supply these decisions; this does not introduce new equipment or AI behaviour. Tactical retains target authority independently of Science illumination.

### Resolved choice: what should middle Tactical add first?

| Option | Form | Trade-off |
|---|---|---|
| A — Weapon-group priorities | Choose separate weapon-group targets and offensive/interception priorities; ordinary AI handles supported execution and flight operations. | Deepens combat decisions about scarce weapons and coverage before widening the operating bundle. |
| B — Sortie priorities | Retain basic targeting and add rescue, transport and combat-sortie goals/priorities; ordinary AI selects and operates craft. | Broadens human work across scenarios, but introduces a second operational problem before deeper weapon control. |

**Selected: A.** Middle Cruiser Tactical adds separate weapon-group targets and offensive/interception priorities. Ordinary individual AI handles supported execution and delegated flight operations. Actual equipment, firing geometry, coverage and resources constrain results; priorities do not guarantee interception. Tactical target authority remains independent of Science illumination. Detailed controls, security and simultaneous workload remain open.

### Detailed Cruiser Tactical — how much sortie dispatch?

S017 already selects bounded craft missions, priorities and recall orders with ordinary individual craft System AI handling flight/execution. The Station review places human flight-operation decisions above the simplest and middle levels. Shared craft use already supports standing authorisation, priorities and reserves without per-trip approvals. The remaining dispatch question is whether detailed Tactical can also select particular craft for a mission, or always delegates that selection.

### Resolved choice: should detailed Tactical allow explicit craft assignment?

| Option | Form | Trade-off |
|---|---|---|
| A — Goals and reserves only | Choose sortie goals, priorities, reserves and recalls; ordinary AI always chooses suitable available craft. | Keeps management compact, but limits deliberate use of a particular craft's equipment or condition. |
| B — Optional craft assignment | Keep goal-level dispatch and add the option to assign particular craft or a selected group to a bounded mission. | Supports deliberate equipment/risk decisions, but needs clear availability and commitment feedback to avoid unnecessary micromanagement. |

**Selected: B.** Detailed Tactical supports optional assignment of particular craft or groups to bounded missions. Goal-level dispatch remains available; ordinary individual AI can select craft and handles flight/execution. Existing commitments, launch/recovery capacity, knowledge and communication limits still apply. Explicit assignment does not permit double-booking, instantaneous recall/recovery or control of another Station's Systems. Routine authorised work does not acquire mandatory per-craft approval.

### Cruiser Tactical security — where human team objectives enter

S082 already selects whole-team objectives with automatic routing and encounter execution, rather than individual combat control. Tactical owns security in the six-seat reference. Its simplest level concentrates on targeting, middle adds weapon-group priorities, and detailed adds human flight operations. The remaining placement question is whether middle also exposes security objectives, or leaves security delegated until detailed complexity. Both preserve existing human-seeking scenario requirements and ordinary individual System AI.

### Resolved choice: at which Tactical level should human security-team objectives appear?

| Option | Form | Trade-off |
|---|---|---|
| A — Detailed | Ordinary AI manages security at simplest/middle; detailed Tactical can assign whole-team protection, boarding and other supported objectives. | Keeps middle focused on weapons, but security decisions remain delegated at that level even during boarding incidents. |
| B — Middle | Add whole-team security objectives alongside weapon-group priorities; detailed adds finer supported assignment and flight-operation choices. | Offers earlier agency over onboard threats, but broadens middle-level attention during combat. |

**Selected: A.** Human whole-team security objectives enter at detailed Cruiser Tactical complexity. Simplest/middle delegate security to ordinary individual AI using its actual knowledge, orders and constraints. No special automatic rescue or privileged threat information is introduced. Detailed Tactical's combined weapons, sorties and security workload must be tested. Scenario-required human control still uses the existing human-seeking mechanism.

### Simplest Cruiser Comms — conversation versus intelligence emphasis

The reference Comms seat owns external conversations/requests, interception/decoding and jamming/disruption. S065 selects persistent WhatsApp-like threads; G022/S069 provide contextual individual and fixed-group requests. S066/S067 provide constrained interception/decoding, and S068 leaves interpretation of risk to Comms rather than adding automatic risk judgements. The simplest authored Station level needs a coherent human activity while ordinary individual AI handles delegated Systems.

### Resolved choice: what should simplest Cruiser Comms emphasise?

| Option | Form | Trade-off |
|---|---|---|
| A — Conversations and requests | Read/reply to contacts and make contextual requests; ordinary AI handles listening/decoding allocation and jamming/disruption. | Gives direct social and coordination agency with a familiar thread interface, but relies on useful traffic and responsive contacts. |
| B — Listening and interpretation | Inspect public/captured traffic and communicate its meaning; ordinary AI handles routine outgoing dialogue and electronic warfare. | Makes intelligence the entry activity, but delegates much of the seat's direct interaction with other ships. |

**Selected: A.** Simplest Cruiser Comms emphasises reading/replying to contacts and making contextual requests. Ordinary individual AI manages delegated interception/decoding allocation and jamming/disruption. Legitimately available public/captured traffic remains readable; collection assistance does not create omniscient summaries or automatic risk judgements. Human-player contacts retain normal communication channels and existing human-seeking boundaries remain. Exact controls and traffic workload need testing.

### Middle Cruiser Comms — intelligence collection versus electronic attack

Conversations and requests remain the entry activity. The next authored level could add listening/decoding priorities, extending the player's ability to discover useful information, or add jammer/disruption target choices, extending their direct influence during encounters. S066/S067 already select constrained collection and decoding; S018/S021 select equipment-supported interference without remote control of enemy Systems. Neither route adds manual frequency-sweeping or cipher puzzles.

### Resolved choice: what should middle Comms add first?

| Option | Form | Trade-off |
|---|---|---|
| A — Intelligence priorities | Choose where finite interception coverage and feasible decoding effort go; ordinary AI handles electronic warfare. | Builds on reading and interpreting conversations, with useful discovery outside combat, but direct interference decisions arrive later. |
| B — Electronic-warfare targets | Choose jamming/disruption targets and intent; ordinary AI handles collection/decoding allocation. | Adds direct encounter support earlier, but interference can undermine ongoing friendly communications. |

**Selected: B.** Middle Cruiser Comms adds jamming/disruption targets and intent. The user's reason is to give Comms a combat task without requiring deep understanding of the game. Ordinary individual AI handles supported execution and collection/decoding allocation. Preserve equipment geometry, resources and possible friendly interference; S018 already requires relevant tools and ordinary reports to surface known effects. Teach the consequential target/intent decision without requiring technical signal-management expertise. Exact controls and workload need testing.

### Detailed Cruiser Comms — full allocation or intelligence priorities alone?

Middle Comms now supplies accessible electronic-warfare targeting. Detailed complexity can add interception/decoding priorities and also expose independent assignments for fitted jammers/disruption equipment, or keep electronic warfare at middle-level abstraction while adding intelligence priorities. Both use the existing individual System AI and selected mechanics. Neither introduces manual frequency sweeping, cipher puzzles or enemy System takeover.

### Resolved choice: how much electronic-warfare control should detailed Comms add?

| Option | Form | Trade-off |
|---|---|---|
| A — Full allocation | Add listening/decoding priorities and independent target/intent assignments for fitted jamming/disruption Systems. | Allows deliberate allocation across several threats or tasks, but increases concurrent decisions alongside conversations. |
| B — Intelligence depth | Add listening/decoding priorities while keeping electronic-warfare control at the middle level's coarse targeting abstraction. | Concentrates added attention on intelligence, but leaves equipment-level interference allocation delegated. |

**Selected: A.** Detailed Cruiser Comms adds listening/decoding priorities and independent target/intent assignments for fitted jamming/disruption Systems. Ordinary individual AI continues delegated execution; detailed availability does not require constant manual operation of every System. Actual coverage, resources, knowledge, keys/capability and friendly interference constrain results. No privileged information or compulsory technical puzzles are added. Test concurrent conversations, intelligence and electronic warfare.

### Simplest Cruiser Helm — docking assistance

S044/S045 already select desired speed and absolute heading with ordinary actuator assistance; S046 requires a visible HUD toggle for direct steering/thrust/lateral control. These operating modes are already selected, rather than new complexity choices. S049 permits whole-System ordinary AI to handle delegated approach and mating while retaining manual approach. The Station-level question is which docking interaction the simplest authored Helm level presents by default.

### Resolved choice: how should simplest Helm normally dock?

| Option | Form | Trade-off |
|---|---|---|
| A — Request assisted docking | Choose a compatible destination and request docking; ordinary AI handles approach and mating. The human can interrupt and resume steering. | Keeps beginners focused on destination, timing and crew readiness rather than precision alignment. |
| B — Human approach, assisted mating | The human steers into a suitable docking approach; ordinary AI completes the supported final mating operation. | Preserves a hands-on manoeuvre, but demands more precision from beginners. |

**Selected: A.** Simplest Cruiser Helm defaults to choosing a compatible destination and requesting assisted docking. Ordinary individual AI handles approach and mating using actual observations, permissions, equipment and physical/resource limits. The player can interrupt and resume steering; manual approach and the visible direct-control toggle remain available. Assistance does not decide where/when to dock or control Engineering transfers. Exact interruption/handoff controls need definition.

### Assisted docking — returning movement authority to Helm

The selected assistance is a delegated manoeuvre, not a second simultaneous movement controller. Before physical capture, the interface must make it clear how Helm resumes steering. G032 already cancels Match Speed on throttle input, but that does not by itself settle whether docking should use the same takeover convention. The choice below concerns approach assistance, not opening an attached docking connection or bypassing S049's separation process.

### Resolved choice: should movement input cancel an assisted docking approach?

| Option | Form | Trade-off |
|---|---|---|
| A — Input takes over | A deliberate steering/throttle/lateral command cancels assisted approach and takes effect; an explicit cancel control is also available. | Makes urgent takeover immediate, but stray input can interrupt the manoeuvre. |
| B — Explicit takeover first | Use a visible cancel/take-control action before movement inputs take effect during assisted approach. | Protects the manoeuvre from stray input, but adds an action before urgent steering. |

**Selected: A.** Deliberate steering, throttle or lateral input cancels assisted docking approach and takes effect, with a clear assistance-state change. An explicit cancel remains available. The approach does not silently resume; the player must request it again. This concerns the same human-held Station and ordinary AI, not new temporary Backfill behaviour. Actual capture/attachment and separation constraints remain: steering input cannot magically undock the ship.

### Helm complexity — does the seat need a three-step ladder?

The selected flight controls already offer desired speed/heading, Match Speed with manual steering, fitted lateral thrust/boost and a visible direct-control toggle. Assisted docking is optional and interruptible. Unlike Science or Tactical, much of Helm's depth comes from geometry, momentum and coordination using the same controls. G002/G003 select authored Station presets; that does not require every Station to have three mechanically distinct human levels. Navigation routes and fleet formations retain their authored ownership and delegation rules; they must not be moved to Helm merely to fill a complexity tier.

### Resolved choice: should Helm have the same core flight controls at every human complexity level?

| Option | Form | Trade-off |
|---|---|---|
| A — Shared flight core | Retain the same flight controls and optional assisted manoeuvres across human levels; do not invent separate middle/detailed Helm tiers unless additional owned Systems justify them. | Lets skill deepen through flying and coordination, with less interface relearning, but provides less preset-based simplification of the flight surface. |
| B — Staged flight surface | Author distinct human levels that progressively expose optional manoeuvre/mission controls while preserving basic steering and the required visible direct-control toggle. | Can reduce initial interface breadth, but requires explicit choices about which controls to defer and how that affects useful activity. |

**Selected: A.** Helm uses the same core flight controls across human complexity levels. First test the selected controls and optional assistance as one coherent surface; do not manufacture middle/detailed flight tiers. Depth comes from geometry, momentum and crew coordination. Ordinary whole-System AI remains available. Equipment and authored ownership may justify additional controls on particular hulls. Other Stations retain their selected complexity ladders; no per-System complexity menu or transfer of Navigation authority is introduced.

### Detailed Engineering — optional component/team dispatch

Middle Engineering already chooses System restoration priorities while ordinary repair AI schedules component jobs and teams. S075 supplies one team per component, with parallel jobs across a System; S077 lets teams automatically choose full repair or eligible fallback patching. Detailed complexity could expose optional component/team assignments without changing either rule, or retain System-level restoration control and concentrate additional depth in power/cooling and stores.

### Resolved choice: should detailed Engineering allow explicit component/team dispatch?

| Option | Form | Trade-off |
|---|---|---|
| A — Optional detailed dispatch | Inspect known component conditions and optionally assign a particular team to a component job; ordinary AI still schedules delegated work. | Supports deliberate use of team expertise, access and scarce parts, but adds decisions during resource crises. |
| B — System priorities only | Keep repair control at System priorities even at detailed complexity; ordinary AI always chooses component jobs and teams. | Keeps repair management compact and focuses added depth elsewhere, but limits intervention in specific restoration bottlenecks. |

**Selected: A.** Detailed Engineering allows optional assignment of a particular repair team to a known component job. Ordinary individual AI continues scheduling delegated repairs and routing teams. One-team-per-component remains, and teams still choose the supported remedy; no manual patch/full-repair selector or diagnostic puzzle returns. Actual evidence, parts, access and commitments constrain work. Test concurrent fine power/cooling, repair and stores decisions before treating the detailed bundle as balanced.

### Engineering stores — when human supply priorities enter

The reference Engineering seat owns main hold stores, reserves and external supply priorities; local operators retain their own local stock targets and Comms negotiates terms. Simplest Engineering delegates stock handling without required setup. G065/G067 already select timed automatic replenishment against targets, with manual initiation of trades where required. The open complexity question concerns human control of Engineering's supply priorities, not manual movement of every item or authority over another Station's local settings.

### Resolved choice: when should human stores/resupply priorities enter Engineering?

| Option | Form | Trade-off |
|---|---|---|
| A — Middle | Add main-store desired stock levels, reserves and external resupply priorities alongside System restoration priorities; ordinary AI handles routine movement. | Connects repair choices with material planning and supports non-combat work, but broadens middle-level attention. |
| B — Detailed | Keep these supply decisions delegated at simplest/middle; expose them with detailed repair dispatch and fine power/cooling. | Keeps middle focused on power and restoration, but defers deliberate supply planning to the busiest level. |

**Selected: B.** Human main-store stock goals, reserves and external resupply priorities enter at detailed Engineering complexity. Simplest/middle delegate those decisions to ordinary individual AI. Routine movement remains automatic; local operators retain their local stock targets. Existing trade initiation and authority, actual stocks, access and delivery time remain constraints; external craft commitments still belong to Tactical. Test the combined detailed workload.

### Middle Engineering power — applying the provisional reactor ladder

The user's provisional ladder distinguishes minimum priority-ordered groups with no/low/high coolant usage, middle several individually allocated power/coolant groups, and maximum many fine-grained groups. The Station review has adopted the minimum form for the first manual layer and added System restoration priorities at middle. Confirm whether middle also adopts the proposed individual coarse-group allocations or retains minimum-style power control to leave attention for repairs.

### Resolved choice: how should middle Engineering control power/cooling?

| Option | Form | Trade-off |
|---|---|---|
| A — Individual coarse-group allocations | Allocate power flow and coolant separately to several authored groups, as in the user's provisional middle reactor design. | Gives direct reserve/thermal trade-offs while ordinary AI handles allocation inside each group, but adds ongoing attention alongside repair priorities. |
| B — Keep priority-list allocation | Retain minimum-style priority ordering and no/low/high coolant usage; middle depth comes from restoration priorities. | Reduces simultaneous resource-control demands, but defers direct group allocation to detailed complexity. |

**Selected: A.** Middle Engineering allocates power flow and coolant individually to several authored groups. Ordinary individual AI handles supported distribution inside groups. Flow still replenishes local batteries and cooling addresses actual heat; this is an authored Station level, not a per-System complexity menu or different simulation. Test concurrent resource allocation and restoration priorities. Exact group membership, controls and detailed grouping remain under review.

### Power-group continuity across complexity levels

Minimum Engineering prioritises several groups and middle allocates resources to several groups. The provisional maximum level exposes many finer groups. The relationship between these authored groupings affects whether operators can understand the same ship when changing complexity, without changing its physical Systems, conduits or resources.

### Resolved choice: should finer power groups subdivide the coarse groups?

| Option | Form | Trade-off |
|---|---|---|
| A — Consistent hierarchy | Minimum/middle use the same coarse group membership; detailed control subdivides those groups into finer authored groups. | Preserves a recognisable resource structure across levels, but constrains authors to nested groupings. |
| B — Independent authored groupings | Each Station level can group the same Systems differently, with an explicit mapping to physical consumers. | Offers more freedom to optimise each interface, but makes transitions harder to explain and reconcile. |

**Selected: A.** Minimum/middle share coarse group membership; detailed control subdivides those groups into finer authored groups. Grouping is a control/AI-budget abstraction over the same physical consumers, not additional power or a new physical conduit hierarchy. Preserve individual System AI and a single effective allocation per consumer. Exact groups and allocation handoff remain under review.

### Cooling allocation — capacity versus continuous expenditure

G055 selects finite consumable coolant and S034 optional lossy recycling. S035 retains passive heat rejection without a separate controllable radiator. The exact relationship between allocated cooling and coolant expenditure remains open. It matters for stable preparation: a cooling allocation could reserve heat-removal capacity and spend stock only when removing heat, or continuously circulate/spend stock whenever enabled.

### Resolved choice: when does an allocated cooling flow consume coolant?

| Option | Form | Trade-off |
|---|---|---|
| A — On actual heat removal | Allocation limits cooling rate; coolant expenditure follows actual heat removed. A cool System does not spend coolant merely because cooling capacity is allocated. | Lets stable cooling settings remain useful without idle waste, while active work still consumes finite stock. |
| B — Whenever flow runs | Allocated coolant flow consumes stock while enabled, including when little heat needs removing. | Adds a reason to trim idle cooling, but risks recurring shutoff/restart chores. |

**Selected: A.** Allocated cooling limits heat-removal rate, and coolant expenditure follows actual heat removed. A cool System does not spend stock merely because capacity is allocated. Reserved capacity remains unavailable elsewhere until reallocated; no automatic redistribution is implied. Active cooling still consumes finite stock and optional recycling remains lossy. Exact rates, limits and operating costs remain open. This is a Phoenix proposal choice, not a claim about the research games.

### Thermal protection — default response to overheating

G060/S078 already allow overheating to cause stress-dependent wear. S037 supports equipment-specific overdrive, risk policy and potential shutdown/recovery rules, while S038 distinguishes thermal shutdown from a generic empty-battery restart lock. The default response to dangerous heat remains open: equipment can protect itself by reducing/stopping output, or maintain requested output while warning the operator and taking actual stress consequences.

### Resolved choice: should normal equipment automatically protect itself from dangerous heat?

| Option | Form | Trade-off |
|---|---|---|
| A — Protective default | Equipment reduces/stops output at authored thermal limits; supported overrides let its operator accept higher risk deliberately. | Gives beginners recovery margin and makes overrides consequential, but can interrupt an important action. |
| B — Warning default | Equipment continues requested output, with clear heat warnings and actual wear/damage consequences; reducing demand is the operator's responsibility. | Preserves immediate output, but unattended overload can become destructive more quickly. |

**Selected: A.** Normal equipment reduces/stops output at authored thermal limits. Equipment-supported overrides let the owning operator accept increased risk deliberately. Engineering supplies flow/cooling and communicates limits rather than gaining remote authority over human-held Systems. Safeties do not remove accumulated heat, guarantee survival after cooling damage or create universal immunity to thermal wear. Exact thresholds, recovery, override availability and failure consequences remain open.

### Thermal recovery — automatic resumption versus restart acknowledgement

A protective stop is now the normal response where equipment reaches its authored thermal limit. Recovery can clear that protective state once sufficiently cool, or require an operator restart after cooling. This is distinct from repairing actual component damage or overriding an active safety limit; neither option restores missing parts or cancels a deliberate off command.

### Resolved choice: should an ordinary thermal stop clear automatically after cooling?

| Option | Form | Trade-off |
|---|---|---|
| A — Automatic recovery | Clear the thermal stop below a lower recovery threshold and resume only as existing enabled state, orders and current authority allow. | Avoids routine restart clicks and rapid threshold oscillation, but the recovery state must be obvious. |
| B — Restart required | After cooling, remain stopped until the owning operator or authorised ordinary AI issues a restart. | Makes resumption deliberate, but adds a recovery action for each protective stop. |

**Selected: A as the normal rule.** Ordinary thermal stops clear automatically below a lower recovery threshold; separate trip/recovery thresholds provide hysteresis. Resume only as current enabled state, orders and authority permit. Clearing protection does not restore cancelled actions, override a human stop, invent a new target or bypass actual damage, energy and preparation requirements. Ordinary individual AI continues using current knowledge/orders. Equipment-specific startup requirements remain possible where justified; exact thresholds and exceptions remain open.

### Power routing response — avoid duplicating the preparation delay

S031 already selects reactor output with finite response and its own capacitor reserve. That decision remains unchanged. Local System batteries also take time to replenish through allocated flow. The remaining question concerns changing distribution settings on healthy links: whether the requested routing changes promptly within available supply, or each distribution link adds its own normal ramp delay.

### Resolved choice: should healthy power-distribution allocation changes add a separate ramp delay?

| Option | Form | Trade-off |
|---|---|---|
| A — Prompt routing | Apply new distribution requests promptly within actual supply/link/headroom limits; reactor response and battery charging provide the preparation delays. | Keeps resource controls responsive and causal while retaining the need to build reserves. |
| B — Link ramping | Healthy links also take time to reach newly requested flow, with equipment-specific ramp limits. | Adds another anticipation constraint, but compounds reactor and battery delays and makes crisis rationing slower. |

**Selected: A.** Healthy distribution requests respond promptly within actual supply, link and battery-headroom limits, without a separate routine link ramp. Reactor response and battery charging still provide preparation delays. Damaged/intermittent links can constrain delivery; prompt routing cannot create supply or instantly fill reserves. This does not change S031's finite reactor response, auto-redistribute unaccepted flow or override another System's output authority. Exact limits and update behaviour remain open.

### Manual power allocations — handling a supply shortfall

Middle/detailed Engineering sets explicit group flows, but reactor ramping, thermal protection or damage can reduce available supply below those requests. Local batteries buy response time without eliminating the shortfall. Minimum-style priority-list allocation remains selected; this question concerns the fallback when explicit manual allocations cannot all be met. Adaptive distribution inside a group remains S033.

### Resolved choice: how should explicit manual group allocations share insufficient supply?

| Option | Form | Trade-off |
|---|---|---|
| A — Proportional reduction | Reduce requested group flows by the same proportion, subject to actual link/headroom limits; show requested versus delivered flow. | Preserves the operator's relative allocations without another policy to configure, but every supplied group may lose flow. |
| B — Priority protection | Apply a fallback group priority order, satisfying higher-priority requests first and cutting lower groups. | Protects selected critical uses automatically, but adds a second decision layer alongside explicit flow settings. |

**Selected: user-supplied C — random cuts to 0% until the remaining allocations fit available supply.** The user explicitly clarified: cut incoming flow, not the System. Affected Systems can continue consuming local battery reserves under ordinary operating rules. Do not substitute proportional scaling or automatic priority protection. Show cut allocations and actual delivery so Engineering can react and communicate. Minimum-style priority allocation remains unchanged by this explicit-manual-allocation decision. Exact eligible groups, restoration and anti-oscillation behaviour remain open; no power is created or battery costs removed.

### Flow-cut recovery — operator restoration versus automatic retry

Random shortfall cuts now set incoming allocation flow to zero, leaving local batteries to sustain operation. The next dependency is whether that cut persists as an operator-reset state or the previous request can recover automatically once supply permits. This is a power-routing state, not a System shutdown or thermal recovery event.

### Resolved choice: how should a shortfall-cut flow be restored?

| Option | Form | Trade-off |
|---|---|---|
| A — Operator restoration | Keep the affected flow at zero until Engineering explicitly reallocates it; ordinary AI can do so when that allocation is delegated. | Makes the cut stable and forces deliberate rerationing, but creates restoration work after supply returns. |
| B — Automatic recovery | Retain the requested allocation and restore it when supply can sustain it, with hysteresis to avoid repeated cut/reconnect cycling. | Reduces restart administration, but requires clear pending requests and controlled restoration behaviour. |

**Selected: B.** Preserve the manually set flow values while actual delivery is cut. Flows return automatically as available power can handle those requests. Keep requested and delivered flow distinct, with hysteresis against repeated cut/reconnect cycling. Engineering can still edit/cancel requests. Exact eligible groups, restoration order and thresholds remain open. Ordinary individual AI and the same resource constraints remain unchanged.

### Flow restoration order — waiting time versus random recovery

Several cut requests may be waiting while supply recovers. Each candidate must fit available headroom before restoration; the user's manual settings remain stored. The next choice concerns which feasible request is considered first, not reducing its stored value or cutting a currently supplied group merely to rotate service.

### Resolved choice: in what order should feasible cut flows recover?

| Option | Form | Trade-off |
|---|---|---|
| A — Random eligible request | Choose randomly among cut flows that can currently be sustained. | Matches random shedding, but makes waiting order less predictable. |
| B — Longest waiting, if feasible | Restore the longest-waiting cut request that fits available headroom; a larger request cannot block all smaller feasible ones. | Gives a stable, explainable recovery order, but large requests may still wait for enough supply. |

**Selected: A.** Choose randomly among cut flows that available supply can currently sustain. Do not substitute waiting-time or priority ordering. Stored manual values remain unchanged; Engineering can edit them. Do not evict supplied flows merely to rotate service. Exact feasibility/headroom accounting and hysteresis remain open.

### Random shedding eligibility — critical equipment

The selected overload rule cuts incoming flow, not System output. Local batteries and compartment survival margins provide time to respond. Eligibility remains open: all supplied groups could participate, or authors could exempt critical groups such as life support from random shedding. An exemption cannot guarantee power when actual supply is insufficient and needs defined behaviour in that case.

### Resolved choice: should any power groups be exempt from random flow cuts?

| Option | Form | Trade-off |
|---|---|---|
| A — No critical-group exemptions | All supplied allocation groups are eligible; local reserves and Engineering's rationing protect important work. | Keeps the rule consistent and makes readiness consequential, but critical Systems can lose incoming supply. |
| B — Authored protected groups | Hull data can exclude selected critical groups from ordinary random shedding, with an explicit fallback if protected requests alone exceed supply. | Protects critical uses first, but adds exception rules and an additional shortage case. |

**Selected: A.** No critical-group exemptions: all supplied allocation groups are eligible, including life support. Local batteries and actual survival margins provide response time; validate that margin rather than silently introducing protection priorities. Show affected groups clearly. This concerns allocation groups on the distribution network, not an instruction to cut the reactor's internal operation or shut down connected Systems. Exact reserve sizing and shortage feedback need testing.

### Review checkpoint — catalogue coverage versus design validation

All **114 game-system entries and 102 ship-system entries (216 total)** have completed their first ordered review. No catalogue entries remain unreviewed. Subsequent questions have developed Station ownership/complexity and cross-System resource rules; those are synthesis and refinement, not a remaining count of untouched Systems. Completing that refinement has no reliable fixed question count. The next useful milestone is an end-to-end combat and non-combat activity walkthrough for the selected mixed-depth six-seat crew, exposing gaps and overload before continuing low-level refinement. Walkthroughs are design hypotheses, not implemented scenarios or playtest results.
