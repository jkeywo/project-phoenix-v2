# Increment 6: stores, transfers and service links

Contract: [increment 6](../gameplay-redesign/implementation/increment-6.md).

## Implemented scope

Canonical shared stocks replace the scalar magazine and capacity quantities
inside their existing owners. Tube-stage claims preserve available ammunition
timing while making cancellation, completion and continuation explicit. Loaded
rounds occupy storage space during replenishment. No stock mirror, new AI
controller or changed command authority is introduced.

Named authored services and consumer-owned provider overrides resolve capacity
or torpedo-magazine endpoints. Existing direct-capacity flows retain their
docking, power, damage and authored-rate rules. Stock transactions account for
preceding queued effects and contending links in stable order, preserving goods
at both endpoints. Facility counters and threshold events still publish through
the infrastructure owner.

Snapshot 42/rules 0.14 retain canonical stocks, claims, tube activities and
fractional umbilical carry. Restore rejects malformed or absent inventory,
orphan claims, over-capacity tubes and invalid timers before writing state.
Single-entity GM recovery uses the same preflight. Networking/protocol 6 remains.

Two intentional conservation/continuation corrections accompany migration:
partial-volley firing releases an unfinished loading claim; fractional flow
progress survives restore. Contending flows cannot overbook the same stock or
destination room. These corrections are tested explicitly rather than preserved
as previous defects.

Workshop edits capacity quantity/limit/label, magazine quantity, named services
and consumer links through the existing exact-source history and validation.
Materialisation, undo/redo, export/reload, actual Preview and disposable Test
remain the supported journey. Tagged endpoints and complete links replace
atomically during composition.

## Evidence

Command receipts and gameplay/performance comparison are recorded during final
integration. Independent reviews passed the shared stock/magazine implementation,
authored schema/Workshop and runtime transfer/continuation integration. Physical
device and human-workload evidence is separate from automated tests.

## Playable configuration and human acceptance

On the Cruiser, choose its existing dock and Engineering umbilical controls.
First exercise the existing reserve-fuel flow. Then use Workshop to configure
ammunition service on a disposable copy:

```toml
[resource_services.torpedoes]
kind = "torpedo_magazine"

[umbilical]
capacity = "torpedoes"
direction = "collect"
# Retain the hull's authored rate and minimum power.
```

The Cruiser already declares the ammunition endpoint. Configure a dockable
provider with real stock; the named resource may bind a differently named store:

```toml
[resource_services.torpedoes]
kind = "capacity"
id = "ordnance"

[[infrastructure.capacity]]
id = "ordnance"
amount = 50
ceiling = 50
```

Fire ammunition before replenishing: a full ship should receive nothing, even
when some rounds are loaded or loading. Dock with the provider and start the
ordinary umbilical. Confirm the provider loses exactly what the magazine gains.
Stop/undock or interrupt power; confirm normal refusal/AI behaviour. Save/resume
mid-flow and mid-loading; reclaim/reconnect without resetting work or losing
fractional progress. Repeat supported 2/4/8-seat journeys.

In Workshop edit stocks/services/links, validate bad references, materialise
inherited data, undo/redo, export/reload, Preview and run disposable Test. Confirm
the supplying dependency is unchanged. Judge readable controls and attention
cost on physical phones and native Station displays.

Physical phone, native multi-monitor and human-workload acceptance remain open,
including prior increments. The user explicitly authorised Increment 6 while
those checks remained pending. Increment 7 has not started; commits remain local.
