# Increment 8 initial energy tuning

These values are **provisional AI-authored tuning**, approved as an implementation starting point. They have not yet passed integrated gameplay, balance, performance or human-workload acceptance. Authored entity fragments are the runtime source of truth; root integration owns physical work grants and ordinary Power control.

Each real consumer has one local reserve and one fine `energy-<SystemId>` group. Existing coarse group identities and labels remain; their maximum/default flow and cooling are the sums of their children. Coarse groups contain no additional energy store or duplicate physical demand. Administrative views, phaser control, aggregate shield control and legacy joystick/engine control surfaces receive no extra reserve. Synthesized shield arcs use their exact stable `shield-arc-*` identities.

The normalized-work starting costs are one unit per second for continuous machinery and one unit per actual discrete operation/projectile. Bolt banks, tubes, magazine grants, transporters and Comms use discrete cost only. Sensors, navigation and security expose both work kinds; the runtime charges only work actually performed. Other consumers use continuous cost only.

For continuous cost C and discrete cost D, capacity is max(30C, 6D, 10), initial energy is 75% of capacity, maximum intake is max(2C, D, 1), and full-reserve passive loss is 1% of capacity per second. Default incoming flow is nominal demand plus half-full loss, capped by intake. This permits a stable partial reserve and additional intake before an anticipated crisis.

Charge and discharge each generate 0.1 heat per energy unit. Passive cooling removes up to 0.2 heat per second; coolant removes ten heat per unit consumed. Consumer thermal trip/recovery thresholds are 50/30, with default/maximum active cooling 1/5 heat per second. Flow restoration delay is 0.5 seconds. Actual heat removal, not allocation alone, consumes coolant.

Reactor sustainable output equals the sum of fine default flows, peak output is 1.5 times sustainable, and ramp rate is one quarter sustainable per second. Its capacitor capacity is three times sustainable, initially 75% full, with intake limit twice sustainable. It uses the same reserve/heat mechanisms. Default reactor cooling is max(1, 0.2*sustainable) and maximum cooling max(5, 0.4*peak), accounting for gross charging plus discharging heat at ship-scale flow rather than imposing a consumer-sized cooling ceiling.

Each hull carries max(100, 30*consumer count) coolant in an explicit finite capacity exposed by `resource_services.coolant`. Axiom carries a finite 10,000-unit service stock. Existing umbilical resource choices remain unchanged; an explicitly configured coolant collection journey must demonstrate replenishment through the normal docked service link.

The root-owned closed Power-tree generator was invoked once during authoring. Each reactor owns an explicit tree, preserving other System configuration; no runtime generator or fallback is introduced. Numeric literals were matched to authoritative f32 hardware values before serializing. Low reserve raises the requested flow; high heat raises cooling; ordinary allocation, admission and source limitations remain authoritative.

| Entity / fixture | Consumers | Coarse groups | Sustainable | Peak | Coolant | AI tasks |
|---|---:|---:|---:|---:|---:|---:|
| alliance_battleship.toml | 26 | 6 | 30.400 | 45.600 | 780 | 131 |
| alliance_courier.toml | 14 | 6 | 17.500 | 26.250 | 420 | 83 |
| alliance_cruiser.toml | 26 | 6 | 31.700 | 47.550 | 780 | 131 |
| alliance_destroyer.toml | 25 | 6 | 30.250 | 45.375 | 750 | 127 |
| alliance_tender.toml | 5 | 3 | 5.750 | 8.625 | 150 | 35 |
| dock_probe.toml | 1 | 1 | 1.150 | 1.725 | 100 | 11 |
| dynasty_player_cruiser.toml | 26 | 7 | 31.700 | 47.550 | 780 | 135 |
| lifecycle_tender.toml | 2 | 2 | 3.150 | 4.725 | 100 | 19 |
| repair_tender.toml | 1 | 1 | 1.150 | 1.725 | 100 | 11 |
| ship_civilian_hauler.toml | 4 | 2 | 5.450 | 8.175 | 120 | 27 |
| ship_harrow_cruiser.toml | 12 | 3 | 13.350 | 20.025 | 360 | 63 |
| ship_harrow_destroyer.toml | 9 | 3 | 10.050 | 15.075 | 270 | 51 |
| ship_harrow_patrol.toml | 9 | 3 | 10.350 | 15.525 | 270 | 51 |
| ship_harrow_tug.toml | 5 | 3 | 5.750 | 8.625 | 150 | 35 |
| ship_harrow_warhawk.toml | 13 | 3 | 14.350 | 21.525 | 390 | 67 |
| ship_requiem_courier.toml | 6 | 2 | 6.900 | 10.350 | 180 | 35 |
| station_axiom.toml | 2 | 1 | 2.300 | 3.450 | 10000 | 15 |
| test/rng_coverage_lancer.toml | 11 | 3 | 12.200 | 18.300 | 330 | 59 |
| tractor_tug.toml | 1 | 1 | 1.150 | 1.725 | 100 | 11 |
| umbilical_probe.toml | 2 | 2 | 2.300 | 3.450 | 100 | 19 |
| fragments/composed_escort.toml | 3 | 1 | 3.450 | 5.175 | 100 | 19 |

The 21 complete definitions contain 203 local reserves, including the existing composed-escort fixture. A lightweight read-only authoring check verified reference coverage, hierarchy/cap sums, real coolant endpoints and task limits. It also compared the local weapon definitions, weapon-console data, torpedo payloads, shield-arc tuning, Stations, doctrine, hulls and visuals with the pre-migration source: those fields are unchanged. No scenario source or weapon balance threshold was edited.

Remaining integration checks include Rust content admission and synthesized-arc bindings, non-Ship Axiom execution, fixture/world capacity overrides retaining coolant, playable human/AI allocation, save/restore and deterministic digests, cold-flow versus explicit restraint, finite coolant replenishment, and intentional gameplay/balance evidence. Older Power tuning is retained in source until the root-owned controller/runtime switch removes its old role; it is not a physical-flow conversion formula.
