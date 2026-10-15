---
title: Resource Stores and Service Links
type: concept
tags: [resources, inventory, torpedoes, infrastructure, umbilical, workshop, continuation]
sources: [crates/phoenix-simulation/src/resources/ports_equipment.rs, crates/phoenix-sim-gameplay/src/resources/stock.rs, crates/phoenix-sim-gameplay/src/resources/service.rs, crates/phoenix-sim-gameplay/src/weapons/torpedo.rs, crates/phoenix-sim-gameplay/src/infrastructure/condition.rs, crates/phoenix-simulation/src/resources/server.rs, crates/phoenix-simulation/src/umbilical/server.rs, crates/phoenix-simulation/src/infrastructure/server.rs, crates/phoenix-simulation/src/snapshot/preflight.rs, crates/phoenix-simulation/src/sim_digest.rs, editor/workshop-resources.js, gui/workshop-resource-panel.js, crates/phoenix-model/src/logistics.rs, crates/phoenix-sim-gameplay/src/resources/logistics.rs, crates/phoenix-sim-gameplay/src/resources/logistics_state.rs, crates/phoenix-simulation/src/resources/logistics_server.rs, editor/workshop-logistics.js, gui/components/ph-logistics-controls.js, crates/phoenix-model/src/production.rs, crates/phoenix-model/src/recovery.rs, crates/phoenix-sim-gameplay/src/resources/production.rs, crates/phoenix-sim-gameplay/src/resources/recovery.rs, crates/phoenix-simulation/src/resources/production_server.rs, crates/phoenix-simulation/src/resources/recovery_server.rs, editor/workshop-production.js, editor/workshop-resource-recovery.js, crates/phoenix-model/src/ports.rs, crates/phoenix-sim-gameplay/src/resources/ports.rs, crates/phoenix-simulation/src/resources/ports_server.rs, crates/phoenix-simulation/src/snapshot/ports.rs, editor/workshop-ports.js]
updated: 2026-10-15
---

# Resource Stores and Service Links

Gameplay `resources` supplies bounded integer stocks, atomic next-stage
requirements and named service/provider bindings. Stocks stay in existing
owners: TorpedoSystem's magazine and InfrastructureState's named capacities.
No separate ship inventory mirrors their quantities.

A stock's level includes reserved physical units. Available stock excludes its
claims. Loading tubes reserve a named round, release it when cancelled and
consume it when loading completes. Ready rounds remain in tubes and occupy
storage space during replenishment. The existing Tactical readout reports
available magazine stock, retaining its decrease at load start.

Named resource services resolve concrete capacity or magazine endpoints.
Umbilicals may override a service or provider explicitly; the dock relationship
still supplies the remote entity. Neither binding selects another nearby
provider or grants information/control. Existing direct capacity references are
supported endpoints. Explicitly authored bindings are validated before loading
or Workshop Test.

Simulation's resource transaction adapter settles flows in stable operator and
endpoint order after ordinary Infrastructure effects. It stages both actual
endpoint owners, validates the whole affected storage profile, then commits both
and mirrors counters and threshold events. This includes mixed magazine/capacity
bays, incoming commitments, tube-held rounds and protected main-store floors.
Keyed receipts report actual movement, including zero; their queue is transient
and must drain before a fold. No deferred capacity debit competes with an
immediately changed magazine.

Snapshot 51/rules 0.23 preserve stocks, claims, timed cargo and fractional flow carry.
Preflight validates tube identity, capacity, timers and claim relationships
before writes. Resource digests include magazine/tube commitments and facility
stocks. Other weapon effect state retains its existing deferred fold boundary.
Services are rederived authored configuration. Protocol 13 carries the current
typed stock, logistics, production and recovery controls/readouts; transport and
delivery classes remain unchanged.

Umbilical resource selection is an admitted setting limited to authored choices;
its selected service and fractional progress survive continuation. Coolant uses
real capacity stocks through these same service/transfer mechanisms. The energy
owner retains only already-withdrawn fractional credit and queues actual debits.
Pending paid torpedo bursts still occupy magazine space until emitted.

Workshop's Composition resource panel edits stocks, magazine quantity, services
and consumer links through ordinary validation, source history, export/reload,
Preview and disposable Test. Inherited data requires explicit materialisation;
endpoint variants and complete link bindings replace atomically. Resource
definitions in Live Inspector require recreation and expose no new actions.

Located logistics binds managed main/local stores to existing capacity or magazine
owners and physical compartments. `Stock.incoming` reserves destination room; it
is separate from held source reservations. Departed deliveries own actual cargo
and one exact destination claim until arrival or a routed return. Shared bay space
counts held and incoming occupancy. Team travel and cargo reuse the same route
cursor, without consuming repair teams.

