# Increment 16 — Shared Comms channels and remote requests

Adopted contract preparation, 15 October 2026. The user approved continuation
through 24 and dependency-ready parallel work. Runtime entry requires 15's initial
integrated implementation and targeted checks; full completion depends on 14/15's
verified exits. This document changes no ledger status or acceptance result.
The integrator's [parallel-work contract](parallel-work.md) governs dispatch.

## Outcome and selected scope

Initial text-radio checkpoint: optional `[comms.radio]` channels now use the
existing Comms System, real energy consumer and recipient-scoped inbox. The
four Alliance hulls have public/protected channel profiles and the existing
Comms screens have one shared text composer. Local sent records and actual
received copies remain distinct; a send result cannot claim reception.
Recipient-copy identity does not advance a shared counter conditionally.

This is partial implementation. Generic requests/receipts, automatic AI chatter,
capture/decoding, relays and selected human media remain to be implemented.
Workshop radio-form work is in its parallel authoring slice. Positive continuous
listening cost 0.05/sec and public/fleet discrete work 1/4 are provisional data,
not balance acceptance. See `comms-radio-checkpoint.json` for exact evidence.

Let human and AI crews converse, request useful action and learn what a distant
recipient actually acknowledged, through equipment-supported in-world channels.
Keep concurrent persistent threads and preserve local specialist authority.
Phoenix signalling, peer topology, delivery classes and command admission remain
the transport foundation. Gameplay links and receipts are game state, not
transport acknowledgements or a second connection protocol.

The selected catalogue, architecture AD70–74 and
[Comms contracts](../../../pasm/spec/design/comms-channels.yaml) govern scope.
Research is motivation; no competitor timings or additional mechanics are adopted.

| Selected entries | Delivery and limits |
|---|---|
| S065; G022 refinement | Concurrent persistent plot and generic conversations, urgency, human text and selected human audio/video focus. Every supported ship can receive contextual generic requests independently of plot dialogue. AI uses text and ordinary urgency cues; no AI voices. |
| S069; G043 | Individual, own-fleet, per-faction and all-ship audiences. Each recipient independently receives, considers and responds. No arbitrary custom groups, remote actuator control or guaranteed obedience. |
| S066; G024 | Systemic supported traffic plus authored transmissions; automatic public chatter and finite prioritised listening/search coverage. Reception must actually occur; no retrospective uncaptured-message retrieval. Enemy player text is interceptable under the same supported gameplay rules. |
| S067; G024 | Protected channels and decoding constrained by actual capability and held keys. Time cannot defeat every protection. Ordinary AI handles feasible work; no cipher puzzle or change to transport security. Audio/video interception remains unselected detail, not an automatic plaintext/media grant. |
| S101; AD72–74 | Equipment-backed automatic relay routing, explicit pending-order validity/supersession and returned delivery/decision receipts. No message-capacity, traffic-budget or volume-congestion resource. |
| G040/G041; S068 | Known actor/faction relationships affect supported willingness; retain dated attributed interactions and content-authored terms/settlement. Generic chat does not create a universal contract, morality score or automatic commitment-risk warning. |
| G042/G044/G109 | Surrender can change supported intent without magical compliance, capture or despawn. Claims, acceptance and actual conduct remain distinct. Pursuit information follows transmitted evidence, not hidden destinations; transit and broader capture consumers join 18/20/23. |

Reference Cruiser Comms owns conversation, requests and signal interpretation;
Engineering retains stocks/service priorities, Tactical physical commitments,
and each receiving specialist its own machinery. Later 17 assigns jamming and
disruption to Comms and electronic protection to Science through authored data.
Preserve 2/4/6/8-seat hull targets, visiting/human-seeking authority and Captain's
lack of a private map. Simple Comms prioritises conversation/requests; later
middle/detailed electronic-operation controls do not become 16 prerequisites.

## Extend actual owners

The first structural migration scopes issued hail history to its actual source
ship. Human and ordinary AI hails use the issuing fleet slot's existing range
gate, and each crew clears or retires only its own issued history. Preserve
single-ship plot events and dialogue outcomes. This migration adds no channels,
generic requests or media by itself; those remain the subsequent consumers below.

