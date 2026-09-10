# GM Station-workload producer inventory

Issue #1438, PRD #1419 M4 (stories 11, 12, 14), presentation contract PRD #1418.
Implementation: `src/gm_workload.rs`. Design contract:
`pasm/spec/design/gm-console-t3.yaml`, component `gm-t3-station-workload`.

The Station-workload advisory tells a Game Master how much is being asked of the
people at each Station. It is a count of **distinct outstanding demands that
genuinely require a human**, and this file is the complete list of where those
demands come from. Nothing counted is derived from text, from notification
volume, or from how often anybody pressed anything.

Every producer declares the same five things:

| field | question it answers |
|---|---|
| **source** | which existing subsystem's own state is read |
| **stable key** | the canonical identity used to deduplicate, and to keep one row steady while the demand holds |
| **owner** | the ship, the System, and therefore the Station it belongs to |
| **human-action-needed** | while what is true does a person actually have to act |
| **terminal** | what makes it stop counting |

Station ownership of a **demand** is always
`command_admission::station_for_system(config, human_seeking_hosts, system)` —
the same resolution admission itself uses, so a human-seeking System's demand is
attributed to the seat that is currently presenting it, not to the seat that
authored it.

Station ownership of a **System**, for the purpose of deciding what a Station
*is*, is a different question with a different answer: see
[Which Systems a Station owns](#which-systems-a-station-owns) below.

---

## 1. Pending Comms

| | |
|---|---|
| source | the live Comms inbox (`CommsInboxRes`) |
| stable key | `comms:<CommsMessage id>` — the id the world minted |
| owner | the message's recipient ship (or the ship whose inbox it is, for fleet-wide traffic), at whichever Station currently holds the `comms` System |
| human-action-needed | `gm_attention::pending(message)` — the message is not orphaned, has no selected response, and actually offers responses — **and** the `comms` System accepts human input |
| terminal | answered, withdrawn (orphaned), cleared from the inbox, or the `comms` System ceasing to accept human input |

**Never counted.** An unread informational message with no responses; an
automatic follow-up placeholder while the other side is still speaking. Both are
the empty-response case the shared `pending` predicate already excludes, so the
attention queue and the workload advisory cannot drift about what "pending"
means.

## 2. Navigation clearance (typed Coordination decision)

| | |
|---|---|
| source | `NavigationWaypoint`, `NavClearanceIssueState`, `HelmWaypointClearance`, the router's own `route_coordination` decision, and the hull's `ShipPhysics` |
| stable key | `nav:<ship uuid>#<waypoint generation>` — the request generation |
| owner | the Station currently holding `helm-steering`, the System the clearance is addressed to |
| human-action-needed | a waypoint is set, the clearance for **this** generation has been issued, the Helm has not latched that generation, the delivery of that clearance actually routes to a person, and the hull has not yet arrived |
| terminal | **arrival** (auto-completes, latched per generation); **replacement** (a new generation, hence a new key and a new demand); **withdrawal** (the waypoint cleared, or the routing ceasing to be a popup) |

### Routing, not the issuer frontier

`NavClearanceIssueState::issued_generation` is latched once per generation
**whatever the helm's control state** — that is the exactly-once issue policy,
and it is deliberately blind to what happens at the far end. Three different
things happen there:

* an **AI Helm** consumes the clearance and latches it — the AI's own work;
* an **AI Navigation to a human Helm** raises a popup — the only case that asks
  a person for anything;
* a **human Navigation to a human Helm** is suppressed outright, because two
  people at the same table coordinate out loud.

So the producer asks the router's own question —
`route_coordination(navigation's live source, the Helm seat's delivery control)`
must be `Popup` — rather than treating the frontier latch as the demand. A
producer keyed on that latch would have told a Helm operator they owed an answer
to somebody sitting next to them. Reading it live is the point: a Navigation seat
a human takes over stops asking.

On the shipped hulls this is not a corner case. `navigation` is a human-seeking
Station whose host order reaches the Helm, so the moment anybody sits down
Navigation comes and sits with them and the clearance is suppressed. The demand
exists when the hull's **own** Navigation plotted the course.

### Arrival is the completion, and the course is not touched

The clearance is modelled as a task-style demand that **auto-completes on
arrival**: the hull inside the waypoint's existing arrival tolerance — the hull's
authored `[behaviour] waypoint_arrival_radius`, falling back to
`ai::WAYPOINT_ARRIVAL_RADIUS`, which is the same radius `ai::server`'s patrol
cursor advances on and `helm_ai`'s Reach completion fires on. No new constant,
and **no new acknowledge action on the Helm**: a human Helm that has flown the
course has answered the clearance by arriving, and asking them to also press
something would be inventing work in order to measure it.

Completion is latched per generation, so a hull that arrives and then flies onward
does not have the demand come back at it.

The **actual navigation waypoint is never auto-cleared**. Arrival completes the
DEMAND, not the course: Navigation keeps sole ownership of setting and clearing
waypoints, and a standing order to hold at a point is still a standing order once
the hull gets there.

The completion is computed in the producer, in the fixed loop, from owner state —
not opened as a `core::task_lifecycle` activation. Activations are authoritative
simulation state that every peer folds identically, opened and closed by the
consoles that own the work from real admitted commands; this advisory is
peer-local presentation on a GM desk and must not write task state, and there is
no console verb behind a nav clearance to open an activation from — a clearance
is a Channel-3 message, not a task. The per-generation latch it needs rides in
`GmWorkloadWatch` beside the overload stopwatch, for the same reason: an arrival
is history a restored world cannot recompute.

**Never counted.** A waypoint whose clearance Navigation has not sent yet — that
is still Navigation's to send, not the Helm's to fly. A clearance already latched
by an AI Helm: that hull is flying it, and it is the AI's work. A clearance the
router suppressed between two people. A course the hull has already flown.

## 3. Repair dispatch request (typed Coordination decision)

| | |
|---|---|
| source | the ship's own `RepairRequestQueue` — the state behind the popup, never the popup |
| stable key | `repair:<ship uuid>/<damaged Station id>` — exactly the identity the queue merges on |
| owner | the Station currently holding the `repair` System, the destination the request is addressed to |
| human-action-needed | the entry is in the queue and `repair` accepts human input |
| terminal | `prune_repair_request_queue` dropping the entry once the damage it named is gone; the seat ceasing to be human |

### The queue is the record, on both delivery arms

Before #1438 only the **AI** arm of `receive_repair_coordination` wrote the
queue. A request delivered to a human Repair seat therefore lived solely as a
popup — a thing that has been *shown*, not a thing that is still *owed* — and
nothing in the game could answer "is this crew still being asked to send a team".
Both arms now write it: the entry is the record and the popup is the notification
of it. The AI's own consumption is unchanged; it dispatches from exactly the entry
it always did.

The write rides `CoordinationDelivery::HumanRouted`, a peer-identical delivery the
router emits alongside the popup, rather than the popup delivery itself. The
Popup *decision* is a function of the hull's live control sources, which every
peer holds, but the popup is raised on the presenting peer alone — `LocalShip`
plus a seated session — so hanging ship state off it would leave two hosts of one
fleet carrying different queues from identical input.

### Behaviour change: AI repair now dispatches on a mixed seat

Writing the queue on the human arm has a consequence outside this advisory, and
it is a deliberate one. `operate_repair_ai` is gated on the **fine** `repair`
System being AI-operated (`ai_operates(&sources.0, repair_system_id())`), not on
the seat. A seat can be both at once, and the shipped fleet authors exactly that:
`assets/entities/alliance_cruiser.toml` gives the `engineering` Station a
`Simplified` rating with `automated_systems = ["repair"]`, so `apply_rating` sets
`repair` → `Ai` and every other Engineering System → `Human`. The router asks
`seat_control_source` about the whole seat, gets `Human` because the power Systems
accept human input, and routes the `RepairRequest` **Popup**.

Before #1438 that combination produced a popup and nothing else: the queue stayed
empty, so the AI repair the rating had just created had nothing to dispatch, and
the ship's damage went unswept until a person acted. The rating's own manual copy
promises the opposite — *"Simplified rating lets the AI run the repair teams so
you concentrate on power allocation"* — so the silence was the bug, not the fix.

Since #1438 the `HumanRouted` arm writes the entry, `operate_repair_ai` sees it,
and teams cross, repair hull HP and change mission outcomes on a stock cruiser
under the Simplified Engineering rating. `RepairRequestQueue` is folded into no
digest, but the hull HP downstream of it is authoritative state, so this is a real
gameplay change on a shipped configuration. The rationale — the rating's authored
promise is the contract and the code now keeps it — is recorded as an `[ai]`
bullet on `gm-t3-station-workload` in `pasm/spec/design/gm-console-t3.yaml`, so it
is reviewable and revisable there. The ratified cruiser balance matchups were
re-run against this change; see the commit body for the measured table.

Because that dispatch is authoritative, #1438 bumps `SIMULATION_RULES` to
`"0.7"` in `src/snapshot.rs`. Two rules moved: the human-routed write itself,
and the stale-entry prune becoming seat-independent and running every fixed step
(below). The queue is in no digest and its snapshot field is serde-defaulted, so
a pre-#1438 save *parses* — it restores intact with an empty queue and then
diverges at the run's next damage-tier crossing, which is the "restores cleanly
and then continues as a different run" failure the rules dimension exists to
refuse. `SNAPSHOT_FORMAT` is unchanged; nothing about the payload's shape did.

One more visible consequence of the same write, on hulls with no AI repair at
all: a **fully human** Repair seat now has a non-empty `queue_depth` on its own
console readout (`src/console/repair/visibility.rs`), where that number was
always zero before. That is the intended semantics — the entry is the record of
what is still owed — and it is the same single change as the dispatch above, not
a second one.

The advisory itself is unaffected by that dispatch. `collect_repairs` asks
`needs_human` about the `repair` System, which is `Ai` here, so a request on this
seat is **not** counted as a human demand even while it is queued and being acted
on. Test:
`a_simplified_engineering_seat_dispatches_ai_repair_without_counting_as_human_work`
in `tests/gm_workload.rs`, which drives the mix through the real rating rather
than by hand-setting control sources.

### Ending a request is about damage, not about who is reading it

The stale-entry prune used to sit **inside** `operate_repair_ai`, after that
system's control-source gate, so an entry only ever ended on a hull whose Repair
seat was AI-operated. That was invisible while only the AI arm wrote the queue; it
stopped being invisible the moment the human arm started writing it too. It now
runs seat-independently every fixed step in `prune_repair_request_queue`, ordered
immediately before `operate_repair_ai` in the same Physics phase, so the AI still
decides against an already-pruned queue exactly as it did — and a hull whose
Backfill seat flipped to Human mid-repair no longer carries its old entries for
the rest of the mission.

**Never counted.** A second, worsening `RepairRequest` for a Station already in
the queue: the queue merges it, so it is one demand however many times it is
filed. This is the duplicate-alert case; it is handled by reading owner state
rather than by counting deliveries. A request the router **suppressed** because a
person reported the damage to a person: never delivered, so never queued and never
counted.

## 4. Task activations

| | |
|---|---|
| source | the live `TaskLifecycles` registry |
| stable key | the activation's own `TaskKey` wire form (`operator/system/verb/target#ordinal`) |
| owner | the activation's operator hull, at the Station currently holding its `slot.system` |
| human-action-needed | the verb's rule in `TASK_DEMAND_INVENTORY`, **and** the owning System accepting human input |
| terminal | the activation's own single terminal event |

Every verb this build can open an activation on is listed below, and **every one
of them is never counted**. That is the finding, not a placeholder: PRD #1419
says in terms that "running tasks are not demands merely because a human started
them", and not one of these activations has owner state that stops it
progressing until a person decides something. What a person genuinely has to act
on when that work goes wrong is a *different* piece of state — a repair request,
a conversation — and those are counted by their own producers, once.

| verb | counted | why |
|---|---|---|
| `scan` | no | runs to its own reading or refusal; the operator is not asked anything while it runs |
| `tractor_hold` | no | a coupling holds by itself and ends on release, range or power; holding is not a pending decision |
| `dock_hold` | no | a formed mate persists on its own; the approach that formed it is already over |
| `umbilical_flow` | no | a running transfer moves capacity automatically and closes on capacity, range or cancellation |
| `external_repair` | no | a dispatched team crosses, repairs and returns unattended; the *request* that asked for it is the demand, counted by producer 3 |
| `transport` | no | a running rescue transport recovers automatically and closes on range, target loss or completion |
| `security_team_<n>` | no | a committed team deploys, works and withdraws on the authored clock; a refused dispatch never opens an activation at all |
| anything else | no | an activation whose meaning this inventory has never been told is unattributed source state, which PRD #1419 excludes rather than guesses at |

A `src/gm_workload.rs` unit test reads `src/core/task_lifecycle.rs` and fails if
a `TASK_VERB_*` constant exists with no entry in the table, so a new continuous
task cannot be added without a recorded decision.

---

## Levels

| level | when |
|---|---|
| **Backfill** | every System the Station **authors** is AI-operated |
| **Offline** | every System it **authors** is damage-disabled or explicitly offline |
| **Underused** | a human seat with zero counted demands |
| **Engaged** | one or two counted demands — or the count is at the threshold but has not held there for the authored duration yet |
| **Overloaded** | the count has been at or above the threshold continuously for the authored duration |

A **mixed** Station — some Systems human, some backfilled — is a human seat, and
counts only the demands whose own System is human-operated.

There is deliberately **no level for a Station whose Systems have migrated
away**: it is omitted from the summary instead. See the next section.

## Which Systems a Station owns

Two different questions are asked of the same hull, and they have two different
answers. Conflating them is what made the stock cruiser's `navigation`, `command`
and `comms` rows read *Offline* the moment anybody sat down.

| question | answer | used for |
|---|---|---|
| *where is this System being operated from, right now?* | `command_admission::station_for_system` **with** the live `HumanSeekingHosts` | attributing a demand to a seat |
| *which Systems does this Station own?* | `ShipConfig::systems_for_station` — the authored `[[system]] station = …` blocks | deciding whether a Station is a seat at all, and which word it reads |

The second is the **authored** membership: the same set the lobby projects into
the roster pill's `station_systems`, and the same set the Channel-3 router's own
generic branch (`ship::coordination_systems::station_delivery_policy`) reduces.
Backfill / Offline / the mixed-Station human subset are then read from those
Systems' **live** control sources, so `Offline` means what its name says — every
authored System damage-disabled or explicitly offline — rather than "this
Station's Systems went somewhere else".

### A Station whose Systems have all migrated is omitted

A Station every one of whose authored Systems is currently hosted at **another**
Station is not a seat anybody holds this tick, so it is **left out of the
published summary entirely**. It is not Offline (its Systems work perfectly
well), not Backfill (no AI holds it) and not Underused (nobody is there to be
under-used), and no fifth word was invented for it: the honest presentation of
"nobody sits here" is no row.

Its demands are already attributed to, and counted at, the host Station by
`station_for_system` above — so the omission loses nothing, and naming the
Station a second time would double the fleet's apparent workload.

The Station that **hosts** migrated Systems keeps counting their human-required
demands. That is unchanged, and it is what makes the Captain's row carry the
Comms traffic while the Comms seat is empty.

On the stock Alliance Cruiser this is the ordinary case rather than a corner. The
hull authors three visiting Stations, each owning exactly one System:

* `navigation` — human-seeking Station,
  `host_order = ["comms", "captain", "helm", "tactical", "science", "engineering"]`;
* `command` — human-seeking Station, `host_order = ["captain"]`;
* `comms` — its `comms` System is human-seeking with no `seek_order`, so with its
  own seat empty it goes to any crewed seat.

Reading membership from the live host map left each of them with the EMPTY set as
soon as a host was found, and an empty set reduces to `Offline` — so `navigation`
read "No System here can be operated" whenever any of those six seats was crewed,
while the roster pill beside it read `Comms · Human`.

`tests/gm_workload.rs::a_station_whose_systems_have_all_migrated_is_left_out_of_the_summary`
drives the whole shape on the real cruiser through real lobby seat claims, and
`tests/client/gm-workload-panel.test.js` pins the panel half: an omitted Station
is not a row, and the status sentence counts only the rows drawn.

Falling below the count ends the overload **and** resets its timer: a Station has
to earn the label again. The elapsed time is counted in fixed simulation steps,
so a paused world banks nothing, and it rides the snapshot because a restored
world cannot be asked how long a seat has been underwater.

## Authored overrides

All three live on the existing `[gm_attention]` table and are validated at world
load:

```toml
[gm_attention]
workload_overload_count = 3     # positive; distinct demands
workload_overload_secs  = 30.0  # positive, finite; SIMULATION seconds
workload_disabled       = false # silences this advisory and nothing else
```

There is no zero sentinel. A zero count would report a Station with nothing to do
as a candidate for Overloaded, and a zero duration would delete the "continuously
for" half of the rule while still looking like a setting somebody chose; both are
refused at load, naming `workload_disabled` as the switch that actually means
off. Silencing the workload advisory leaves the pending-Comms attention queue,
the idle-NPC advisory and the technical banners exactly as they were.
