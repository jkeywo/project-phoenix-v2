# Alliance destroyer recreation

An editable hard-surface interpretation of `raw/models/PPAllianceDestroyer.png`.
The broad bow, wing gun recesses, raised command bridge, white/graphite armour
and blue apertures follow that reference. Revision 2 lowers the bridge, lengthens
the bow and uses curved plan outlines and rolled armour shoulders. The stern and underside are inferred
because the concept only shows one angle. The deliberately simplified geometry
and procedural panel atlas give this candidate a cleaner, more stylised finish
than the concept's dense surface detail.

Run from the repository root, using the installed Blender 5.0.1 and the existing
Node dependencies. No network service or additional package installation is used.

```powershell
node scripts/art/destroyer-atlas.mjs
& 'C:/Program Files/Blender Foundation/Blender 5.0/blender.exe' --background --factory-startup --python scripts/art/recreate-alliance-destroyer.py
node scripts/art/inspect-destroyer.mjs
& 'C:/Program Files/Blender Foundation/Blender 5.0/blender.exe' --background --factory-startup --python scripts/art/render-destroyer-comparison.py
```

Outputs are in `raw/models/PPAllianceDestroyer/recreated/`:

- `alliance_destroyer_recreated.blend`: editable, named parts and packed textures,
  plus the studio camera and lights. The master preserves polygonal surfaces.
- `alliance_destroyer_recreated.glb`: one joined, triangulated mesh and one
  back-face-culled PBR material, with normals, UVs and precomputed tangents.
- `destroyer_base.png`: 2048px colour atlas; `destroyer_normal.png`: 1024px;
  `destroyer_emissive.png` and `destroyer_orm.png`: 512px packed material data
  (green roughness, blue metallic; red is white, not baked occlusion).
- `destroyer_atlas.svg`: editable source for colour details and lettering.
- `preview_hero.png`, `preview_aft.png`, `preview_top.png`, `preview_front.png`,
  `preview_side.png`: transparent renders of the **re-imported GLB**, verifying
  the export's material conversion as well as its geometry.
- `preview_existing.png`: existing shipped mesh rendered under the same studio
  lights and camera, centred and uniformly scaled to the candidate's width.
- `metrics.json`, `comparison.json`, `validation.json`: measured output costs
  and the Khronos glTF validator report.
- `v1/`: the first-pass GLB, Blender master, hero render and metrics, retained
  for comparison. Revision 2 trades a larger texture footprint and 20,048
  triangles for smoother geometry and finer panels, stencils and wear. Its
  triangle count and file size remain below the shipped model; texture pixels
  are now higher than the shipped model, not lower.

The inspector validates the GLB and checks the triangle, texture and byte
budgets, single primitive/material, precomputed tangents, back-face culling
and absence of external texture or buffer dependencies.

The recreated model is used by the Alliance destroyer entity. `raw/` is ignored by Git, so the master files
remain local; the scripts preserve the recreation recipe. The original live
model is retained; its attachment positions and directions are preserved in the new sidecar's identity frame.
Its coordinate convention is Blender +Z up / -Y bow, glTF +Y up / +Z bow.
The separate `assets/models/alliance_destroyer_recreated.glb` viewer candidate
has its dimensions, centre and orientation matched to the original GLB with
its base rig applied. This transform is embedded in the candidate's GLB scene
graph; its sidecar uses identity. Do not copy the old advisory extents: they
are larger than the old mesh's actual rendered bounds. Regenerate with:

```powershell
node scripts/art/prepare-destroyer-lods.mjs
node scripts/generate-lods.mjs alliance_destroyer_recreated --remesh
node scripts/capture-billboards.mjs alliance_destroyer_recreated
node scripts/generate-model-index.mjs
```

The near mesh retains all details. Distant meshes use the existing voxel pre-pass
at 0.065 raw-model units, followed by meshoptimizer ratios 0.1 and 0.025 and
256px / 128px texture caps. The checked-in `.remesh.glb` intermediate lets the
ordinary LOD command regenerate without Blender. The 8-view billboard uses
256px tiles at 20 degrees pitch. The original distance bands (15, 100, 400)
are retained. Both provenance manifests are updated by the standard tools.