Current `console/comms/server.rs` applies admitted hails/replies and publishes the
ship audience; `console/comms/inbox.rs` owns chronological messages/read state.
`comms/server.rs`, `range.rs` and `roster.rs` own endpoints/reach; `scripted.rs`
and World `comms/content.rs` retain authored dialogue and pending opens.
`recipients.rs` already resolves addressed audiences. Extend these owners and
their existing snapshot continuation rather than adding a second inbox or
scenario engine. An equipment/channel ledger may be a new leaf of this same
Comms owner; it must not shadow existing message/thread/decision state.

Model owns closed channel/request/receipt/protection vocabulary and DTOs.
Gameplay owns pure link/order/decoding rules and per-System tree bindings.
World owns authored dialogue, actor objectives and content-specific effects;
Session owns participation/admission. Simulation joins actual equipment,
resources, fields, terrain and recipients without sibling-domain imports.
Browser controls render permitted projections and emit ordinary correlated
System commands. The host composes existing Phoenix transport; it does not
interpret player prose as structured machinery commands.

Migrate existing hail, reply, addressed script dialogue and shared 14 information
links first with equivalent behaviour. Preserve explicit synthetic narrative/GM
delivery separately from physically emitted traffic; it must not manufacture a
physical emitter or universal link bypass for ordinary requests.

## Channel, evidence and delivery contract

Each transmission has one stable causal identity, actual issuing endpoint/System,
origin audience/channel, emission tick, immutable content/protection and optional
structured request. Copies retain original identity, source and observation time.
Private routing IDs are not automatically public identities; communicate only
permitted endpoint labels/contact references and held payload evidence. A relay
cannot upgrade a claim to observation or turn decoding into physical identity.

Author supported channels/security and actual transceiver/listener/decoder/relay
capabilities on existing Systems, with real energy, condition, placement and
Console scope. First consumers may reuse the existing Comms System; distinct
fitted instruments require their own actual owners, not virtual claimable gear.
Physical sampling uses canonical position, existing signal-path attenuation and
terrain tests. Every hop checks both endpoints' current operability, reach,
compatible channel/protection and permissions. Conceal/deliberate disclosure
retain their relevant current observation rules; a known endpoint does not grant
its private scene state. Central host knowledge is not receipt evidence.

Routing chooses a deterministic viable equipment-backed path, with stable tie
breaking and cycle prevention. Retry pending supported traffic when paths change;
do not mint duplicate deliveries or side effects. Relay forwarding preserves
origin/security and does not expose protected contents to a relay without actual
reception/decoding permission. Technical payload bounds are validation limits,
not a simulated per-second bandwidth pool. Resource payment is actual operation
of equipment/work, never message-volume rationing.

Listening coverage limits which simultaneous signals/channels/areas an instrument
can acquire, not how many messages its inbox can hold. Unsupported/ciphertext
findings remain dated evidence. Decode only captured material under its authored
capability/key policy and paid work. New keys can enable retained captured work;
they cannot recover traffic never received. Specify tuning with the first real
equipment and mark it provisional; do not invent a universal decode delay here.

Human text uses the same channel eligibility/history as AI text. Human media is
an explicitly selected live conversation using the existing host/session and
Phoenix connection architecture; microphone/camera consent remains local. Signal
loss ends/pauses actual media delivery without erasing text/history. Do not save
media streams or replay camera/microphone permissions on restore. Media focus is
presentation intent, not authority over recipient Systems. Any necessary media
transport envelope must be reviewed against current Phoenix capabilities before
implementation; it cannot silently add another broker, authority or gameplay bus.

## Structured requests, decisions and receipts

Use one closed request definition per supported real action. Initial verbs reuse
existing hold/divert/assist/retreat or report consumers where actually supported;
do not expose an action without an ordinary receiving consumer. Each definition
declares parameters, eligible audience, validity, supersession scope and actual
receiving owner. Content-specific surrender terms remain separate authored work.

On send, resolve fixed audience membership deterministically once and create
separate recipient envelopes/outcomes. Later faction/roster changes cannot rewrite
old audiences. A fleet send does not disclose hidden members or readiness. Each
receiver rechecks current authority, willingness, objectives, knowledge, relevant
reading time and feasibility before accepting. Human-held authority requires the
normal human decision; an AI recipient uses its ordinary individual System tree.
Accepting a request may set permitted local intent, never bypass real jobs,
resources, access, protected choices or another human's controls.

