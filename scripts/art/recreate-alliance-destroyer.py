"""Author an editable hard-surface interpretation of PPAllianceDestroyer.png.

node scripts/art/destroyer-atlas.mjs
blender --background --factory-startup --python scripts/art/recreate-alliance-destroyer.py

Output is a review candidate in raw/, never a replacement for the live rig.
Blender coordinates: +Z up, -Y bow; exported glTF: +Y up, +Z bow.
"""
import bpy
import bmesh
import json
import math
from pathlib import Path
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'raw/models/PPAllianceDestroyer/recreated'
OUT.mkdir(parents=True, exist_ok=True)
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)

material = bpy.data.materials.new('Horizon | shared PBR atlas')
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
        # Bevels need a clean trim, not the entire panel sheet squeezed into 2 cm.
        face_tile = 8 if tile == 0 and p.area < .06 else tile
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
        mod.segments=3
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

def mirror_shape(poly,s):
    result=[(s*x,y) for x,y in poly]
    return result if s==1 else list(reversed(result))

# Wide, low, chamfered central hull; the exposed dark waist remains distinct.
hull=[(-1.4,-3.35),(1.4,-3.35),(1.88,-2.85),(2.08,-1.12),(1.95,.15),(1.32,2.50),(-1.32,2.50),(-1.95,.15),(-2.08,-1.12),(-1.88,-2.85)]
slab('01 | ventral armour',hull,-.49,-.29,1,inset=-.025,bevel=.06)
slab('02 | continuous recessed waist',hull,-.28,-.04,11,inset=.015,bevel=.045)
slab('03 | main bevelled hull',hull,-.02,.30,0,inset=.055,bevel=.055)
slab('04 | keel',[(x*.77,y*.90) for x,y in hull],-.65,-.48,2,inset=-.025,bevel=.035)

# Split layered prow plates keep the bow broad rather than a wedge or spear.
for s in (-1,1):
    p=[(.045,-3.08),(1.30,-3.08),(1.72,-2.66),(1.73,-1.64),(.54,-1.72),(.045,-2.08)]
    slab(f'Prow armour {s}',mirror_shape(p,s),.265,.355,0,bevel=.018)
    p=[(.82,-1.60),(1.78,-1.62),(1.83,-.52),(1.61,.60),(1.19,.68),(.90,-.30)]
    slab(f'Long shoulder plate {s}',mirror_shape(p,s),.27,.57,0,inset=.025,bevel=.025)
    p=[(.80,-1.42),(1.02,-1.40),(1.34,.08),(1.12,.29)]
    slab(f'Inset dorsal service strip {s}',mirror_shape(p,s),.573,.579,4,bevel=0)
    # Bow light apertures and thin blue waist segments, recessed into dark sockets.
    block(f'Bow socket {s}',(s*1.02,-3.29,-.105),(.63,.05,.155),12,.024)
    block(f'Bow blue band {s}',(s*1.02,-3.322,-.10),(.50,.016,.050),6,.009)
    corner=block(f'Swept bow light {s}',(s*1.647,-3.083,-.13),(.43,.018,.026),6,.004)
    corner.rotation_euler.z=s*math.radians(46)
    for k in range(4):
        block(f'Bow running light {s}.{k}',(s*(.86+k*.11),-3.331,.005),(.066,.014,.023),6,.003)
    block(f'Beam waist light {s}',(s*2.01,-1.19,-.17),(.018,.84,.033),6,.006)
    block(f'Lower bow rail {s}',(s*.91,-3.335,-.37),(1.27,.08,.065),3,.02)

