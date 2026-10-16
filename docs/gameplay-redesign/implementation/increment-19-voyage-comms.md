# Increment 19 — Voyage requests and returned owner events

**Adopted implementation contract, 16 October 2026. No runtime or acceptance claim.**
Root adopted the coupled Comms/craft contract after independent source review.
This pins the Comms half of [moving craft](./increment-19-moving-craft.md).
Its real utility recipe has passed the accepted-loader gate. Genuine launch
reports and the voyage Navigation/Helm consumers remain separately owned dependencies;
no placeholder voyage, report or Job makes this contract executable.

## Source intent and immutable transmission

Keep the source's choice separate from the immutable emitted instruction. Add a
closed `CraftOrderIntent` with `Travel { voyage, waypoint }` and `Recall { voyage }`.
The existing actual Comms System receives a normal correlated
`SendCraftOrder { channel, intent }` command. `waypoint` is an offered own
Navigation ID, not coordinates. The command contains no audience, source actor,
sequence, expiry or payload-position field. Actual source identity comes from
admission; the single Contact recipient is the selected voyage's craft.

An optional accepted actual Radio profile, `craft_orders`, names the exact
Navigation publication and `request_lifetime_ticks` in `1..=36_000`. This is a
technical supported lifetime bound, not message capacity or a per-message stock.
The accepted ship must provide that actual Navigation System and an explicit
typed Navigation feed to Comms. Ordinary supported channel keys/range/work and
the Comms System's ownership remain unchanged. Absence preserves existing radios.

Before paying, the Radio rechecks the currently permitted exact own Bay launch
report. Its full `VoyageRef { craft, home, launch_scope, launch_tick }` must match
the selected report; home and the scoped Bay actor must be the actual issuer.
Only an explicit typed feed to that real Bay exposes the parent's historical
report. Do not query the child's current location, controller, existence or
foreign custody to decide whether the source may attempt a send. No current
Comms contact is invented from the report. Its historical known receiver can
support this one best-effort request family without becoming a generic roster.

The craft owner provides this selected closed shared type in
`phoenix_model::craft_voyage` (not yet implemented):

```rust
VoyageRef {
    craft: String,
    home: String,
    launch_scope: phoenix_model::activity::ActivityJob,
    launch_tick: u64,
}
```

Use the existing bounded actor/System identity rules and positive `JobId`, with
`launch_scope.actor == home`, actual Bay ownership and an original nonfuture
commit tick. This is the identical cause in the existing Bay owner's retained
`BayView.launch_reports: Vec<VoyageRef>` and the child's proposed
`CraftLaunchAssociation(VoyageRef)`. Only the real successful paid physical
launch commit creates them together. The adopted moving-craft report bound is
128 per Bay, newest local launch per craft; it never blocks a launch.

The selected source accessor is
`entities::craft::launch_reports(config, boards, reader, bay) -> Vec<VoyageRef>`.
Like the existing Navigation accessor, it verifies an actual Bay System and
explicit permitted typed Bay feed, then reads only that own publication. It
does not poll children. A separate craft-local association accessor can read the
receiver's own launch cause for recipient validation; it accepts no foreign
actor/pose query. These launch APIs remain dependencies, not existing runtime.

For Travel, recheck the exact configured Navigation feed and its explicitly
published charted current-World point choice. The following API is already on
main, inspected at `a1a77047`; it is the source contract for this consumer:

```rust
phoenix_model::navigation_point::NavigationPointView {
    id: String,
    label: String,
    point: [f32; 3],
}

navigation::known::charted(
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    reader: &SystemId,
    source: &SystemId,
) -> Vec<NavigationPointView>
```

Call this accessor with the actual Comms System as `reader` and the configured
actual Navigation System as `source`. It checks that the source is Navigation
and that the reader has its permitted typed publication. It returns only
`NavigationBlackboard.charted_points`, derived from the existing `Picture`
charted-contact path with actual chart range, shown tags and concealment checks.
Require exactly one current offered point with the selected ID and validate its
existing finite value before preparing an instruction. Neither co-hosting nor
an unpermitted canonical Navigation lookup can supply a point.

The neutral DTO preserves the existing `transit::TransitRouteView` alias, fields,
validation and serialized semantics. `NavigationBlackboard.charted_course` is
the independently selected local fixed intent, not an additional offer, a
voyage Job or arrival evidence. Withdrawal can legitimately leave that local
course in place, but it cannot make a missing current offer eligible for a new
Travel emission. There is no new authored point list or registry.

A foreign World's Transit arrival point is not automatically a local XYZ
destination. This consumer does not union Transit routes, civilian routes or
private actor positions into charted choices. Reusing the DTO retains the actual
coordinate frame and original current-World chart eligibility.

