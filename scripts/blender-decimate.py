# Pre-pass for scripts/import-asteroids.mjs (issue #946).
#
#   blender --background --factory-startup --python scripts/blender-decimate.py \
#           -- <input.glb> <output.glb> <target_triangles>
#
# Why this exists, and why gltf-transform is not enough
# ----------------------------------------------------
# The raw asteroid scans under raw/models are not exported at a uniform
# density. PPAsteroidCommon1-4 arrive at ~31k triangles and ship almost as-is;
# PPAsteroidUncommon1-4 and PPAsteroidRare1-4 arrive at 500k-860k triangles
# behind the same 2048px maps.
#
# `gltf-transform simplify` cannot bring those down. Its weld is bitwise, so
# the UV and normal seams the exporter split (uncommon 2: 431k vertices over
# 500k triangles, where a closed surface would have ~250k) survive as
# topological borders that meshoptimizer refuses to collapse across. Measured:
# `--ratio 0.05` stalls at 116k triangles and the error bound makes no
# difference — 0.005, 0.05 and 0.5 all land on the same 116,000.
#
# Blender's Decimate modifier in COLLAPSE mode does not have that problem: it
# collapses interior edges and carries the UV layer with them, so a 500k
# triangle scan reaches the commons' density with the parameterisation intact.
# That is the difference from scripts/blender-voxel-remesh.py, which rebuilds
# the surface from scratch, loses the UVs entirely and re-projects them
# approximately — affordable for a FAR LOD level and, as that script says, for
# nothing else. This pre-pass IS used on the near level, so it must be the
# UV-preserving one.
#
# Like the voxel pre-pass, this is a local, opt-in command: its output is the
# checked-in `.glb`, so only whoever imports a new rock needs Blender at all.
# Neither `npm run lods` nor CI ever invokes it.
#
# The gotcha both scripts share: Blender's startup file ships a default cube
# (plus a camera and a light). Import a GLB on top of that and the cube is
# still in the scene, so it gets decimated, joined and exported as part of the
# asset. `--factory-startup` guarantees the same startup scene on every
# machine; the delete pass below then guarantees it is empty before the import.

import sys

import bpy


def parse_args(argv):
    """Everything after the `--` separator Blender uses for script arguments."""
    if "--" not in argv:
        raise SystemExit("usage: blender --background --python <this> -- <in> <out> <triangles>")
    args = argv[argv.index("--") + 1:]
    if len(args) != 3:
        raise SystemExit("usage: ... -- <input.glb> <output.glb> <target_triangles>")
    return args[0], args[1], int(args[2])


def clear_scene():
    """Empty the startup scene — the default cube above all."""
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    # Deleting the objects leaves their data behind; purge it so nothing can be
    # revived by the exporter or by a stray reference.
    for collection in (bpy.data.meshes, bpy.data.materials, bpy.data.images):
        for block in list(collection):
            collection.remove(block)


def mesh_objects():
    return [obj for obj in bpy.context.scene.objects if obj.type == "MESH"]


def triangle_count(obj):
    """Triangles in the evaluated mesh — n-gons counted as they will export."""
    return sum(max(len(polygon.vertices) - 2, 0) for polygon in obj.data.polygons)


def decimate(obj, target_triangles):
    """Collapse `obj` towards `target_triangles`, keeping its UV layer.

    Returns the ratio used, or None when the mesh is already at or below the
    target (decimating up is not a thing, and a modifier at ratio >= 1 is just
    a slow copy).
    """
    current = triangle_count(obj)
    if current <= target_triangles:
        return None

    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)

    modifier = obj.modifiers.new(name="Decimate", type="DECIMATE")
    modifier.decimate_type = "COLLAPSE"
    modifier.ratio = target_triangles / current
    # Without this, collapse leaves n-gons behind wherever it can and the
    # exporter triangulates them on the way out — so the file lands above the
    # target it was asked for. Triangulating inside the modifier keeps the
    # count the ratio implies.
    modifier.use_collapse_triangulate = True
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    return modifier.ratio


def main():
    input_path, output_path, target_triangles = parse_args(sys.argv)

    clear_scene()
    bpy.ops.import_scene.gltf(filepath=input_path)

    objects = mesh_objects()
    if not objects:
        raise SystemExit(f"no mesh found in {input_path}")
    for obj in objects:
        before = triangle_count(obj)
        ratio = decimate(obj, target_triangles)
        after = triangle_count(obj)
        if ratio is None:
            print(f"[blender-decimate] {obj.name}: {before} triangles already <= {target_triangles}")
        else:
            print(f"[blender-decimate] {obj.name}: {before} -> {after} triangles (ratio {ratio:.4f})")

    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.export_scene.gltf(
        filepath=output_path,
        export_format="GLB",
        use_selection=True,
    )
    print(f"[blender-decimate] {input_path} -> {output_path} (target {target_triangles})")


main()