# Broad wing outriggers and two gun trenches. Armour above and below the trench
# is separate geometry: the guns have an actual recess, not a painted black patch.
for s in (-1,1):
    wing=[(1.50,-.12),(2.02,-.40),(2.50,-1.06),(4.08,-1.00),(4.30,-.68),(4.19,1.17),(3.76,1.63),(1.55,1.42)]
    slab(f'Wing structure {s}',mirror_shape(wing,s),-.32,-.08,2,bevel=.06)
    slab(f'Wing waist {s}',mirror_shape(wing,s),-.075,.10,12,bevel=.035)
    # Outer pods stop outside the gun well; the inboard aft deck closes its back.
    pod=[(2.85,-1.30),(3.96,-1.30),(4.32,-.91),(4.20,1.18),(3.77,1.53),(2.92,1.31)]
    slab(f'Outboard pod armour {s}',mirror_shape(pod,s),.10,.39,0,inset=.045,bevel=.055)
    slab(f'Wing aft cap {s}',mirror_shape([(1.74,.34),(2.92,.06),(3.0,1.37),(1.63,1.63)],s),.11,.37,0,bevel=.035)
    slab(f'Gun well floor {s}',mirror_shape([(2.12,-1.63),(2.64,-1.63),(2.92,.27),(2.10,.32)],s),-.17,-.08,3,bevel=.025)
    for x in (2.13,2.66):
        block(f'Gun trench lip {s}.{x}',(s*x,-.91,-.04),(.10,1.39,.17),0,.025)
    block(f'Wing forward dark recess {s}',(s*3.53,-1.267,.11),(1.01,.12,.255),12,.06)
    block(f'Wing blue radiator {s}',(s*3.53,-1.34,.10),(.88,.025,.14),5,.028)
    # Radiator inset on pod upper deck.
    block(f'Dorsal pod radiator well {s}',(s*3.51,.39,.381),(1.01,.96,.025),2,.075)
    block(f'Dorsal pod radiator {s}',(s*3.51,.39,.398),(.80,.64,.016),5,.026)
    # Outer edge armour rail and side-light.
    block(f'Pod lateral blue band {s}',(s*4.227,.12,.025),(.023,.84,.045),6,.007)
    # Aft drive housing with an unmistakable exhaust at the stern.
    block(f'Wing drive housing {s}',(s*3.52,1.43,-.075),(1.04,.48,.53),1,.11)
    block(f'Wing exhaust cavity {s}',(s*3.52,1.68,-.07),(.84,.035,.34),12,.065)
    block(f'Wing exhaust {s}',(s*3.52,1.705,-.07),(.65,.018,.23),5,.04)

def gun(name,x,y,z,scale=1):
    cyl(name+' rotating plinth',(x,y,z),(x,y,z+.11*scale),.29*scale,2,16)
    block(name+' breech',(x,y-.04*scale,z+.24*scale),(.43*scale,.52*scale,.32*scale),3,.06*scale)
    for s in (-1,1):
        block(name+f' cheek {s}',(x+s*.24*scale,y,z+.24*scale),(.09*scale,.39*scale,.26*scale),0,.03*scale)
        a=(x+s*.115*scale,y-.19*scale,z+.25*scale)
        b=(a[0],y-1.07*scale,a[2])
        cyl(name+f' barrel {s}',a,b,.067*scale,3,10)
        for k in range(3):
            yy=y-(.35+k*.20)*scale
            cyl(name+f' barrel collar {s}.{k}',(a[0],yy,a[2]),(a[0],yy-.075*scale,a[2]),.087*scale,13,10)
        cyl(name+f' muzzle {s}',(a[0],b[1]+.03*scale,a[2]),(a[0],b[1]-.11*scale,a[2]),.094*scale,2,10)
        cyl(name+f' bore {s}',(a[0],b[1]-.112*scale,a[2]),(a[0],b[1]-.115*scale,a[2]),.059*scale,12,10)

for s in (-1,1):
    gun(f'Wing cannon {s}',s*2.40,-.20,.00,.84)

# Central dorsal weapon recess and paired guns.
slab('Dorsal cannon dark inset',[(-.53,-2.02),(.53,-2.02),(.58,-.60),(-.58,-.60)],.31,.33,2,bevel=.08)
gun('Dorsal cannon',0,-1.08,.345,.82)

# Tapered aft superstructure with wraparound dark glazing between armour tiers.
bridge=[(-.80,-.49),(.80,-.49),(1.15,.14),(1.03,1.52),(.76,1.90),(-.76,1.90),(-1.03,1.52),(-1.15,.14)]
slab('Bridge foundation',bridge,.33,.72,0,inset=.12,bevel=.04)
slab('Lower observation gallery',[(x*.91,y) for x,y in bridge],.70,.84,7,inset=.10,bevel=.03)
slab('Sloped command citadel',[(x*.94,y+.06) for x,y in bridge],.85,1.18,2,inset=.20,bevel=.035)
for s in (-1,1):
    # White flanks frame the darker centre of the citadel.
    poly=[(.58,-.37),(.90,-.12),(.91,1.48),(.68,1.62),(.52,.56)]
    slab(f'Citadel cheek {s}',mirror_shape(poly,s),.82,1.21,0,inset=.1,bevel=.035)
    block(f'Citadel side vent {s}',(s*.91,.86,.86),(.035,.63,.26),4,.018)