Persist origin request ID, recipient, issuing sequence, known supersession scope,
emission tick and authored expiry/persistence. Newer intent supersedes only where
it has arrived and the scope/version is known. Reject duplicate/reordered older
effects idempotently. An unseen replacement/cancellation cannot alter remote
work. Expiry applies to undelivered/unaccepted intent; already accepted onboard
work continues under its existing rules on link loss. A durable mission order
can explicitly persist; players need not choose timeout numbers for every send.

Keep these outcomes separate in ordinary records:

1. Local pending/sent/expired facts describe the sender's own queue.
2. Delivery receipt means the recipient accepted the envelope, not interpreted it.
3. Reading/interpretation is recipient-owned progress, using authored semantic
   ticks and existing per-System reading/tree continuation.
4. Accepted/refused is the recipient's dated decision with a supported reason.
5. Completion/failure is a later actual consumer report or observation.

Return each receipt through a currently viable permitted link; retain pending
return information across outages. Missing receipt leaves remote state unconfirmed,
including where the recipient already acted. Transport acknowledgements, local
timeouts and host-private job state cannot substitute. Receipt replay never repeats
acceptance or completion effects. Group displays retain per-recipient outcomes.

## Adopted first request: Report Status

The first closed generic request is `ReportStatus`, delivered through the actual
paid radio path. It has one real receiver: the addressed ship's ordinary Comms
System. It does not issue course, weapons, repairs or other specialist commands.
Those request kinds require their own receiving consumers in later slices.

Immutable transmitted content distinguishes text, a Report Status request and a
returned outcome. A receiver's existing inbox copy separately retains its local
read, decision and completion progress. There is no second request inbox or a
second controller. Fixed Report/Decline response options use the existing admitted
`RespondToMessage` path alongside unchanged plot responses. Read means an actual
human read action or completion of the ordinary AI's authored reading delay;
receipt alone cannot mark a request read or accepted. Reporting exposes only facts
permitted to that Comms Console. An alert condition is not implicitly shared.

Local paid emission establishes sent only. Automatic mechanical delivery receipts
and subsequent read, decision and completion claims return through independently
viable paid links. Each claim preserves its original event tick and references a
request actually heard by that reporting source. A Report decision may produce
its factual report in the same local step, while acceptance and completion remain
distinct records. A viable link or paid return emission is not remote acceptance.
No return path leaves the sender's remote outcome unresolved; missing recipients,
unsupported actions, link loss or timeouts cannot manufacture a refusal oracle.

This first request is persistent, has no automatic retry or time-based expiry,
and does not supersede another request. Each deliberate send is a new causal
request. Pending returned claims remain on the receiver's existing inbox copy and
can be retried automatically when the real link becomes viable, preserving one
claim identity and never repeating the original local decision or report. Clearing
a local conversation is an explicit local dismissal, not an unseen remote cancel.
Request copies retain a local dismissed flag on the same canonical inbox record;
Clear hides eligible rows without creating read/decision/completion facts,
cancelling work or discarding pending returns. Actual lineage remains until session
end in this persistent slice. A new returned stage is a new visible outcome row
and does not silently un-dismiss the old request. Return delivery is a sustained
reliable service: discrete work admits each immutable local Outcome transmission
once, independent of whether the requester hears it. The same retained copy and
actual continuously paid radio operation fund later delivery attempts, each gated
by both actual endpoints, compatible channel/key and physical path. Reception is
idempotent. The internal returned frontier cannot alter reporter-visible work,
payment, availability, sent history or AI choices and is omitted from permitted
publication. Near/far histories and reserves must match through delayed delivery
and exhaustion. Ordinary text retains its existing one-shot semantics. No extra
retry queue, work credit or public private-acknowledgement field is introduced.
Ordinary text/plot Clear is
unchanged; there is no separate archive or expiry/garbage-collection success.
The same Console continues receiving its own retained records; a Show cleared
requests filter reveals them. Default hiding does not revoke human knowledge while
AI uses retained history, and it never pools another Console's records.
Technical record bounds are validation limits, not message-capacity accounting.