No frame-rate improvement has been measured: the comparison reports
asset costs, not an FPS benchmark.

Fleet recreation for Combat Test and Falling Skyway
-------------------------------------------------

`fleet.json` maps nine distinct models to their concept sheets. The Cruiser
uses `PPAllianceStarship.png`. `recreate-fleet.py` authors the Alliance ships,
ring starbase, research station and Dynasty ships as separate silhouettes,
using `fleet_geometry.py` for mesh/atlas primitives. Every exported model has
one primitive and one PBR material. Editable packed Blender masters, textures
and hero/top renders are in `raw/models/<concept>/recreated/` (ignored by Git).

```powershell
node scripts/art/build-fleet.mjs
node scripts/art/prepare-fleet.mjs
# Repeat these two commands for each <model>_recreated from fleet.json:
node scripts/generate-lods.mjs alliance_cruiser_recreated
node scripts/capture-billboards.mjs alliance_cruiser_recreated
node scripts/art/check-fleet.mjs
node scripts/art/integrate-fleet.mjs
node scripts/generate-model-index.mjs
npm run lods:check
npm run lod-captures:check
```

`PHOENIX_BLENDER` overrides the Blender executable. The builder and preparation
script accept model names to rebuild a subset; preparation preserves the other
fitted assets and report entries. The atlas generator accepts `PHOENIX_ART_OUT`,
`PHOENIX_ART_TITLE` and `PHOENIX_ART_FACTION` when invoked by the fleet builder.

Preparation fits the actual rendered GLB bounds, not advisory cached extents.
It bakes the fitted transform into all three geometry sources and transfers
markers and anonymous target points into the identity sidecar frame. The
courier's `dock_probe` variant gets separate fitted geometry and three LODs:
its original identity rig drew a different size from the ordinary courier.
The umbilical berth keeps that variant and its two authoritative dock plates.

The middle/far sources under `scripts/art/lod-sources/` are generated from the
same authoring recipe with fewer curves, fittings and ribs. They are kept in
the repository so the standard LOD generator and hash checks work without
Blender or ignored raw masters, and are outside the shipped asset bundle.
This avoids voxel remeshing thin station rings, spokes and solar arrays.
The sidecar retains the original distance bands and owns the simplification
ratios, texture caps and billboard recipe. A higher station middle-tier cost
than the initial draft was explicitly accepted for rounded spokes/berths.

`fleet-fit.json` records bounds and original rig data for the independent
quaternion checks in `check-fleet.mjs`; `fleet-validation.json` records actual
GLB triangle counts, file sizes, validation results and attachment counts.
`integrate-fleet.mjs` changes only the twelve scenario entity model references
and verifies every other parsed entity value remains identical.

The Dynasty Destroyer uses `PPDynastyDestroyer.png` and replaces the battleship
mesh previously assigned to `ship_harrow_destroyer`. Its scale and rig are fitted
to the original Dynasty Destroyer asset. The Alliance cruiser has an unarmed
visual hull, elevated nacelles and a rounded bow; the courier has a deeper aft
belly and blended wing roots. Research arrays alternate with laboratories on
radial spokes, and both dish assemblies face away from the centre. Dynasty wings
use paired Bezier boundaries and closed airfoil sections, with gun sockets
embedded in their surface. The battleship has a restored four-tier tower;
the outer fitted bounds still match the original asset.
Dynasty simplification errors are limited to 0.0015 for the middle level and
0.003 for the far level to preserve the open crescent silhouette.

The Harrow cruiser retains its broad forebody outline with a flatter vertical
section, layered dorsal plates, exposed curved ribs and flank reactors. Harrow
battleship batteries sit on thick forebody armour and reinforced wing decks.
Its four-tier aft tower rises above the hull; the forebody's panel joints are
shallow Boolean cuts into the solid armour, rather than overlapping raised plates.
The Alliance battleship has no visual turrets at any detail level; its fixed
spinal laser has hull-integrated shoulders, field coils and a recessed blue
focusing lens at the bow.