upper=[(-.72,.12),(.72,.12),(.86,.34),(.86,1.46),(.67,1.68),(-.67,1.68),(-.86,1.46),(-.86,.34)]
slab('Command bridge windows',upper,1.14,1.37,7,inset=.025,bevel=.035)
slab('Command bridge armoured roof',[(x*1.06,y) for x,y in upper],1.37,1.49,0,inset=.025,bevel=.04)
slab('Flag bridge sensor black band',[(-.29,.86),(.29,.86),(.34,1.36),(-.34,1.36)],1.49,1.59,7,bevel=.025)
block('Sensor roof',(0,1.12,1.61),(.71,.61,.075),2,.025)
for s in (-1,1):
    block(f'Roof sensor fairing {s}',(s*.54,1.15,1.51),(.15,.34,.10),2,.027)
    block(f'Roof sensor aperture {s}',(s*.54,.972,1.51),(.09,.013,.03),6,.003)
    block(f'Deck access hatch {s}',(s*1.40,-1.26,.578),(.21,.30,.012),1,.015)
for s in (-1,1):
    cyl(f'Mast foot {s}',(s*.67,1.32,1.48),(s*.67,1.32,1.63),.071,3,10)
    cyl(f'Tall communications mast {s}',(s*.67,1.32,1.61),(s*.69,1.39,2.20),.024,3,8,radius2=.007)
    cyl(f'Short antenna {s}',(s*.79,.85,1.47),(s*.79,.89,1.89),.014,3,6,radius2=.004)
    block(f'Roof running light {s}',(s*.48,.41,1.48),(.13,.06,.019),6,.004)

# Secondary dorsal hardware and stern engine bank.
for s in (-1,1):
    slab(f'Aft shoulder armour {s}',mirror_shape([(1.07,.53),(1.41,.35),(1.65,1.57),(1.29,2.21),(.96,1.87)],s),.29,.63,0,bevel=.04)
    block(f'Engine back {s}',(s*.66,2.43,-.03),(.85,.40,.53),3,.09)
    block(f'Main exhaust socket {s}',(s*.66,2.647,-.015),(.70,.025,.36),12,.055)
    block(f'Main engine emitter {s}',(s*.66,2.665,-.015),(.53,.015,.235),5,.04)
    block(f'Aft service radiator {s}',(s*.73,2.07,.34),(.54,.44,.032),4,.03)
    block(f'Ventral service hatch {s}',(s*.73,-.95,-.652),(.58,1.16,.017),3,.04)
    block(f'Ventral rail {s}',(s*1.22,-.85,-.57),(.10,2.60,.11),1,.025)

# Name plaques are real atlas decals on shallow vertical plates.
for s in (-1,1):
    plaque=block(f'HORIZON hull designation {s}',(s*1.983,-.72,.10),(.018,.65,.27),9,0)
    # Map the outward face horizontally along the ship, upright; mirrored on port.
    for p in plaque.data.polygons:
        if abs(p.normal.x)>.9:
            for k in p.loop_indices:
                v=plaque.data.vertices[plaque.data.loops[k].vertex_index].co
                u=.5+v.y/.65 * (-s)
                vv=.5+v.z/.27
                plaque.data.uv_layers.active.data[k].uv=((1+(8+240*u)/256)/4,1-(2+(248-240*vv)/256)/4)

# Lower the citadel and lengthen the bow as a coherent deformation, keeping
# guns, lights and armour together rather than leaving floating accessories.
for obj in parts:
    matrix=obj.matrix_world.copy()
    inverse=matrix.inverted()
    for vertex in obj.data.vertices:
        p=matrix@vertex.co
        if p.z>.55:p.z=.55+(p.z-.55)*.68
        if p.y<0:p.y*=1.12
        if abs(p.x)>2.1:
            p.x=math.copysign(2.1+(abs(p.x)-2.1)*.94,p.x)
            p.y+=.12
        vertex.co=inverse@p
    obj.data.update()
    bpy.context.view_layer.objects.active=obj
    normal=obj.modifiers.new('Area weighted surface normals','WEIGHTED_NORMAL')
    normal.keep_sharp=True
    bpy.ops.object.modifier_apply(modifier=normal.name)

