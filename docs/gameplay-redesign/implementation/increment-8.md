# Increment 8: power, local batteries, coolant and heat

Adopt `pasm/spec/design/local-energy-and-thermal.yaml` before runtime changes.
The user authorised increments 7–18 on 14 October 2026 and subsequently
authorised validation agents on pinned worktrees after each initial increment
commit while the main implementation proceeds. Merge fixes in dependency order;
an initial commit is not a verified playable exit. Keep Cargo builds sequential.

Replace the existing allocation-to-output multipliers and ship-wide exhaustion
lock with incoming energy flow and consumer-local reserves. Engineering controls
intake and cooling; owning Systems control their work. A zero incoming flow does
not stop a System with stored energy. Continuous work receives an affordable
fraction; discrete work waits for its entire cost before consuming ammunition or
producing an effect. Tactical restraint is an explicit admitted weapon intent,
including the existing scripted hold-fire/return-fire journeys.

Use one reusable continuous-capacity and thermal mechanism for each consumer
and the reactor capacitor. Fullness-dependent passive loss makes a full reserve
costly. Heat records gross intake and gross depletion, even when net charge is
unchanged. Intake respects headroom; rejected delivery remains at the reactor and
is not redistributed into another allocation. Reactor generation ramps toward
commanded output, has sustainable/burst limits, and uses its own capacitor,
thermal margin and coolant service. The battery screen presents this reserve;
it must not retain a second authoritative ship battery.

Cooling spends actual finite coolant stock only for heat removed, using the
increment 6 service/stock and replenishment contracts. Persist fractional stock
consumption. Thermal trip/recovery uses hysteresis, automatically resuming the
saved intent when safe. Energy exhaustion introduces no global restart lock.

Coarse, detailed and minimal controls change the abstraction, not the physics.
Minimal management uses ordered group priorities and no/low/high coolant use;
mid-level management sets coarse flows and cooling; detailed management exposes
nested fine groups. Full delegation uses the ordinary individual System AI.
Manual shortage cuts randomly selected whole incoming allocations to zero using
the existing deterministic RNG. Save requested values and restore feasible flows;
do not turn off the consuming System or rotate service unnecessarily.

Migrate the owning Power controllers to supported reusable state trees. Extend
the closed tree vocabulary only for supported numeric observations, bounded
outputs and host allocation actions. Preserve Console-scoped knowledge, ordinary
Backfill, deterministic top-down/last-write semantics, inheritance and reference
nodes. Workshop cannot add code, expressions or arbitrary game access.

All shipped equipment must remain operable. Author and validate finite energy,
intake, work-cost, thermal, reactor and coolant parameters. Include human controls,
local reserve/heat/status feedback, ordinary AI, Workshop composition/editing,
history, validation, export/reload, Preview and disposable Test. New authoritative
state participates immediately in snapshots, preflight, restore, deterministic
digests and existing multiplayer projections; advance affected compatibility
versions and explicitly reject incompatible state/content.

Shields remain usable through the transition; shield upkeep/redistribution belongs
to increment 9. Physical placement/routes, component damage and automatic local
deliveries remain increments 10–12. Coolant recycling remains provisional. New
gameplay requires combat and non-combat activity and balance evidence, rather
than claiming the structural equivalence used for increment 7.

### Validation reconciliation: reactor generation and stored export

The reactor's admitted generation-enable intent is independent of its export-rate request. Disabling generation stops new energy production while the same finite capacitor can still supply export; passive losses, cooling, thermal limits and ordinary upkeep continue. The Console exposes this control and the ordinary Power tree can write the boolean `reactor-generation` channel. Its numeric `strike_boost_enabled` fact comes only from the existing permitted Power observation.

Dynasty authors this supported tree behavior to stop generation during an enabled strike and resume it after depletion, retaining its existing charge/strike/recovery activity requirements and actual weapon costs. This corrects the provisional energy migration's empty historical charge group without adding a special controller or another battery. The canonical generation intent is saved, restored, folded and published with the rest of the reactor.