Extend the existing response AI and pending-response reading map to actual ship
UUID ownership. Preserve the legacy fleet-slot sweep and its decision/draw order;
process additional non-roster actors afterwards in deterministic UUID order.
Host slots remain derived legacy routing context. A human-held Comms System
stands the same AI down; handoff resumes the retained inbox and reading frontier.
No text parsing, independent polling controller or alternate remote authority is
introduced. Supported request facts enter the current response policy.
The heard typed `ReportStatus` copy supplies the closed
`request_report_status` fact to that existing policy. Shipped response policies
explicitly choose Report only from this known request, actual read completion
and own Comms availability. They do not require or fabricate current remote
`sender_in_range` for generic requests. The legacy plot rule and weighted branch
keep their existing facts, precedence and RNG sweep; the new fact is absent
there. Custom policies can explicitly Report/Decline or retain the request.

Request reading uses explicit positive authored `request_reading_ticks` and a
request-only remaining/last-tick frontier inside the existing pending response
entry. Mechanical elapsed-tick advancement counts only eligible AI-controlled
intervals, pauses under Human/Offline and preserves partial progress on handoff.
It makes no choice and creates no second timer map or controller. The existing
weighted dialogue delay branch and its draw order remain unchanged.

Complete-candidate and single-row validation bind request identity, source,
recipient, reporting source, immutable content, local progress and original clock
frontiers before any write. Returned claims cannot predate the events they report,
claim stages that did not occur locally, or refer to an unrelated request. Preserve
ordinary inbox/plot continuation and existing deterministic ID/delivery rules.

The authored proof must include actual paid request/reverse delivery, elapsed AI
reading followed by an admitted response, a human Report/Decline path, asymmetric
return loss with identical local sender facts, and concurrent plot dialogue. Fresh
JSON restore must preserve pending reading and return work for at least 360 steps
with exact complete captures/digests; malformed cross-request/stage/clock claims
must leave the live host unchanged. Different-LocalShip peers must agree. Source
review and owner tests alone do not establish this playable exit.

## Adopted first equipment-backed relay

The first relay is an optional Boolean `relay` on an actual authored radio
channel, defaulting false. It uses that same installed Comms System and its
ordinary continuously paid operation, condition and exact receiver protection.
A channel-compatible endpoint with positive current paid operation may forward;
intermediate endpoints must explicitly enable relay. Protected forwarding in this
first slice requires the relay's actually held matching channel key. Opaque
forwarding without a key and captured-message decoding remain later consumers.

At each delivery boundary derive a direct-first, shortest-hop viable route,
using stable endpoint UUID order to break equal-length ties. Every hop uses both
actual endpoint ranges and the existing physical terrain/field/interference path.
Cycles are excluded by visited endpoint identity. There is no route cache,
reservation, transit queue, extra controller, per-message forwarding payment or
message-capacity accounting. Continuous operation funds this supported service;
no relay means no additional work beyond the existing radio owner.

The recipient receives the original immutable emission and original sender; a
relay does not become the sender, invent observations, obtain an inbox copy just
by forwarding or disclose route membership to source/recipient controls. Address
eligibility stays between the original source and final recipient. One-shot text
and initial requests use the route available on emission. Retained returned
outcomes use their existing sustained service and can arrive after a real relay
returns, without paying another discrete transmission or repeating a decision.
Current endpoint sampling and complete deterministic state retain their existing
owner. Removing power, disabling equipment, incompatible keys or a blocked hop
removes the derived route at the next ordinary radio boundary.

Workshop edits the same optional channel field through the accepted source/history
transaction. The first reference is an optional relay Cruiser with unchanged
base energy and a complete materialised radio profile. Real three-endpoint tests
must prove blocked direct reach restored by an actual relay, relay power/condition
loss, incompatible protection, cycles and deterministic equal-hop selection.
Actual host continuation/media and different-LocalShip evidence remain separate.