The new immutable instruction itself retains a bounded point ID and whole-unit
World XYZ within plus/minus 1,000,000. Conversion is exact and only applies to
this new send: fractional/off-grid values refuse without changing existing
routes' supported precision. The source is an actually permitted known point,
not a hidden actor position or a path to foreign content. A caller cannot inject
or replace coordinates, and the accepted underlying source must agree before
the instruction can be emitted.

The Radio then prepares one closed immutable wire request:

- `CraftTravel { voyage, sequence, waypoint, position: [i32; 3], expires_tick }`;
- `CraftRecall { voyage, sequence, expires_tick }`.

`expires_tick` is checked addition of the actual emission tick and the accepted
equipment lifetime. Overflow refuses before payment or identity mint. The
positive sequence lives inside these two immutable request variants. Existing
request references already retain the whole request, so no additional common
transmission/reference sequence field is added. The existing
`formation_sequence` stays unchanged and absent on voyage requests. It is the next
sequence in retained genuine sent history for this exact home/craft/launch, not
a caller value, delivery-dependent number or new counter. Source records remain
the sole issuance history; no retry queue, expedition registry or second inbox
is introduced. Existing FormUp sequence meaning remains unchanged.

Existing `SendCommsRequest` must refuse caller-supplied CraftTravel/CraftRecall
wire variants. Only this revalidated semantic adapter can create them. The
normal paid source-emission commit binds the final request, sequence, origin ID
and expiry together. Merely preparing, selecting a destination or failing work
consumes no sequence or successful emission. The attempt does not depend on
private delivery viability. A successful local feedback means emission only.

Both requests are Contact-only sustained delivery through the existing retained
sent record and actual paid route. Copies retain the original content, sequence
and emission/expiry clocks. Current listening coverage, keys, physical path,
endpoint work and existing receiver protection still govern actual delivery.
Retiring a parent's historical report cannot cancel a previously emitted request.
An expired retained request may still arrive as dated historical evidence; expiry
ends eligibility for new reading/positive acceptance, not the existence of an
already emitted record. Preserve existing structured-request dismissal/clear
retention for these kinds, including their sole sent-sequence history.

## Receiving scope and expiry

Use the existing ordinary per-System Comms reading and Human/AI Accept/Decline
owner. A received offer is eligible for new reading/positive acceptance only
while unexpired and matched to the craft's genuine own launch association. Do
not look up a foreign pose or select an observation by private source UUID.
Unsupported, stale, wrong-voyage or later-owner requests cannot create a new
positive decision or a Navigation command. Dated evidence can remain visible;
visibility is not permission or proof that work began.

Keep the pending accepted offer on its same actual recipient inbox copy.
Existing structured-request clear/remove only dismisses presentation and retains
that copy. Do not add a pre-Navigation queue, copied acknowledgement owner or
second receiving store. Navigation's normal controller reads the exact permitted
accepted original, then issues its own ordinary admitted command. A Human Comms
Accept never operates a Human-controlled Navigation System.

Comms Accept reports coordination only. The actual craft's Navigation separately
revalidates its own authority, exact current launch, unexpired request and
strictly newer accepted sequence. Only that admission creates or redirects the
actual Navigation Job/current leg. The Comms decision is not the accepted-leg
frontier. An older delayed offer cannot clear the course, reset the Job or win
by being viewed later. Accepted onboard work persists past link loss and expiry
under its ordinary own constraints. Recall without a genuine admitted voyage
Job cannot create an invented return history.

The selected onboard owner is one optional `Job<CraftVoyageActivity>` inside
existing Navigation. Its activity binds `VoyageRef`, the latest locally admitted
exact `CommsRequestReference`, existing waypoint `course_generation`, closed
stage, positive `transition` and original `transition_tick`. First admitted
Travel mints the real Job; a greater same-voyage sequence redirects that same
Job/current leg. It retains no competing point, transform, pilot or work
account: the existing NavigationWaypoint owns the fixed course/generation.
AtWaypoint is a nonterminal wait for the next admitted leg; it cannot settle
the Job and prevent Recall from redirecting it. Failed or actual recovered
settlement remains the Job owner's terminal boundary. A different later trip
after settlement must use its actual new Job rather than reopening history.

## Owner events and return delivery

Extend closed `CommsRequestFacts` with optional `voyage` owner-event evidence
rather than treating `CommsRequestStage::Completed` as every later stage. The
selected Model type is:

```rust
CraftVoyageEvent {
    voyage: VoyageRef,
    request: CommsRequestReference,
    nav_scope: ActivityJob,
    course_generation: u64,
    transition: u64,
    event_tick: u64,
    stage: CraftVoyageStage,
}
```

`nav_scope.actor` is the actual craft and `nav_scope.owner` its actual Navigation
System. The original request must be its own received CraftTravel/CraftRecall,
matching that voyage, source home and recipient craft. `transition` is positive
and increments only when this actual Job changes stage or admits a new leg;
unchanged state produces no new event. It is the Job's existing transition
frontier, not a second event clock/store. Overflow refuses the transition before
partial writes. Its original event tick follows actual local admission/read/
decision and never exceeds the containing emission or restore frontier.

