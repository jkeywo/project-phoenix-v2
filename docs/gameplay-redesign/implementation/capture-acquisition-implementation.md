# Bounded uncrewed vessel acquisition — implementation handoff

Status: implemented source; native checks and playable acceptance pending. This
slice does not close increment20, Patrol/War/Sandbox or generation/Director work.

## Runtime join

The original `SecurityJobs` and `JobId` own dispatch, paid work, return and actual
team release. An explicit `SecurityAcquisitionOrder` freezes the target, authored
hold profile and original accepted tick. The existing team tick pays and advances
that job, but withholds its success edge. The immediately chained exclusive
adapter in the same `TickSecurityTeams/Modifiers` step prepares current eligibility,
Session ownership and the existing NPC doctrine owner before committing.

An unsuccessful prepare starts ordinary Failed withdrawal without an owner,
doctrine change or success flag. Successful work records its exact cause on the
same job and target, projects the frozen acquiring faction onto the same body,
and reuses ordinary faction-lock revalidation. Work completion and physical home
settlement remain distinct. Existing hull damage, resources, equipment, emitted
effects, launch slots, Stations, Console ownership and System controllers remain
with their existing owners.

The first optional target is `acquirable_harrow.toml`, using existing hardware and
an explicit Board acquisition effect. Its World must bind its name to a base
`acquired-hold` palette with nonempty existing None objectives. Content validation
rejects missing/incompatible/layered or dynamically altered hold declarations.
Security needs its own permitted selectable contact with an actually observed
disabled assessment; exact physical disablement is rechecked at useful completion.

## Continuation and authoring

`VesselOperationalOwner` belongs to Session and contains a frozen registered
faction and exact Security actor/System/Job/team/target/accepted/completed clocks.
The snapshot and digest include it and the same Job evidence. Whole-candidate
preflight validates source, identity, clocks, mint and retained bidirectional
receipts before writes. Later control, crew, GM or damage changes do not
retroactively invalidate the acquisition. A missing original acquirer preserves
the same prize and frozen faction. These structural guards do not authenticate
an otherwise self-consistent fabricated past.

Workshop's existing Security panel edits an explicit target acquisition toggle
and hold-profile ID through accepted source/history, including target-only
entities, undo/redo, save/reload and existing Preview/Test entry points. Ordinary
Security UI distinguishes an acquisition effect and acquired result from the
team's continuing withdrawal. It receives no foreign machinery controls.

## Evidence recorded by the worker

- Independent Craft source review: core transaction/history and later ordinary
  faction revalidation/Content-closure joins passed; no native execution claimed.
- Independent Comms UI/owner-proof review identified and corrected the sparse
  fixture machinery schema, saved-publication Vec lookup and disposed-button
  liveness guard; no native execution claimed.
- Targeted client suite:33 tests in3 files passed (Workshop, dispatch, component).
- Strict strings:6492 strings, zero errors/warnings. Layer rules passed.
- Changed Rust files formatted; diff whitespace check passed. YAML/TOML source
  parsing and affected wiki-source/index checks are recorded at handoff.
- Native source probes cover actual spawned owners and finite paid Security
  work, a partial work full-JSON1200 continuation with full captures/digests,
  target resource/damage conservation, guessed/lost aliases, same/separate-actor
  contention, stale Human authority, malformed whole-candidate no-write and
  original-acquirer removal. The incident and assessed return are explicit owner
  fixture preconditions, not evidence of physical encounter or sensor acquisition.

## Integrator checklist

- Replay docs203c1823→f6e71c58 plus the coherent runtime/proof donor(s), preserving
  current main's media, personnel, finite-resource and movement work.
- Review central same-step schedule chain, EntityState optional field, both
  restore doors, capture, digest and Folded/DeferredFold census declarations.
  The effective Faction projection is derived from the Session record/original
  accepted target recipe rather than another stored mutable pool.
- Advance protocol/save/rules compatibility as required on the integrated
  result; the worker intentionally changed none of those central epochs.
- Run sequential owner tests in Model, Gameplay, Session, Content Preparation and
  Simulation (`security::acquisition`, existing `security::server`,
  `snapshot::security`, `gm::operations::npc`). Verify actual current-source
  linked library when sharing Cargo target output.
- Run required integrated gates once before push and record device/crew
  acceptance separately. Source tests are not native or Human acceptance.

Still open: populated-crew capture/prize staffing/cooperation, negotiated surrender,
remote operational orders, combined local/external Security equipment, contested
recapture, full physical boarding transport, capture/rescue mission primitives,
Patrol/War/Sandbox progression, role-load evidence and Director pacing.
