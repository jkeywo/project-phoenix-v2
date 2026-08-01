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
#   - Voxel size is in the model's own units. Too coarse melts detail; too fine
#     produces more triangles than you started with.
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


def main():
    input_path, output_path, voxel_size = parse_args(sys.argv)

    clear_scene()
    bpy.ops.import_scene.gltf(filepath=input_path)

    objects = mesh_objects()
    if not objects:
        raise SystemExit(f"no mesh found in {input_path}")
    for obj in objects:
        voxel_remesh(obj, voxel_size)

    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.export_scene.gltf(
        filepath=output_path,
        export_format="GLB",
        use_selection=True,
    )
    print(f"[blender-voxel-remesh] {input_path} -> {output_path} (voxel {voxel_size})")


main()
