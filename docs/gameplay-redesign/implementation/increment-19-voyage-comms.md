# Increment 19 — Voyage requests and returned owner events

**Contract candidate, 16 October 2026. No runtime or acceptance claim.**
This pins the Comms half of [moving craft](./increment-19-moving-craft.md).
Its real utility recipe has passed the accepted-loader gate. Genuine launch
reports and the Navigation/Helm consumers remain separately owned dependencies;
no placeholder voyage, report or Job makes this candidate executable.

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

For Travel, recheck the exact configured Navigation feed and its explicitly
published charted current-World point choice. Root has pinned the existing
`Picture` charted-contact path with its actual chart range, shown tags and
concealment checks. Reuse/extract the neutral existing `TransitRouteView`
id/label/point shape while preserving that API and serialized semantics; put
charted choices in a separate field with an explicit source kind. There is no
new authored point list or registry. The exact shared DTO/accessor is owned by
Navigation and must be pinned before payload implementation.

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

Comms Accept reports coordination only. The actual craft's Navigation separately
revalidates its own authority, exact current launch, unexpired request and
strictly newer accepted sequence. Only that admission creates or redirects the
actual Navigation Job/current leg. The Comms decision is not the accepted-leg
frontier. An older delayed offer cannot clear the course, reset the Job or win
by being viewed later. Accepted onboard work persists past link loss and expiry
under its ordinary own constraints. Recall without a genuine admitted voyage
Job cannot create an invented return history.

## Owner events and return delivery

Extend closed request facts with actual Navigation owner-event evidence rather
than treating Comms `Completed` as every later stage. A supported event binds
the full voyage, original request reference, actual Navigation `ActivityJob`,
current course generation, closed stage/reason and original event tick. The
first stages are NavigationAccepted, AtWaypoint, Returning and Paused. Only the
actual Navigation owner can establish their conditions and transition identity;
Comms never infers arrival or completion from a line of text, silence or a
ReportStatus response. The exact typed event/accessor is pinned with that owner
before runtime; no placeholder event or fabricated scope is supported.

Returned emission/delivery deduplication must include that actual owner-event
identity in addition to the existing coarse request stage. A later genuine
Paused/Returning transition can therefore return after an earlier completion
receipt. A retained sent outcome supports delivery across a gap without paying
another discrete send or fabricating a new event; the destination receives only
actual heard copies. Do not introduce a parallel event clock or mission log.

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
