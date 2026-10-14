# Increment 6: stores, transfers and service links

The user instructed “do 6 then” on 14 October 2026. This authorises advancing;
prior physical-device and human acceptance remains open. Adopt
`pasm/spec/design/resource-stores.yaml` before runtime changes.

Migrate the existing magazine and named infrastructure capacities onto shared
bounded integer stock arithmetic, resource requirements and stage reservations.
Keep each stock in its existing owner. A loading tube claims its next round at
load start; the available magazine count falls then, as before. Cancellation
releases the claim; completion consumes it. Loaded rounds remain physically held
by tubes. Do not count a claim twice or allow replenishment to fill space still
occupied by tube-held ammunition. Preserve existing grant policy and admission.

Introduce authored named services resolving to capacity or magazine endpoints,
with explicit consumer link overrides. Reuse existing docked umbilical controls,
AI and flow arithmetic. Ordinary capacity references remain direct supported
endpoints, not an old-data adapter. Validate missing/incorrect service references
before loading or Workshop Test. Connections confer no new information/control.

Plan transfers in stable UUID order, securing source stock and destination room
together, including preceding same-tick claims. Commit both ends together or
neither. Keep infrastructure counters, threshold events and stock order/labels
through their current publication boundary. Add an authored ammunition-service
demonstration using real stock and the ordinary docked transfer flow.

Persist canonical stocks, reservation identity and fractional flow progress.
Advance compatibility explicitly; validate restored claims against their tube
activities. Include state in deterministic digests without parallel scalar stock
or duplicate simulation. Integrate supported authoring into Workshop history,
validation, save/reload, Preview and disposable Test.

Verify scarcity, cancellation/completion, multi-tube exclusion, loaded-round
headroom, simultaneous transfers, conservation, refusal and ordinary AI/human
controls. Verify save/restore and replay, shipped-content compatibility and
permitted projections. Run targeted owner tests while developing, independent
review, and the required integrated gates once on the integrated implementation.
Compare representative reports/performance and unchanged Cruiser thresholds.

No new consumable drains, coolant, parts, stock targets, fabrication, automatic
internal delivery, compartments or personnel-rescue inventory are introduced.
Those remain their scheduled increments. Keep GM optional and networking intact.
