"""Render the existing shipped model in the candidate's studio for comparison."""
from pathlib import Path
import bpy
from mathutils import Vector

root=Path(__file__).resolve().parents[2]
out=root/'raw/models/PPAllianceDestroyer/recreated'
bpy.ops.wm.open_mainfile(filepath=str(out/'alliance_destroyer_recreated.blend'))
for obj in list(bpy.context.scene.objects):
    if obj.type=='MESH':bpy.data.objects.remove(obj,do_unlink=True)
bpy.ops.import_scene.gltf(filepath=str(root/'assets/models/alliance_destroyer.glb'))
objects=[o for o in bpy.context.selected_objects if o.type=='MESH']
points=[o.matrix_world@Vector(corner) for o in objects for corner in o.bound_box]
lo=Vector(tuple(min(p[i] for p in points) for i in range(3)))
hi=Vector(tuple(max(p[i] for p in points) for i in range(3)))
centre=(lo+hi)/2
factor=8.64/(hi.x-lo.x)
# Bake the imported hierarchy, centre, and scale uniformly to the same width.
# No reshaping or repair of the existing model is performed.
for obj in objects:
    transform=obj.matrix_world.copy()
    for v in obj.data.vertices:
        v.co=(transform@v.co-centre)*factor+Vector((0,-.31,.26))
    obj.parent=None
    obj.matrix_world.identity()
scene=bpy.context.scene
camera=scene.camera
camera.location=(10,-15,9)
camera.data.ortho_scale=11.4
camera.rotation_euler=(Vector((0,-.1,.38))-camera.location).to_track_quat('-Z','Y').to_euler()
scene.render.filepath=str(out/'preview_existing.png')
bpy.ops.render.render(write_still=True)
