---
title: Resource Stores and Service Links
type: concept
tags: [resources, inventory, torpedoes, infrastructure, umbilical, workshop, continuation]
sources: [crates/phoenix-sim-gameplay/src/resources/stock.rs, crates/phoenix-sim-gameplay/src/resources/service.rs, crates/phoenix-sim-gameplay/src/weapons/torpedo.rs, crates/phoenix-sim-gameplay/src/infrastructure/condition.rs, crates/phoenix-simulation/src/resources/server.rs, crates/phoenix-simulation/src/umbilical/server.rs, crates/phoenix-simulation/src/infrastructure/server.rs, crates/phoenix-simulation/src/snapshot/preflight.rs, crates/phoenix-simulation/src/sim_digest.rs, editor/workshop-resources.js, gui/workshop-resource-panel.js]
updated: 2026-10-14
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

Simulation's resource transaction adapter plans each flow in stable operator and
endpoint order against canonical stock and preceding queued stock effects. It
commits both ends conservatively. Infrastructure adjustments retain their
existing drain, counter publication and threshold events; ammunition movement
uses the magazine owner. The transaction view lasts only for that drain.

Snapshot 42/rules 0.14 preserve stocks, claims and fractional flow carry.
Preflight validates tube identity, capacity, timers and claim relationships
before writes. Resource digests include magazine/tube commitments and facility
stocks. Other weapon effect state retains its existing deferred fold boundary.
Services are rederived authored configuration. Protocol and transport are unchanged.

Workshop's Composition resource panel edits stocks, magazine quantity, services
and consumer links through ordinary validation, source history, export/reload,
Preview and disposable Test. Inherited data requires explicit materialisation;
endpoint variants and complete link bindings replace atomically. Resource
definitions in Live Inspector require recreation and expose no new actions.
