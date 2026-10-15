# Facility21 — equipment-bound replenishment foundation

This source checkpoint follows Personnel20's `320e8f0e` checkpoint and its
`2eddbe66` fixture correction. It implements the first consumer in the
[increment21 contract](../gameplay-redesign/implementation/increment-21.md),
not the complete facilities, workforce or outside-actor scope.

Optional `port_service.system` binds replenishment to an actual composed Dock
System, its Console, physical placement, Operation component and positive-cost
local energy consumer. Axiom and the existing manual-trade provider bind their
existing Dock. No stocks, terms, capacities, geometry or scenario timing change.
Omitted bindings retain the legacy passive provider path.

The existing PortOrder fraction and receipt remain the work frontier. Settlement
stages both existing Power owners over a common affordable work interval, then
commits the existing finite stock/quota/capacity exchange in stable visitor UUID
order. Rejected final plans do not debit energy or advance work. Offline or zero
Operation denies service; partial condition or funding limits actual output.
The visitor receives a generic coupled-service interruption reason, without
foreign reserve or component details. Repair uses the existing Repair owner,
parts and paid work; it never replenishes provider supplies.

Workshop edits the binding through the existing port form and accepted-source
history. Ship, Energy, Interiors and Damage operations guard the actual bound
references. Explicit legacy selection removes the binding. Parsed source parity
and current-document checks protect unrelated content and stale forms.

Targeted JavaScript validation passed 39 distinct tests across the port source
and form plus adjacent Ship, Energy, Interiors and Damage forms. One obsolete
source assertion was migrated from the retired legacy Dock field to the actual
DockMating component; the original berth geometry assertions remain. Strict
strings reported 5911 with zero errors or warnings; layer checks passed.
PASM validate passed with 1115 entities, 614 warning/concern findings and
StatusOK; scan and traceability also exited zero. Existing warning/concern
findings remain informational.

New native source cases cover closed optional binding, actual provider
composition, paired payment and duplicate-tick refusal, unavailable work and
rejected stock without debits, partial funded output, real external Repair
recovery, stable two-visitor settlement, and a fresh JSON restore during paid
fractional work followed by 360 exact existing-owner comparisons. These cases
have not been compiled or executed at this source checkpoint. The JSON case is
an owner continuation test, not a full-host capture/digest or mesh proof.

Independent read-only review covers production paired settlement and the six
new owner tests, Model/config acceptance, source/forms and binding-only data.
No browser, workload,
balance or full acceptance result is claimed. Physical workforce, shipment and
dependency networks, outside actors, wider21 and unfinished20 Medical/Security/
external work remain open.
