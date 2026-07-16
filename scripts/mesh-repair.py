import bpy
import sys

argv = sys.argv[sys.argv.index("--") + 1:]
input_path = argv[0]
output_path = argv[1]

bpy.ops.import_scene.gltf(filepath=input_path)

obj = bpy.context.view_layer.objects.active or bpy.context.selected_objects[0]
bpy.context.view_layer.objects.active = obj
bpy.ops.object.select_all(action="DESELECT")
obj.select_set(True)

bpy.ops.object.mode_set(mode="EDIT")
bpy.ops.mesh.select_all(action="SELECT")
bpy.ops.mesh.remove_doubles(threshold=0.0001)
bpy.ops.mesh.normals_make_consistent(inside=False)
bpy.ops.object.mode_set(mode="OBJECT")

bpy.ops.export_scene.gltf(
    filepath=output_path,
    export_format="GLB",
    export_image_format="AUTO",
)
