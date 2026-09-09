"""Shared mesh and atlas primitives for the fleet recreation recipe."""
import bpy
import bmesh
import json
import math
from pathlib import Path
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
import os
OUT = Path(os.environ['PHOENIX_ART_OUT'])
OUT.mkdir(parents=True, exist_ok=True)
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)

material = bpy.data.materials.new('Fleet | shared PBR atlas')
material.use_nodes = True
material.use_backface_culling = True
nodes = material.node_tree.nodes
links = material.node_tree.links
bsdf = nodes.get('Principled BSDF')
for name, socket in [('base','Base Color'),('emissive','Emission Color'),('normal',None),('orm',None)]:
    node = nodes.new('ShaderNodeTexImage')
    node.image = bpy.data.images.load(str(OUT / f'destroyer_{name}.png'))
    node.label = name
    node.location = (-650, 350-['base','emissive','normal','orm'].index(name)*250)
    if name in ('normal','orm'):
        node.image.colorspace_settings.name = 'Non-Color'
    if socket:
        links.new(node.outputs['Color'], bsdf.inputs[socket])
    elif name == 'normal':
        nm = nodes.new('ShaderNodeNormalMap')
        links.new(node.outputs['Color'], nm.inputs['Color'])
        links.new(nm.outputs['Normal'], bsdf.inputs['Normal'])
        nm.inputs['Strength'].default_value = 0.45
    else:
        sep = nodes.new('ShaderNodeSeparateColor')
        links.new(node.outputs['Color'], sep.inputs['Color'])
        links.new(sep.outputs['Green'], bsdf.inputs['Roughness'])
        links.new(sep.outputs['Blue'], bsdf.inputs['Metallic'])
bsdf.inputs['Emission Strength'].default_value = 2.2

parts = []

def atlas_uv(obj, tile):
    """Project each hard face into a padded atlas tile, avoiding texture bleed."""
    mesh = obj.data
    mesh.update()
    uv = mesh.uv_layers.new(name='UVMap') if not mesh.uv_layers else mesh.uv_layers.active
    for p in mesh.polygons:
        coords = [mesh.vertices[mesh.loops[k].vertex_index].co for k in p.loop_indices]
        # Dominant-normal projection keeps bevels valid without a UV unwrap operator.
        axis = max(range(3), key=lambda a: abs(p.normal[a]))
        a, b = [q for q in range(3) if q != axis]
        lo = [min(c[q] for c in coords) for q in (a,b)]
        hi = [max(c[q] for c in coords) for q in (a,b)]
        if tile in (0,1,2,8,14):
            lo=[min(v.co[q] for v in mesh.vertices) for q in (a,b)]
            hi=[max(v.co[q] for v in mesh.vertices) for q in (a,b)]
        # Small faces on a finely sampled hull still belong to its panel sheet.
        # Area alone cannot distinguish a bevel from a curved armour surface.
        legacy_battleship = OUT.parent.name == 'PPAllianceBattleship'
        face_tile = 8 if legacy_battleship and tile == 0 and p.area < .06 else tile
        for k,c in zip(p.loop_indices,coords):
            u=(c[a]-lo[0])/max(hi[0]-lo[0],1e-8)
            v=(c[b]-lo[1])/max(hi[1]-lo[1],1e-8)
            # Atlas image origin is top-left; Blender texture V starts at bottom.
            uv.data[k].uv=((face_tile%4+(8+u*240)/256)/4,
                           1-(face_tile//4+(248-v*240)/256)/4)

def finish(obj, tile, bevel=0):
    bpy.context.view_layer.objects.active = obj
    if bevel:
        mod=obj.modifiers.new('Machined edges','BEVEL')
        # Oversized bevels on shallow inserts collapse opposite edge rings;
        # preserve a face between them so exported tangent frames stay valid.
        coords=[v.co for v in obj.data.vertices]
        thickness=min(max(v[i] for v in coords)-min(v[i] for v in coords) for i in range(3))
        mod.width=min(bevel,thickness*.35)
        mod.segments=2
        mod.affect='EDGES'
        bpy.ops.object.modifier_apply(modifier=mod.name)
    bm=bmesh.new(); bm.from_mesh(obj.data)
    bmesh.ops.dissolve_degenerate(bm,dist=1e-7,edges=bm.edges[:])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(obj.data); bm.free()
    obj.data.materials.append(material)
    atlas_uv(obj,tile)
    for polygon in obj.data.polygons:
        polygon.use_smooth=True
    obj.data.set_sharp_from_angle(angle=math.radians(38))
    parts.append(obj)
    return obj

def block(name, loc, size, tile=0, bevel=.04):
    bpy.ops.mesh.primitive_cube_add(size=1, location=loc)
    o=bpy.context.object; o.name=name
    o.scale=size
    bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
    return finish(o,tile,bevel)

def slab(name, outline, bottom, top, tile=0, inset=.0, bevel=.02):
    """Loft a convex planar outline, with optional sloping upper armour sides."""
    # Round plan-view corners before lofting: this changes the silhouette,
    # rather than merely shading the edges of an otherwise rectangular slab.
    rounded=[]
    for i,p in enumerate(outline):
        prev=outline[i-1];nxt=outline[(i+1)%len(outline)]
        a=Vector(p).lerp(Vector(prev),.19)
        b=Vector(p).lerp(Vector(nxt),.19)
        for t in (0,.33,.67,1):
            q=(1-t)**2*a+2*(1-t)*t*Vector(p)+t*t*b
            rounded.append(tuple(q))
    outline=rounded
    n=len(outline)
    cx=sum(p[0] for p in outline)/n; cy=sum(p[1] for p in outline)/n
    # Armour has a rolled shoulder and tucked lower lip instead of vertical walls.
    sculpted=tile in (0,1,2) and top-bottom>.08
    rings=[(bottom,.035), (bottom+(top-bottom)*.28,0),
           (bottom+(top-bottom)*.72,inset*.45), (top,inset+.045)] if sculpted else [(bottom,0),(top,inset)]
    verts=[(x+(cx-x)*shrink,y+(cy-y)*shrink,z) for z,shrink in rings for x,y in outline]
    faces=[tuple(reversed(range(n))),tuple(range((len(rings)-1)*n,len(rings)*n))]
    faces += [(r*n+i,r*n+(i+1)%n,(r+1)*n+(i+1)%n,(r+1)*n+i) for r in range(len(rings)-1) for i in range(n)]
    m=bpy.data.meshes.new(name);m.from_pydata(verts,[],faces);m.update()
    o=bpy.data.objects.new(name,m);bpy.context.collection.objects.link(o)
    return finish(o,tile,0)

def cyl(name, a, b, radius, tile=3, vertices=12, radius2=None):
    mid=(Vector(a)+Vector(b))*.5; d=Vector(b)-Vector(a)
    bpy.ops.mesh.primitive_cone_add(vertices=vertices,radius1=radius,radius2=radius if radius2 is None else radius2,depth=d.length,location=mid)
    o=bpy.context.object;o.name=name
    o.rotation_euler=d.to_track_quat('Z','Y').to_euler()
    bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
    return finish(o,tile)
