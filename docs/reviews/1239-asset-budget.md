# Asset budget recovery (#1239, #1055, #1056)

8 September 2026, #1449 A3. The release `phoenix-perf` executable was rebuilt
from runtime base `349dea20` before pre-init code changes, using the ordinary
release profile (`perf` feature). Its asset measurement code is unchanged.

The initial `assets` capture failed `assets.glb.without_lod`: **5 versus 0**.
These are five entity templates sharing the same courier `dock_probe` variant:
`alliance_tender`, `dock_berth`, `dock_probe`, `umbilical_berth`, and
`umbilical_probe`. The sidecar lacked a ladder. It now uses the existing courier
LOD1/LOD2 meshes after the near original, with an unbounded final tier. Its
identity base and docking markers are unchanged. The default courier billboard
is deliberately absent because it was captured with a different base transform.

Commands after the sidecar change (each exit 0):

```powershell
target/release/phoenix-perf.exe assets --capture target/issue-1449/assets-fixed.json
target/release/phoenix-perf.exe report --capture target/issue-1449/assets-fixed.json --gate
target/release/phoenix-perf.exe mesh --capture target/issue-1449/assets-mesh-fixed.json
target/release/phoenix-perf.exe report --capture target/issue-1449/assets-mesh-fixed.json --gate
npm run lods:check
```

| Metric | Result | Current expected |
| --- | ---: | ---: |
| Largest GLB bytes | 13,601,536 | 13,601,536 |
| Total GLB bytes | 181,030,868 | 175,619,464 (+3.1%, within budget) |
| Entity GLBs without LOD | 0 | 0 |
| Maximum ladder depth | 4 | 4 |
| Maximum triangles | 131,560 | 131,560 |
| Total triangles | 1,968,208 | 1,968,208 |
| Maximum texture count | 3 | 3 |
| Maximum texture pixels | 4,194,304 | 4,194,304 |

All eight reported metrics passed. All 44 generated LOD outputs were current.
The native `every_shipped_ladder_declares_the_tier_rig_its_files_actually_have`
test passed against the changed sidecar and count (in a nine-test focused run).
Independent read-only review: PASS, including identity tier sizing and marker
preservation. No baseline was adopted or edited. Existing baselines had already
accepted the older starbase size/texture growth described by #1055/#1056.

This is asset evidence only; it does not discharge the headless-release timing
drifts in #1247/#1272. Final integration gates and push are separate.