The fitted Logistics System has its own Console, local-energy reserve, ordinary
state tree and admitted controls. Local goals remain commands to the actual
consuming System. Inventory reads local ammunition, repair and coolant demand through explicit
typed feed; receiving that feed does not confer control. Saved goals, delivery
stages, committed crossings and encountered blockage reports are folded and
validated against the actual endpoint owners before restore writes. Workshop
edits bindings, finite bays and handling through ordinary source history and
validation. Main floors and waiting priorities belong to Inventory. A latched
reorder cycle continues successive bounded batches after stock falls low; it
reserves no absent supply. Human enabled targets are standing orders; an AI
controller uses its ordinary tree and the same physical dispatch boundary.

`resources/production` owns finite recipe work over those actual stores. Inputs
are consumed at the first paid step with all output space secured; interruptions
retain that work and claims. A deliberate recipe change discards unfinished
consumed material and releases its output claims. Recipe planning consumes only
permitted Inventory reports, while execution checks actual stocks and floors.
Production state, claims, local energy and private AI frontiers participate in
capture, restore preflight and deterministic digests.

`resources/recovery` carries paid fractional tool work and selected intent.
Source manifests bind resource types to finite existing endpoints; no kill loot
or HP conversion creates material. The Simulation adapter acquires contacts
through explicit Radar feeds and measures candidates within actual tool range.
An acquired target remains tracked while Science selects another contact.
Human and ordinary AI use the same admitted selection/enable controls, paid
energy and finite transfer settlement. Exact receipts settle before capture/fold;
invalid or missing recovery owners, destination bindings or receipt frontiers
refuse restoration. Empty sources can remain selected and paused.

Workshop's production and resource-recovery panels edit these declarations
through existing accepted-source history, materialisation, validation, Preview
and disposable Test. The Cruiser recovery probe and finite cargo/deposit
templates exercise acquisition and optional ore/scrap processing without adding
a crew seat.

`resources/ports` owns physical-visit identities, delivered-lot quotas and approved
orders, without owning a second inventory. `ports_server` connects actual Dock
coupling and explicitly permitted Inventory reports to the existing Umbilical.
Free allied services replenish authorised targets; manual offers require a human
order or the typed trusted World effect. Ordinary AI can execute approved work,
but cannot originate purchasing authority. Empty faction permissions deny access;
zero quota disables an offer. Positive-quota donations do not spend the outbound
allowance. Ended visits clear unperformed work while retaining delivered goods.

The resource settlement adapter stages all goods and barter-cost endpoints
together. Costs require pre-existing available stock; final net quantities and
shared space permit valid exchanges at full endpoints. Receiving-bay cost stores
take priority, then stable store identity. Stock, quota and publication commit in
stable UUID order. Partial Operation faults reduce useful output after energy is
paid; disabled tools pause. Snapshot preflight checks the complete candidate
Dock/provider/order roster before writes, and pending port exchanges must drain
before capture or fold. Workshop edits terms through its existing source/history
workflow; the Umbilical screen holds editable, visit-bound manual orders.

An optional `port_service.system` binds the provider's actual composed Dock.
Axiom and the existing manual-trade provider use it. `ports_equipment.rs`
stages both endpoint Power owners and the same PortOrder fraction/receipt
against the current finite exchange in stable visitor UUID order. Unavailable
Operation/control, insufficient funding or rejected stock/headroom produces no
debit. Partial paid work remains in the existing order; actual delivered lots
are recorded only after stock commits. Restored equipment/power and existing
Dock/visit state are consulted directly, without a second service-availability
cache or provider progress clock. Public feedback describes the known coupled
service's availability without publishing foreign reserve/component detail.
Workshop's existing port form preserves the actual producer binding and offers
an explicit legacy passive choice. Omitted bindings retain the legacy path;
this facility foundation does not establish physical workforce, shipments or
the complete outside-actor network.

The individual Umbilical AI gives an explicit permitted Transfer directive
priority over opportunistic Main-store collection. It disables maintenance through
the same admitted command before starting the retained resource/direction/store.
An idle free-allied visit still enables replenishment, including equipment whose
default direction is Deliver. Delegating an already running unclaimed human flow
preserves its mode; ordinary standing-operation ownership still controls withdrawal.
The pure decision is in Gameplay `umbilical/flow.rs`; the existing Simulation
Umbilical host emits and applies it without another inventory or controller.