First closed stages are NavigationAccepted, Travelling, AtWaypoint, Returning,
`Paused { reason: CraftVoyagePauseReason }` and
`Failed { reason: CraftVoyageFailureReason }`. The owner-agreed closed reasons
are:

- `CraftVoyagePauseReason::{NavigationUnavailable, HelmUnavailable,
  HomeObservationUnavailable}`. The last covers absent, stale or ambiguous
  permitted home evidence without inventing private truth.
- `CraftVoyageFailureReason::{CourseReplaced, Cancelled, LocalJobFailed}`.
  CourseReplaced requires an actual different own intent generation outside
  an accepted newer voyage leg; offer withdrawal never proves it.

These report actual local admission, machinery, Job or permitted observation
state, with no free text, foreign resource/pose lookup or inferred remote loss.
NavigationAccepted proves its own admitted leg; Travelling requires actual
same-generation paid Helm execution. AtWaypoint requires its actual arrival and
speed tolerances, while Returning means a Recall leg was actually admitted.
The original request kind still identifies a return leg if its later stage is
Travelling. `course_generation` always binds the actual admitted intent frontier,
not a fresh generation minted by projection or reporting.
Only that owner can establish these conditions; Comms cannot infer them from a
desired course, text, silence, ReportStatus or its own Accept.

The selected read-only accessor
`navigation::voyage::events(config, boards, reader, source)` returns zero or one
latest `CraftVoyageEvent`, from the selected
`NavigationBlackboard.voyage_event: Option<CraftVoyageEvent>`. It checks the actual
Navigation source and the reader's explicit typed feed. Comms may copy only that
exact event into the matching actual received request facts. No private Job scan,
foreign publisher or hardcoded Station identity can supply one. This API and
field are implementation dependencies; current main has no voyage Job/event owner.

First reporting honestly coalesces to the latest current Job event. It does not
promise that every intermediate stage will be transmitted across a gap. Already
emitted events remain immutable retained Outcomes and keep their existing paid
delivery behavior. There is no unbounded transition archive or retry queue.

Returned emission/delivery deduplication must include the exact
`(voyage, nav_scope, leg sequence, transition)` owner-event identity in addition
to the existing coarse request stage. Current `request_returns::settle` looks
only at `facts.stage()` when finding an already-sent Outcome; that comparison
must include this event identity for voyage facts. It otherwise suppresses a
genuine later Paused/Returning event after an earlier Completed stage. Existing
`CommsRequestProgress.last_returned` remains a coarse delivery frontier, not a
second authority for the latest Nav transition. Validate its supported voyage
evidence against actual retained emitted/heard copies instead of pretending that
coarse stage equality authenticates a new event.

Each new local returned event pays the existing discrete channel work before
minting its ordinary Outcome. Failure cannot stamp a successful return, and
remote viability cannot decide whether this local emission occurred. An already
sent Outcome supports later delivery across a gap without another discrete send
or new event. The destination learns only actual heard copies; delayed older
events remain dated evidence and cannot replace a greater received transition
for that same scoped Job, issue a command or change the trip. Do not introduce a
parallel event clock, mission log, synthetic `ActivityJob` or ReportStatus bridge.

Recovered remains the home carrier's genuine local completed Bay receipt. A
stored/Disabled craft cannot send a fictional final message. Other-party
physical recovery reporting remains a later explicit producer. Captured
CraftTravel/CraftRecall projections initially expose only their closed kind;
they must redact voyage/craft/home/Bay/Nav/waypoint/position/receipt references
and cannot provide a response, order receipt or machinery action.

## Continuation and evidence

Integrate each new canonical field with the existing original-copy validation,
snapshot whole-candidate/no-write guards and digest immediately. Validate exact
source/Contact recipient/home, launch scope, lifetime, immutable content and
per-voyage contiguous source sequence. Available genuine same-launch reports,
child association and actual accepted Job evidence must agree. Legitimate
replacement/removal/retirement does not invalidate retained historical facts,
and validation must not claim authentication of an erased past or protect a
physical actor from removal. Original references stay sufficient for accepted
work after the presentation inbox or report retires.

Before a playable exit, prove normal Human and ordinary AI choices, real finite
paid source emission, actual recipient reading and independent own Navigation
admission; same-voyage Travel then Recall; paid physical XYZ motion; and genuine
returned owner stages. Exercise unavailable work, unknown waypoint/feed,
off-grid data, expired/newer/duplicate/old-voyage messages, foreign issuer,
actual link loss, report retirement and real parent removal. Source refusal must
leave payment, message mint and sequence unchanged. Fresh whole JSON360 at
partial reading and moving/link-gap/return frontiers must compare complete
captures/digests, with malformed candidates refused before any write. Actual
Workshop, Console controls and native/device evidence remain required.