# Save an editable master with separately named parts. Game GLB joins the parts
# and uses one atlas material, so object count does not become draw-call count.
scene=bpy.context.scene
scene.render.engine='CYCLES'
scene.cycles.samples=32
scene.cycles.use_denoising=True
scene.render.resolution_x=1500;scene.render.resolution_y=1100
scene.render.resolution_percentage=100
scene.world.color=(.19,.19,.19)
scene.view_settings.view_transform='AgX'
scene.render.image_settings.file_format='PNG'
scene.render.film_transparent=True

def point_at(obj,p):
    obj.rotation_euler=(Vector(p)-obj.location).to_track_quat('-Z','Y').to_euler()

def area(name,loc,power,size,color):
    bpy.ops.object.light_add(type='AREA',location=loc)
    l=bpy.context.object;l.name=name;l.data.energy=power;l.data.shape='DISK';l.data.size=size;l.data.color=color
    point_at(l,(0,0,0))

area('Studio | key',(-4,-7,10),1400,7,(.84,.92,1))
area('Studio | fill',(7,-1,5),700,6,(.65,.81,1))
area('Studio | rim',(0,7,7),1700,5,(1,.84,.68))
bpy.ops.object.camera_add(location=(10,-15,9))
camera=bpy.context.object;camera.name='Review camera';camera.data.type='ORTHO';camera.data.ortho_scale=11.5
point_at(camera,(0,-.1,.35));scene.camera=camera
for image in bpy.data.images:
    if image.source=='FILE':image.pack()
bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'alliance_destroyer_recreated.blend'))

bpy.ops.object.select_all(action='DESELECT')
for p in parts:p.select_set(True)
bpy.context.view_layer.objects.active=parts[0]
bpy.ops.object.join()
ship=bpy.context.object;ship.name='Alliance Destroyer | Horizon'
# Material slots left by join can be collapsed without altering per-face UVs.
ship.data.materials.clear();ship.data.materials.append(material)
for p in ship.data.polygons:p.material_index=0
# Blender's tangent calculation cannot process n-gons. Triangulate only the
# export copy, preserving the editable master, before generating MikkTSpace.
bm=bmesh.new();bm.from_mesh(ship.data)
bmesh.ops.triangulate(bm,faces=bm.faces[:])
bm.to_mesh(ship.data);bm.free();ship.data.update()
bpy.ops.export_scene.gltf(filepath=str(OUT/'alliance_destroyer_recreated.glb'),export_format='GLB',use_selection=True,export_cameras=False,export_lights=False,export_apply=True,export_yup=True,export_tangents=True)
ship.data.calc_loop_triangles()
stats={'triangles':len(ship.data.loop_triangles),'vertices_blender':len(ship.data.vertices),'materials':1,'texture_maps':{'base':2048,'emissive':512,'normal':1024,'orm':512},'glb_bytes':(OUT/'alliance_destroyer_recreated.glb').stat().st_size,'source_concept':'raw/models/PPAllianceDestroyer.png','parts_in_editable_master':len(parts),'coordinates':'glTF +Y up, +Z bow','integration':'review candidate; live model and rig unchanged'}
(OUT/'metrics.json').write_text(json.dumps(stats,indent=2)+'\n')
print('DESTROYER_METRICS',json.dumps(stats))

# Import the deliverable again: review images must show the exported GLB,
# including material conversion, UVs and tangents, rather than only the master.
bpy.data.objects.remove(ship,do_unlink=True)
bpy.ops.import_scene.gltf(filepath=str(OUT/'alliance_destroyer_recreated.glb'))
imported=[o for o in bpy.context.selected_objects if o.type=='MESH']
assert len(imported)==1, 'Expected one exported mesh'
views={'hero':((10,-15,9),11.4,(0,-.1,.38)),
       'aft':((-10,14,8),11.4,(0,-.1,.38)),
       'top':((0,0,18),10.0,(0,0,0)),
       'front':((0,-18,3.2),10.0,(0,0,.55)),
       'side':((18,0,3.5),9.0,(0,0,.55))}
for name,(loc,scale,target) in views.items():
    camera.location=loc;camera.data.ortho_scale=scale;point_at(camera,target)
    scene.render.filepath=str(OUT/f'preview_{name}.png')
    bpy.ops.render.render(write_still=True)