Relay source checkpoint: the shared radio/returned-outcome consumer, optional
reference profile and accepted structured Workshop checkbox are implemented.
Targeted Vitest: 17 tests in three files passed (24.84s); strict strings: 6,148
entries, zero errors/warnings; layer rules and diff check passed. Independent
read-only runtime review by the craft lane passed. Native compilation, actual
three-endpoint tests and host continuation are pending integrator execution;
this is not a playable exit or balance acceptance.

## Adopted planned Report Status initiation and retained delivery

The first planner communication consumer reuses actual Report Status rather than
creating a communication Job. A World-authored released, unaccepted request names
an issuing actor, actual Comms System, channel and declared base-World recipient.
Simulation may resolve that authored reference privately, but publishes a typed
`planned_requests` row only when the same UUID already exists in the issuing
Console's current permitted Comms contact view. Each row contains the Situation
identity, original release tick, already-known recipient UUID, supported channel
and closed request kind. Missing contacts withhold execution without revealing a
private route or granting a contact. Slot-relative recipients are explicitly
unsupported in this first join.

The Human uses ordinary correlated `SendCommsRequest`, with an optional
`planned_request` identity selected from that publication. The existing Comms
response decision host gains a closed `comms_request` policy output for the same
own published request; it uses the existing per-System gate, authored policy and
cadence. It emits that same admitted command. There is no new System controller,
private AI latch, World automatic send or change to legacy plot weighted draws.
Planned work also waits for the radio's positive authored reading ticks through
the SAME existing pending-response map. Its key distinguishes an inbox message
from a planned Situation, avoiding legal-identity collisions. Reuse the existing
remaining/last-tick reading frontier, counting only eligible AI intervals and
retaining partial work under Human, Offline or missing-current-contact periods.
A completed planned reading may wait for actual payment; it does not create a
recipient-read receipt. Successful acceptance removes that pending entry in the
same transaction. Candidate validation binds it to its actual released,
unaccepted Situation and accepted radio/Console owner. No extra timer map or
private initiation latch is introduced.

First selection is stable released-tick/identity order. Human or absent policy
stands ordinary AI down; invalid/unavailable requests hold under ordinary rules.

Before payment the radio consumer revalidates the exact current published row,
recipient knowledge, channel/kind and released unaccepted Situation binding.
The World adapter returns an exclusive prepared acceptance token without writing.
Failed operation/knowledge/payment drops it unchanged. Actual successful discrete
emission creates the canonical source inbox record and binds the consumed token
in the same transaction to a typed actual transmission scope: issuing actor,
Comms owner, minted origin identity, actual recipient and original emission tick.
It does not mint a JobId, an imagined receipt or a permission effect. Accepted
work leaves the unaccepted request publication, so there is no second AI latch.

Only this validated planned Contact request sets immutable `sustained=true` on
its original transmission. Its same retained source inbox copy retries delivery
through current paid continuous radio operation and actual compatible physical
routes. It pays discrete emission once; stable original/recipient copy identity
prevents repeat delivery/read/decision effects. No secondary queue, retry credit,
transport acknowledgement, hidden-receiver feedback or message capacity exists.
Manual generic requests and text retain their existing one-shot delivery;
returned Outcomes retain their existing sustained service. Clearing structured
history still dismisses without cancelling work or deleting lineage.

World may acknowledge original reading/report facts only after the issuing actor
actually hears a matching returned Outcome, using its already-established scoped
lineage and original read/report ticks plus the actual return receipt tick. A
Completed report can also carry reading evidence when no separate Read-only
receipt was emitted. Neither a report nor acknowledgement grants permission;
only an explicitly authored subsequent Scenario effect may do that. Full snapshot
preflight binds every sustained planned request to its exact accepted transmission
and accepted authored Situation before any write. Typed physical Job and
communication transmission execution remain distinct variants of the existing
activity owner, with no copied job or remote-private progress census.

The first proof must run ordinary AI initiation and Human initiation through
normal admission, real paid relay/link loss and recovery, original-clock returned
evidence, explicit permission and hazard consequences, fresh JSON continuation
for 360 complete capture/digest steps and malformed-context no-write refusals.
Authoring, browser, different-LocalShip and measured workload evidence remain
separate from this adoption and source integration.

## Ordered implementation and parallel seams

