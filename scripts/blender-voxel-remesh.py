# Optional pre-pass for scripts/generate-lods.mjs (issue #919).
#
#   blender --background --factory-startup --python scripts/blender-voxel-remesh.py \
#           -- <input.glb> <output.glb> <voxel_size>
#
# Some meshes decimate badly: non-manifold geometry, overlapping shells and
# holes give meshoptimizer nothing coherent to collapse, and the result folds
# in on itself long before the target ratio. A voxel remesh rebuilds the model
# as a single watertight surface first, which then decimates predictably.
#
# This is deliberately NOT part of the main command. A level opts in by putting
# `remesh_voxel_size` in its `[lod.generate]` block; the intermediate written
# here is checked in, so only whoever re-runs the pre-pass needs Blender at all
# — `node scripts/generate-lods.mjs` (and CI) never invoke it.
#
# Caveats, because a voxel remesh is a destructive rebuild:
#   - UVs and materials do not survive it. Re-texture the intermediate (or use
#     it only where the far LOD's shading is carried by vertex colour / a flat
#     material) before wiring it into a ladder.
#   - Voxel size is in the model's own units — the geometry as it sits in the
#     GLB, NOT the world-space size the rig sidecar's `[extents]` reports after
#     `[base] scale`. The shipped asteroids are ~1.9 units across in their own
#     units and ~8 after a 4.2x base scale, so a voxel of 1.0 (which looks small
#     next to 8) spans half the rock and rebuilds it as a cube. Too coarse melts
#     detail; too fine produces more triangles than you started with.
#
# The gotcha this script exists to get right: Blender's startup file ships a
# default cube (plus a camera and a light). Import a GLB on top of that and the
# cube is still in the scene, so it gets remeshed, joined and exported as part
# of the asset. `--factory-startup` guarantees the same startup scene on every
# machine; the delete pass below then guarantees it is empty before the import.

import sys

import bpy


def parse_args(argv):
    """Everything after the `--` separator Blender uses for script arguments."""
    if "--" not in argv:
        raise SystemExit("usage: blender --background --python <this> -- <in> <out> <voxel>")
    args = argv[argv.index("--") + 1:]
    if len(args) != 3:
        raise SystemExit("usage: ... -- <input.glb> <output.glb> <voxel_size>")
    return args[0], args[1], float(args[2])


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


def voxel_remesh(obj, voxel_size):
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    modifier = obj.modifiers.new(name="VoxelRemesh", type="REMESH")
    modifier.mode = "VOXEL"
    modifier.voxel_size = voxel_size
    bpy.ops.object.modifier_apply(modifier=modifier.name)

    # Smooth shading, and this is not a cosmetic choice — it is the whole point
    # of the pre-pass.
    #
    # Blender's voxel remesh produces a flat-shaded mesh. The glTF exporter has
    # to give every flat face its own corner normals, so it SPLITS the shared
    # vertices back apart on the way out: the exported file arrives at
    # meshoptimizer as a soup of unwelded triangles, which is precisely the
    # condition it cannot collapse. Measured on asteroid_common_4: a remesh at
    # voxel 0.03 exported 27,496 triangles across 54,992 vertices, and
    # `simplify --ratio 0.1` then reduced it to 27,496 triangles — nothing at
    # all, the same stall the pre-pass exists to cure.
    #
    # Shading smooth lets the exporter share vertices again; merging by distance
    # first removes any duplicates the remesh itself left behind.
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.remove_doubles()
    bpy.ops.object.mode_set(mode="OBJECT")
    bpy.ops.object.shade_smooth()


def transfer_uvs(target, source):
    """Project the source mesh's UVs onto the remeshed one.

    A voxel remesh throws the UV layer away — it builds a new surface with no
    relationship to the old one's parameterisation — so a level generated from
    it renders untextured however large `texture_size` is. The material and its
    images survive on the object; only the coordinates that sample them are
    gone.

    Blender's Data Transfer modifier puts them back by taking, for each corner
    of the new mesh, the interpolated UV of the nearest face of the old one.
    That is approximate where the remesh moved the surface, and it smears across
    UV seams — which is affordable here and nowhere else: this runs for a FAR
    level, seen beyond the band where its detail is legible at all.

    Returns False when the source had no UVs to give (nothing to do).
    """
    if not source.data.uv_layers:
        return False
    if not target.data.uv_layers:
        target.data.uv_layers.new(name="UVMap")

    bpy.context.view_layer.objects.active = target
    bpy.ops.object.select_all(action="DESELECT")
    target.select_set(True)
    modifier = target.modifiers.new(name="UVTransfer", type="DATA_TRANSFER")
    modifier.object = source
    modifier.use_loop_data = True
    modifier.data_types_loops = {"UV"}
    modifier.loop_mapping = "POLYINTERP_NEAREST"
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    return True


def main():
    input_path, output_path, voxel_size = parse_args(sys.argv)

    clear_scene()
    bpy.ops.import_scene.gltf(filepath=input_path)

    objects = mesh_objects()
    if not objects:
        raise SystemExit(f"no mesh found in {input_path}")
    for obj in objects:
        # Keep the pre-remesh mesh alive just long enough to copy its UVs back
        # off it; the remesh is destructive and there is no other source for
        # them once the modifier is applied.
        original = obj.copy()
        original.data = obj.data.copy()
        bpy.context.collection.objects.link(original)
        try:
            voxel_remesh(obj, voxel_size)
            if not transfer_uvs(obj, original):
                print(f"[blender-voxel-remesh] {obj.name}: no UVs to carry over")
        finally:
            bpy.data.objects.remove(original, do_unlink=True)

    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.export_scene.gltf(
        filepath=output_path,
        export_format="GLB",
        use_selection=True,
    )
    print(f"[blender-voxel-remesh] {input_path} -> {output_path} (voxel {voxel_size})")


main()
