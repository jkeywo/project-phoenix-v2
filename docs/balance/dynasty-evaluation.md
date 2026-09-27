# Alliance–Dynasty cruiser evaluation

## Ratified rule (#1547)

The user ratified this rule during the T5 implementation session:

- Run **200 seeded duels**, with mirrored starting sides and varied engagement conditions.
- A win contributes one win to its cruiser; a draw or simulation timeout contributes **half a win to each cruiser**.
- Divide by **all 200 duels**. The approximate parity target is **40–60%** for each cruiser. Report draws and timeouts separately.
- Pin seeds, content, configuration and report provenance. Follow with the working 2v2 scenario and preserve the existing NPC cruiser.

Process failures are not simulated draws: they invalidate the batch. A partial
or failed batch cannot meet the criterion, regardless of its measured rate.
A numerically passing batch is not evidence of crew usability.

## Repeatable matrix

Seeds **1547001–1547020** are fixed across five conditions and two placements:
`5 × 20 × 2 = 200`.

| Condition | Separation | Alliance heading | Dynasty heading |
| --- | ---: | ---: | ---: |
| Close head-on | 40 | 0 | π |
| Medium head-on | 80 | 0 | π |
| Long head-on | 160 | 0 | π |
| Alliance initially facing away | 80 | π | π |
| Dynasty initially facing away | 80 | 0 | 0 |

Angles are radians. Alliance begins at positive Z and Dynasty at negative Z.
The paired placement rotates the whole engagement by π: both positions and
headings change. Hull/faction identity and entity order remain fixed. Each run
starts with the hull's ordinary reserve and Backfill state and has a **600
simulation-second** limit. No reserve preload or privileged AI damage is used.

The runner derives two fixed berths and the one-loss team threshold from
`assets/worlds/cruiser_elimination.toml`; other result and Objective rules remain
shared. It avoids the legacy NPC duel transform, whose side-faction override is
unsuitable for a playable Dynasty hull.

The follow-up runs the unchanged four-cruiser reference world for seeds
1547001–1547010 in both placements (**20 team fights**). Report those outcomes
alongside duel parity; identical playstyles or team win rates are not required.
The reference scenario tests separately exercise a real human-controlled
Station with the remaining Stations and ships on Backfill.

## Provenance and execution

`scripts/balance-dynasty.mjs` records the source revision and Rust source patch,
executable and runner SHA-256, hashes of all authored TOML/Rhai/CSV/JSON assets,
the exact authored content patch for any uncommitted tuning candidate,
generated world hashes, exact arguments, seeds, every source AAR and log,
report hashes, counts and rates. Retain each batch in a fresh output directory.
`--limit` is for diagnosis and cannot produce an accepted batch.

Only the scenario's simultaneous-elimination flag is a draw. An unfinished
fight at the full 600-second limit is a timeout, including a low-activity AAR
that the generic headless classifier labels `draw`. A premature unfinished
report or contradictory terminal flags invalidate the run.

```powershell
cargo build --release --features headless --bin phoenix-headless
node scripts/balance-dynasty.mjs --binary target/release/phoenix-headless.exe --out .balance-dynasty/final
```

An explicitly named debug binary is also supported; provenance records its
bytes. Outcomes use fixed ticks and seeded RNG, not elapsed wall time. Coordinate
expensive Cargo commands with the other implementation tasks.

## Evidence status

The rule is a recorded human decision. The matrix and runner implement that
decision. The [27 September 2026 result record](dynasty-results-2026-09-27.md)
contains the completed batches, final tuning, 43%/57% duel result, strongly
Alliance-favoured team follow-up and reproducible evidence. Numerical success
does not establish crew acceptance.

Crew checks remain in `docs/acceptance/1549-cruiser-elimination.md` and the Dynasty
Console acceptance work. Existing `assets/entities/ship_harrow_cruiser.toml` and
Alliance/NPC baseline content stay unchanged by tuning: change only the distinct
playable Dynasty hull's authored overrides.