1. **Shared threads and equipment links.** Preserve actual current hails/dialogue;
   add human text and one AI generic request to a real existing recipient. Prove
   concurrent plot/generic threads, dated history, explicit audience and field/
   terrain/range failure. Consolidate the current two Comms policy entry points
   under one existing per-System tree evaluation, not two competing controllers.
2. **Requests and returned receipts.** Exercise one real consumer, ordinary Human
   acceptance/refusal and delegated AI with authored reading time, pending expiry,
   known supersession and asymmetric return loss. Expose shared status/feedback.
3. **Public/protected traffic and decoding.** Author actual finite coverage,
   compatible protected channels, a captured undecodable transmission and a real
   key/capability-assisted case; test player text with the same rules.
4. **Relay and human media.** A real powered relay restores a blocked route; loss
   stops new delivery without undoing accepted work. Exercise text plus selected
   audio/video focus using supported current connections and actual permissions.
5. **Shipped integration and actor knowledge.** Migrate authored/NPC/PC/GM/thread
   routes without plot regressions; show contextual willingness and behavioural
   surrender without inventing capture or an omniscient faction response.

An integrated 16 channel/link/request consumer and fixed shared contract unblock 17
electronic operations and 18 formation delivery before 16's full exit. 19 later uses
the same remote order/receipt/link-loss contract; no temporary craft controller.
20/21/22 may add real action adapters without changing Comms into a planner.
Coordinate Model dispatch, central schedules, snapshot schema/digest and shared
Workshop wiring through the integrator. Fix explicit same-tick sample/send/
deliver/read/decide/receipt/publication edges in code; Workshop does not schedule
processors. Existing AI cadence and one controller per actual System remain.

## Authoring, continuation and exit evidence

Workshop must add/edit/remove supported channel, coverage, protection/key,
request validity and relay capabilities through accepted source/history, with
strict finite/closed validation, includes/materialisation, Preview and actual Test.
World scripts author dialogue/public traffic/contextual request policies and known
relationship/commitment outcomes through existing effects; no embedded arbitrary
runtime program or universal negotiation planner.

Capture canonical envelopes/order frontiers, recipient decisions, pending return
receipts, captured evidence, paid decode/reading work, held keys and actual dated
trust events in their owners. Derived route/reach/presentation caches rebuild
before first command after restore. Preserve queue order and per-recipient audience.
Full-candidate preflight validates owners, channels/scopes, sequences/deduplication,
time/expiry/evidence/work frontiers and immutable capability bindings before any
write, including materialised/missing actors. Versions change coherently with the
integrator; old incompatible saves/packs are refused or explicitly converted.

Required proof includes actual Human/AI same-command paths; no guessed hidden
recipient/action; duplicate/reordered/expired request refusal; asymmetric links;
relay cycles/ties/loss; captured-versus-never-received decoding; permissions/key
changes; concurrent threads and protected choices; accepted work after link loss;
and receipt/knowledge distinctions. Save paid reading/decoding and pending orders/
receipts, JSON-restore fresh hosts and compare complete captures/digests for at
least 360 subsequent fixed steps; prove two peers with different LocalShips agree.
Tampered candidate owners/scopes/clocks/outcomes must refuse without live writes.

Record authored corpus/Workshop round trips, native renderer-independent owner
tests, built browser text/media journeys, unchanged networking/layer checks and
appropriate activity/performance comparisons. Test 2/4/6/8-human layouts and
delegation, simultaneous conversation/other duties and known overload. Automated
or source-only proof cannot claim physical-device/media or human workload
acceptance. Initial 16 integration is not its verified exit, nor completion of 17–24.

### Planned request consumer source handoff (2026-10-15)

The joint increment 16/22 source slice adds the existing Comms policy's typed
planned-request output, a current-own-Console human choice, the same reading
owner, the exclusive prepared World binding and canonical retained delivery.
It depends on increment 22's Communication execution identity, released-request
APIs, actual returned-outcome consumer and complete candidate validation.
No native or milestone completion is claimed by this source handoff.

Focused client checks: 430 tests across comms-requests, comms-radio and
console-state passed; strict String Table checks passed with 6,151 strings and
no errors or warnings. Owner tests added for exact binding, missing feed/contact,
ordinary Cruiser policy reading, handoff and unavailable/undeclared operation
remain unexecuted until root runs the joined native source. Independent craft
source review passed after correcting the optional publication API arguments.

