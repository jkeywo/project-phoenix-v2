# Increment 21: physical facility supply foundation

Source checkpoint only. Native execution, browser/Workshop operation, human
workload and whole-increment acceptance are pending integration evidence.

## Actual consumer

`assets/entities/facility_supply_cruiser.toml` reuses the existing six-seat Cruiser,
finite Infrastructure capacity, Logistics store, Umbilical, Dock, local energy and
ordinary individual AI. It starts with zero alloy and an authored six-unit Main
goal. The quiet source/receiver worlds place two existing equipment-backed Axiom
facilities. Their actual alloy endpoints and one explicit free-allied offer have
six-unit bounds. The full-receiver variant starts with those six receiving units
already present. No economy, shipment timer, mirrored cargo or personnel owner is
introduced. Existing shipped hull definitions and scenarios retain their values.

The crew prioritises a normal Transfer objective, delegates actual operation,
then changes direction and prioritises the other leg through ordinary controls.
Navigation uses its permitted target picture and existing waypoint/clearance;
Helm, Dock and Umbilical retain independent authority and physical limits. A
Transfer directive prevents opportunistic Main replenishment replacing unloading.
Idle delegated resupply and already running unclaimed human work are preserved.

## Source-written checks

- Gameplay `explicit_transfer_precedes_idle_replenishment_and_preserves_active_human_work`:
  explicit-work priority, idle resupply and active human-flow hysteresis.
- Simulation `ordinary_umbilical_ai_applies_explicit_transfer_before_opportunistic_collection`
  and `ordinary_idle_umbilical_ai_retains_default_deliver_resupply_but_preserves_running_human_flow`:
  actual individual AI admission and the existing command applier.
- Navigation `transfer_approach_uses_the_same_paid_permitted_contact_as_docking`:
  unknown names do not reveal a ship; actual paid identified observations permit
  the ordinary command, and private unobserved movement does not replace its
  dated measured position.
- `facility_supply_continuation`: actual paid source loading, loaded undock/travel,
  destination unloading, actual AI start, provider work and paid moving Helm with
  loaded goods after Dock clearance finishes, continuous real goods
  conservation, fresh-host JSON continuation with at least 360 complete captures
  and digests, and malformed resource/store refusal without writes. The separate
  full-receiver journey retains carried goods and source depletion without paying
  its rejected destination service.

These Rust cases have not executed in the worker. The existing JavaScript include
resolver and TOML parser resolved all four effective templates and both worlds,
checking exact amounts, bounded terms, two distinct providers and the actual
six-unit Main goal. Owned Rust formatting and diff checks passed. Scenario design
digest completed before authoring the quiet probe; no shipped pacing was changed.
Root owns all Cargo execution, integration gates and publication.

## Remaining boundaries

Equipment interruption/partial operation and rejected work retain the existing
Port owner tests; this checkpoint adds no claim that their full host matrix or
restoration has passed. Two Axiom instances establish an actual physical leg,
not a second independent facility equipment design. Broader autonomous outside
actor goal selection, workforce/Medical/security custody, systemic dependency
networks, different-LocalShip peers, visual Workshop operation and 2/4/6/8-human
workload remain separate incomplete exits.
