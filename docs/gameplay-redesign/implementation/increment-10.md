# Increment 10: decks, compartments and internal routes

Adopted implementation contract after increment 9 initial commit `90890f8e`.
Pinned validation continues independently. This document does not claim a
verified exit for either increment.

## Outcome

Give fitted Systems a physical compartment without changing their controlling
Console. Give existing repair teams real, persistent positions and routes through
shared access boundaries. Keep current damage and repair outcomes until increment
11 delivers compartment damage, component conditions and restoration together.

## Existing owners

Gameplay owns EntityConfig, System composition, pure repair phases and canonical
repair jobs. Simulation installs capabilities, applies admitted controls,
publishes permitted views and captures/restores deterministic state. Content
Preparation and the existing JS Workshop own accepted-source editing, includes,
history and validation. Extend those owners; add no network or job implementation.

## Layout definition

- An entity's immutable interior defines named decks, rectangular 2D compartments,
  compartment material references, passages and inter-deck connections. A deck
  carries a positive damage-selection weight for increment 11; shot height never
  chooses a deck. Coordinates are ship-local planar coordinates with one explicit
  scale. No component receives a precise position inside a compartment.
- Each installed System has exactly one placement in a compartment, separate
  from its Console binding. Components inherit the System's physical home.
  Generated weapons and shield arcs retain their existing System IDs. Generated
  equipment must be included in validation and authoring placement choices.
- Physical service/conduit routes refer to compartments and connections without
  moving endpoint equipment. Damage to conduits remains increment 11 work.
- Compartments may contain Systems owned by different Consoles. Placement grants
  no command authority or knowledge, and co-location never pools Console feeds.
- Author actual layouts for all affected shipped hulls/equipped entities. An
  incompatible external definition receives an explicit conversion/version error.
  Keep fixture conveniences explicit and outside accepted production parsing.

Rectangles are the initial supported footprint for this increment, not a new
requirement on the long-term design. Ship movement, rendering and hit resolution
remain unchanged.

## Access and routing

Represent walkable connectivity independently from damage paths. Supported access
boundaries have one authoritative state shared by route planning and layout
presentation. Operable boundaries use normal System command channels and per-System
AI; authored fixed connections do not manufacture an operator. Add a supported
component only with an actual boundary consumer and closed Workshop vocabulary.

Route deterministically over authored travel costs and stable IDs, including deck
connections. Store each repair team's location, traversed connection/progress and
destination in the existing canonical job/team owner. Repair readiness requires
arrival. Recall and redirect use the team's actual position rather than returning
an invented fixed travel timer. A blocked route waits or replans through known
connections; it does not teleport or silently finish work. Existing workflows,
external commitments, priorities and individual repair selector authority remain.

## Human/AI and knowledge

Extend the existing Repair Console with a deck selector and map showing permitted
compartments, equipment, team locations and route/wait status. Selecting equipment
continues to issue ordinary repair jobs; the same planner executes AI assignments.
Show simple route blockers and reasons. Preserve text/list access, touch controls,
delegation, reconnect and all supported seat counts. The map is an operational
layout, not a new unrestricted sensor or Captain map.

## Workshop and continuity

Extend the existing document/history path with layout creation, compartment
geometry/material assignment, System placement, connections and team home points.
Provide visual placement/dragging plus precise property edits, inherited-source
materialisation, reference validation, undo/redo, export/reload, Preview and
disposable Test. Do not create a separate fitting or entity model.

Snapshot/digest/replication retain actual boundary state and team navigation
continuation immediately. Derive layout geometry and route caches from immutable
accepted definitions. Reject incompatible or malformed placement/route state before
restore writes. Existing Station displays and Console ownership are unchanged.

## Completion evidence

1. All affected shipped content loads with complete placements and valid layouts.
2. A human and ordinary AI dispatch, redirect and recall existing repair jobs
   over same-deck and inter-deck routes; blocking/clearing access has real effects.
3. Repair outcomes remain current outcomes; no new physical damage is introduced.
4. Save/resume and deterministic replay retain travel position, blockers and work;
   human handoff and explicit information feeds preserve authority and knowledge.
5. Workshop edits, validates, previews and Tests actual layouts through its existing
   history/export path. Native/browser/manual acceptance is recorded separately.
6. Compare reference Cruiser combat and non-combat workload/performance, while
   retaining Courier/Destroyer/Battleship journeys and their supported seat counts.

Compartment HP, penetration, equipment conditions, structural restoration and
parts consumption are increment 11. Automatic internal stock deliveries and
fabrication are increment 12. Personnel beyond existing repair teams is increment
20; facilities' broader systemic conversion is increment 21.

## Concrete integration decisions

The following are implementation judgments within the approved milestone.

- The canonical authored `[interior]` block has `team_homes`, named `deck`,
  `compartment` and `connection` arrays, plus a `placements` System-to-room map.
  Coordinates use ship-local x/z metres. Compartments use axis-aligned rectangles.
- A connection names endpoints and positive `travel_secs`; an optional boundary
  System owns its actual state. Route ties use the full stable connection-ID path.
- A seal request prevents new entry. Teams already traversing clear the connection
  before sealing completes. A deliberate seal remains protected until explicitly
  released by its authorised operator; routing AI cannot silently open it.
- Idle teams keep their actual location. External claims are represented explicitly
  and return to their authored entry/home point; a projected Idle slot never makes
  an externally committed team available. Redirects finish a committed crossing,
  then replan from its endpoint, preserving elapsed travel.
- Existing repair work and workflow outcomes remain authoritative. Cross-room
  sweeps must travel before work or on-site detail becomes available. Same-room
  reassignment incurs no invented travel delay.
- Fixed connections require no artificial System. Operable boundaries consume the
  same admitted commands and supported component vocabulary as other equipment.
- Material references are authored now; HP, penetration and material resistance
  are introduced with restoration in increment 11. Local stock routing follows
  in increment 12. Component/System/compartment template conveniences use the
  existing Workshop document model and may be refined after basic composition.

## Bounded migration adapter

Existing hull damage includes repairable buckets that are not installed Systems,
including Core and Science and the monolithic NPC Captain bucket. Their explicit
`interior.repair_placements` map gives those existing visits physical homes.
Validation requires exactly the legacy hull rows outside the installed System
set. This creates neither additional Systems nor another damage owner. Remove
this adapter in increment 11 when compartment damage and restoration replace the
legacy buckets together.

Access uses a distinct presentation family, preventing a gate's authored order
from changing the representative System used for Repair commands. Its ordinary
AI reads a permitted typed Repair publication, required explicitly at load. A
Console may receive that publication without gaining team or foreign gate control.
Unavailable actuators retain intent and transition progress until recovery.