## Adopted next real action: Formation coordination

[ai] Join increment 18's received-formation-intent-stages contract with the
closed `FormUp { slot: [i32; 2] }` request in existing World x/z axes. The
request contains no leader UUID: the genuine sender and literal callsign on
its actually received inbox copy provide coordination provenance. Bound each
integer to the supported finite World coordinate/slot limit. Existing text
transport, fixed audiences, actual radio payment/path, own Console publication,
positive AI reading, Human reply and independently paid outcomes remain the
owners. Existing ReportStatus wire representation and meaning stay intact.

Add a distinct `Accept` decision for FormUp, with no CommsStatusReport. This
means coordination accepted only; it never claims Navigation selected a leader,
Helm engaged, a ship reached its slot or a drive is ready. ReportStatus retains
Report/Decline and its actual local report. FormUp exposes Accept coordination/
Decline and uses the same correlated RespondToMessage path. The ordinary
Comms policy handles the supported typed request at its existing cadence; no
second controller, plot flag, foreign actuator command or fake JobId is added.

Order FormUp intent using a positive source sequence stored on that immutable
transmission and its returned reference. Derive the next value from the actual
source's retained sent FormUp history at successful emission; no parallel
counter or request queue is saved. Canonical structured history cannot be
retired by the plot placeholder/removal path. Source sequences increase in
actual commit order and never depend on recipient count, private delivery or
receipt success. All copies/returns retain the exact source sequence. Full
candidate validation rejects mismatched, duplicate or out-of-order source
lineage before writes. ReportStatus has no Formation sequence.

Durable Contact FormUp delivery reuses the canonical retained sent request and
same continuously paid radio/relay service. It has no World mission binding;
its closed FormUp sequence/source lineage is validated separately from planned
ReportStatus retention. Manual ReportStatus and text remain one-shot, and a
request cannot masquerade as either retention kind. Canonical copies deduplicate
by original origin/recipient; source-visible state never claims hidden receipt.

The first formation suggestion is explicitly durable: it remains selectable
until a newer FormUp from that same source has actually been accepted by this
receiver, or a local participant declines/breaks away through its own controls.
An older delayed or duplicate request cannot overwrite a newer accepted
coordination request. A replacement not yet received has no effect. Comms
supersession changes its coordination records only; it cannot directly replace
Navigation intent or engage/disengage a Human Helm. Already accepted onboard
work retains its own lifecycle and link-loss behaviour. There is no universal
formation obligation, automatic compliance or timer slider on every send.

Navigation consumes only its explicitly permitted genuinely received and
accepted suggestions. Its Human independently chooses an earned leader alias;
its ordinary AI requires uniquely matured observed identity/callsign or actual
held reference evidence. Matching private Broad source IDs to a Comms route is
not correlation evidence. Navigation and Helm retain separate authority and
current-observation requirements, with honest local stages. Supported later
stage reports must use a real typed ordinary Comms producer; Accept is never
a remote readiness report.

The increment 22 CommunicationReportStatus primitive and template remain
ReportStatus-only when this enum grows. A Formation planner operator requires
its own real Navigation consumer and evidence contract. This adopted contract
claims no implementation, native/browser evidence or increment exit.

### Formation coordination source handoff

The Comms half now has the closed FormUp/Accept vocabulary, immutable derived
source sequence, paid durable Contact transmission, ordinary receiving policy/
reading/reply/return, exact finite Human controls and the explicit-feed read-only
Navigation suggestion seam. Independent source review closed malformed accepted
sequence chronology and authoritative Contact-only scope. World ReportStatus
remains closed. This source does not implement Navigation/Helm formation motion
or claim a complete Formation consumer, native acceptance or increment exit.

Focused client checks cover exact finite packets, unknown recipient/payload
refusal, Contact-only controls and coordination-only status. Model/owner tests
cover sequence gaps/duplicates/reordering, immutable copies, actual reverse-radio
contact acquisition, delayed-old acceptance refusal and explicit-feed isolation;
those Rust tests remain root-owned execution work. Complete fresh-host Formation
continuation joins the actual Navigation/Helm consumer next.
