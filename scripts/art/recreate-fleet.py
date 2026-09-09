"""Eight distinct concept interpretations; one material/primitive per export.

blender -b --factory-startup --python scripts/art/recreate-fleet.py -- alliance_cruiser
The lower-detail source is authored from the same dimensions, without tiny
fittings, so distant stations retain thin solar arrays and open ring structures.
"""
import os, sys, json, math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(Path(__file__).parent))
name=sys.argv[sys.argv.index('--')+1]
spec=next(s for s in json.loads((ROOT/'scripts/art/fleet.json').read_text()) if s['name']==name)
OUT=ROOT/'raw/models'/spec['concept']/'recreated'
os.environ['PHOENIX_ART_OUT']=str(OUT)
from fleet_geometry import bpy, bmesh, Vector, material, parts, finish, block, slab, cyl
COARSE=False

def shell(label, sections, tile=0, sides=24):
    """Longitudinal elliptical loft: (y, half width, centre z, half height)."""
    sides=(8 if COARSE==2 else 12) if COARSE else sides
    verts=[]
    for y,w,z,h in sections:
        for i in range(sides):
            a=2*math.pi*i/sides
            verts.append((w*math.cos(a),y,z+h*math.sin(a)))
    faces=[tuple(reversed(range(sides))),tuple(range((len(sections)-1)*sides,len(sections)*sides))]
    faces += [(j*sides+i,j*sides+(i+1)%sides,(j+1)*sides+(i+1)%sides,(j+1)*sides+i) for j in range(len(sections)-1) for i in range(sides)]
    m=bpy.data.meshes.new(label);m.from_pydata(verts,[],faces);m.update()
    o=bpy.data.objects.new(label,m);bpy.context.collection.objects.link(o)
    return finish(o,tile)

def move(obj,xyz):
    obj.location+=Vector(xyz)
    return obj

def disc(label,xy,z,r,depth,tile=0,r2=None):
    return cyl(label,(*xy,z-depth/2),(*xy,z+depth/2),r,tile,(12 if COARSE==2 else 24) if COARSE else 48,r2)

def ring(label,r,z,profile,tile=0,segments=96):
    """Lathed closed cross-section (radial offset, height offset)."""
    if COARSE:
        # The starbase still spans many pixels at its 100-unit transition.
        # Keep the annulus round there; tiny ship collars can use fewer sides.
        segments=(48 if COARSE==2 else 64) if name=='alliance_starbase' else (12 if COARSE==2 else 32)
    verts=[((r+dr)*math.cos(2*math.pi*i/segments),(r+dr)*math.sin(2*math.pi*i/segments),z+dz) for i in range(segments) for dr,dz in profile]
    n=len(profile)
    faces=[(i*n+j,((i+1)%segments)*n+j,((i+1)%segments)*n+(j+1)%n,i*n+(j+1)%n) for i in range(segments) for j in range(n)]
    m=bpy.data.meshes.new(label);m.from_pydata(verts,[],faces);m.update()
    o=bpy.data.objects.new(label,m);bpy.context.collection.objects.link(o)
    return finish(o,tile)

def ellipse(label,x,y,z,rx,ry,height,tile=0):
    o=disc(label,(0,0),z,1,height,tile)
    o.scale=(rx,ry,1);o.location.x=x;o.location.y=y
    bpy.context.view_layer.objects.active=o
    bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
    return o

def turret(x,y,z,heavy=False):
    r=.32 if heavy else .19
    disc('Turret rotation bearing',(x,y),z,r,.10,3)
    block('Armoured gun breech',(x,y,z+.15),(r*1.7,r*2,.19),1,.05)
    for dx in (-r*.37,r*.37):
        cyl('Gun barrel',(x+dx,y-.12,z+.17),(x+dx,y-(.8 if heavy else .48),z+.17),.038 if heavy else .025,3,8)

def bridge(y,z,w=1):
    ellipse('Command bridge glazing',0,y,z,w,.78*w,.20,7)
    ellipse('Rolled bridge roof',0,y+.04,z+.13,w*.97,.72*w,.16,0)
    ellipse('Bridge upper deck',0,y+.15,z+.24,w*.57,.44*w,.13,1)
    if not COARSE:
        for x,h in [(-.25,.48),(0,.8),(.24,.4)]:
            cyl('Communications aerial',(x*w,y+.30,z+.28),(x*w,y+.42,z+.28+h*w),.016,13,6,radius2=.003)
        block('Bridge blue sensor glass',(0,y-.36*w,z+.28),(.55*w,.17,.07),6,.01)

def pod(x,y,z,length,width=.44):
    sections=[(-length/2,.65*width,z,.18),(-length/2+.18,width,z,.27),(length*.2,width*.93,z,.24),(length/2,width*.32,z+.04,.12)]
    move(shell('Long nacelle armour',sections,0,20),(x,y,0))
    # Recessed cobalt aperture with a surrounding dark collar.
    for dy,r,tile in [(0,.81*width,2),(-.015,.64*width,5)]:
        o=cyl('Nacelle intake', (x,y-length/2-.01+dy,z),(x,y-length/2-.04+dy,z),r,tile,16)
        o.scale.z=.60
    block('Nacelle spine trim',(x,y+.1,z+.235),(width*.48,length*.66,.045),2,.01)
    if not COARSE:
        for sy in (-.25,.2):
            block('Nacelle service hatch',(x,y+length*sy,z+.265),(width*.65,.26,.025),11,.006)
    block('Nacelle aft radiator',(x,y+length*.37,z+.08),(width*.36,.38,.09),6,.02)

def alliance_ship(kind):
    if kind=='courier':
        sections=[(-4.8,.035,-.04,.025),(-3.6,.52,0,.16),(-1.8,.91,.08,.30),(.6,.94,.13,.30),(2.7,.56,.14,.23),(3.6,.09,.15,.07)]
        shell('Needle hull',sections,0,32)
        shell('Long dark canopy',[(-3.6,.12,.13,.04),(-2.3,.47,.32,.12),(-.8,.52,.37,.14),(.2,.29,.40,.07)],7,24)
        for s in (-1,1):
            slab('Swept courier pylon',[(s*.6,.5),(s*2.6,1.7),(s*2.8,2.5),(s*.7,1.9)],-.02,.15,0,.10)
            pod(s*2.8,1.5,.08,3.7,.34)
            block('Courier flank recess',(s*.75,-.5,.02),(.10,2.0,.12),2,.015)
        bridge(1.65,.48,.48)
        return
    heavy=kind=='battleship'
    length=6.0 if heavy else 4.8
    width=2.0 if heavy else 1.62
    sections=[(-length,.32,0,.10),(-length+.42,width*.77,0,.27),(-length+1.4,width,0,.38),(.5,width*.97,.04,.36),(length*.7,width*.76,.08,.30),(length,width*.2,.12,.14)]
    shell('Continuous rounded lower hull',sections,2,32)
    shell('Rolled upper hull',[(y,w*1.012,z+.19,h*.88) for y,w,z,h in sections],0,32)
    # A panoramic bow belt, exposed between the lower and upper armour.
    shell('Bow observation belt',[(y,w*1.019,z+.10,h*.46) for y,w,z,h in sections[:3]],7,32)
    deck=[(-width*.58,-length+.68),(width*.58,-length+.68),(width*.69,1.4),(width*.42,length*.74),(-width*.42,length*.74),(-width*.69,1.4)]
    slab('Inset graphite dorsal deck',deck,.565,.59,2,.04)
    for s in (-1,1):
        wing=[(s*width*.65,-.1),(s*(width+1.8),.8),(s*(width+1.9),2.0),(s*width*.62,2.5)]
        slab('Swept structural pylon',wing,-.03,.25,0,.10)
        pod(s*(width+1.85),.9,.18,8.1 if heavy else 7.8,.49 if heavy else .38)
        if heavy:
            pod(s*(width+1.23),1.0,-.36,6.3,.39)
        # Raised white longitudinal armour rails frame the dark central deck.
        move(shell('Raised shoulder armour',[(-length+.9,.32,.51,.12),(-2,.38,.51,.20),(1.8,.36,.56,.20),(length-.5,.15,.42,.12)],0,16),(s*width*.72,0,0))
        block('Blue lateral reactor strip',(s*width*.92,.7,-.01),(.05,2.3,.08),6,.006)
    if heavy:
        ellipse('Citadel lower deck',0,2.1,.71,1.25,1.45,.34,1)
        ellipse('Citadel armoured shoulder',0,2.15,.89,1.20,1.4,.22,0)
        ellipse('Citadel windows',0,2.28,1.07,.95,1.02,.18,7)
        ellipse('Citadel intermediate deck',0,2.34,1.18,1.04,1.13,.17,0)
        bridge(2.65,1.45,.85)
    else: bridge(2.1,.86,.91)
    if not COARSE:
        turret(0,-length+1.5,.59,heavy)
        turret(0,-.65,.60,heavy)
        for s in (-1,1):
            for y in ((-2.4,0,2.6) if heavy else (0.1,)):
                turret(s*(width+.02),y,.63,False)
            for y in (-2.7,-1.6,-.5,.6):
                block('Recessed maintenance bay',(s*.65,y,.61),(.32,.65,.023),11,.005)
        for y in (-2.1,0.4): block('Transverse blue power bus',(0,y,.62),(.91,.09,.035),6,.006)
    else: turret(0,-1,.59,heavy)

def radial_block(label,a,r,z,size,tile=0):
    if size[0]>1 and tile==0:
        w,h,d=size
        o=slab(label,[(-w/2,-h*.30),(-w*.39,-h/2),(w*.39,-h/2),(w/2,-h*.30),(w/2,h*.30),(w*.39,h/2),(-w*.39,h/2),(-w/2,h*.30)],z-d/2,z+d/2,tile,.10)
        o.location.x=r*math.cos(a);o.location.y=r*math.sin(a)
    else:o=block(label,(r*math.cos(a),r*math.sin(a),z),size,tile,.03)
    o.rotation_euler.z=a
    return o

def station(research=False):
    if not research:
        profile=[(-.48,-.18),(-.45,.20),(-.22,.38),(.24,.34),(.46,.08),(.43,-.27),(.12,-.40),(-.25,-.36)]
        ring('Continuous inhabited annulus',4.2,0,profile,0,128)
        ring('Panoramic annulus glazing',4.2,0,[(.449,-.15),(.47,-.12),(.47,.04),(.458,.07)],7,128)
        ring('Blue lower ring conduit',4.2,0,[(.32,-.39),(.37,-.35),(.32,-.31),(.27,-.35)],6,128)
        disc('Central lower reactor',(0,0),-.25,1.03,.65,2,.82)
        disc('Central habitation deck',(0,0),.04,1.36,.34,0,1.10)
        ring('Central windows',1.15,.19,[(-.05,-.06),(.08,-.06),(.08,.07),(-.05,.07)],7,64)
        disc('Spire socket',(0,0),.35,.85,.27,0,.62)
        for i in range(4):
            a=i*math.pi/2
            radial_block('Four swept ring spokes',a,2.58,-.02,(3.0,.48,.30),0)
            radial_block('Spoke recess',a,2.7,.155,(2.25,.24,.045),2)
            radial_block('Spoke light',a,3.5,.18,(.5,.12,.035),6)
            radial_block('Outboard docking arm',a,4.45,.26,(1.25,.49,.31),0)
            radial_block('Docking arm light',a,4.99,.26,(.07,.29,.11),5)
        # Tall four-sided taper with open blue channels along the buttresses.
        cyl('Spire dark core',(0,0,.36),(0,0,3.55),.37,2,12,.07)
        for i in range(4):
            a=i*math.pi/2
            x,y=math.cos(a),math.sin(a)
            cyl('Swept ivory spire fin',(x*.56,y*.56,.4),(x*.10,y*.10,3.55),.20,0,6,.025)
            cyl('Spire illuminated channel',(x*.39,y*.39,.67),(x*.09,y*.09,3.44),.045,6,6,.015)
        count=24 if not COARSE else 12
        for i in range(count):
            a=(i+.5)*2*math.pi/count
            radial_block('Radial finger berth',a,4.67,-.24,(.75,.16,.12),0)
            if not COARSE:
                radial_block('Berth approach light',a,4.96,-.16,(.18,.07,.028),6)
                radial_block('Ring equipment panel',a,4.2,.365,(.33,.27,.025),11)
        return
    # Research outpost: glazed dome, tall core, three satellite laboratories,
    # paired solar wings and an offset dish/petal sensor array.
    disc('Dome foundation',(0,0),-.12,1.80,.35,0,1.74)
    ring('Dome lower blue belt',1.72,.03,[(-.06,-.04),(.06,-.04),(.06,.045),(-.06,.045)],6,96)
    disc('Glazed research dome',(0,0),.42,1.72,.74,7,1.06)
    for z,r in [(.1,1.74),(.45,1.43),(.79,1.13)]:
        ring('Dome structural hoop',r,z,[(-.035,-.035),(.035,-.035),(.035,.035),(-.035,.035)],0,72)
    for i in range(12 if not COARSE else 8):
        a=i*2*math.pi/(12 if not COARSE else 8)
        cyl('Dome meridian rib',(1.69*math.cos(a),1.69*math.sin(a),.08),(1.10*math.cos(a),1.10*math.sin(a),.80),.025,0,6)
    disc('Dome crown',(0,0),.87,1.15,.17,0,.93)
    cyl('Research spire',(0,0,.90),(0,0,3.2),.33,0,8,.065)
    for s in (-1,1):
        cyl('Research spire light',(s*.20,-.20,1.0),(s*.035,-.06,3.1),.027,6,6)
        for y in (-.51,.51):
            block('Solar wing frame',(s*3.20,y,.15),(2.35,.84,.11),0,.03)
            block('Solar cell field',(s*3.20,y,.218),(2.18,.70,.015),12,0)
        block('Solar wing truss',(s*2.05,0,.08),(1.3,.20,.14),13,.01)
    for i in range(3):
        a=-math.pi/2+i*2*math.pi/3
        x,y=2.45*math.cos(a),2.45*math.sin(a)
        radial_block('Laboratory connecting corridor',a,1.91,-.03,(1.15,.35,.25),0)
        disc('Satellite laboratory',(x,y),.06,.58,.34,7,.56)
        disc('Laboratory roof',(x,y),.28,.67,.17,0,.46)
        cyl('Laboratory antenna',(x,y,.37),(x,y,1.5 if i else 1.14),.11,0,10,.035)
        if not COARSE:
            cyl('Laboratory aerial',(x,y,1.12),(x,y,1.80),.01,13,6)
            for k in (-1,0,1):
                radial_block('Lab docking finger',a+k*.12,3.08,-.08,(.48,.13,.10),0)
    # Six-lobed instrument crown, mounted on the aft laboratory.
    x,y=1.9,1.9
    cyl('Instrument mast',(x,y,.1),(x,y,1.85),.16,0,12,.1)
    disc('Instrument mast collar',(x,y),.82,.28,.12,0)
    for i in range(6):
        a=i*math.pi/3
        petal=block('Sensor array petal',(x+.63*math.cos(a),y,2.2+.63*math.sin(a)),(.60,.10,.60),14,.06)
        petal.rotation_euler.y=-a
        cyl('Sensor petal support',(x,y,2.2),(x+.5*math.cos(a),y,2.2+.5*math.sin(a)),.035,6,6)
    cyl('Sensor hub',(x,y-.11,2.2),(x,y+.11,2.2),.20,3,24)
    # Offset communications dish, a true shallow bowl with radial ribs.
    dish=ring('Communications dish',.40,0,[(-.35,-.10),(-.15,-.07),(.15,.05),(.20,.12),(.17,.15),(-.15,-.03),(-.35,-.06)],0,48)
    dish.rotation_euler.x=math.radians(65);dish.location=(-1.9,1.9,1.1)
    cyl('Dish mounting mast',(-1.9,1.9,.1),(-1.9,1.9,1.1),.075,13,8)

def blade(label,points,z,depth,tile=0):
    # Sharper planform than Alliance rolled slabs, with a raised central ridge.
    n=len(points);cx=sum(p[0] for p in points)/n;cy=sum(p[1] for p in points)/n
    verts=[(x,y,z) for x,y in points]+[(x*.98+cx*.02,y*.98+cy*.02,z+.045) for x,y in points]+[(cx,cy,z+depth)]
    faces=[tuple(reversed(range(n)))]+[(i,(i+1)%n,n+(i+1)%n,n+i) for i in range(n)]+[(n+i,n+(i+1)%n,2*n) for i in range(n)]
    m=bpy.data.meshes.new(label);m.from_pydata(verts,[],faces);m.update()
    o=bpy.data.objects.new(label,m);bpy.context.collection.objects.link(o)
    return finish(o,tile)

def dynasty_ship(kind):
    heavy=kind=='battleship';small=kind=='courier'
    length=5.5 if heavy else (4.3 if small else 4.8)
    width=1.2 if heavy else (.58 if small else .85)
    shell('Exposed mechanical keel',[(-length,.035,0,.03),(-length*.60,width*.7,.06,.22),(-.3,width,.12,.40),(2.4,width*.75,.18,.34),(3.2,width*.18,.23,.12)],2,20)
    for s in (-1,1):
        # Split lance tips and overlapping dorsal armour, leaving glowing gaps.
        for j in range(5 if not COARSE else 3):
            count=5 if not COARSE else 3
            y=-length+.25+j*(length+2.1)/count
            w=width*(.32+.62*min(1,(j+1)/2))
            blade('Overlapping dorsal carapace',[(s*.04,y-.46),(s*w,y-.15),(s*w*1.12,y+.87),(s*.08,y+1.17)],.25+j*.027,.23,0 if j%3 else 14)
        # Cresent wings curl forward to hooked tips, with burgundy inner sails.
        wing=[(s*.65,.5),(s*1.75,.7),(s*3.85,.30),(s*4.65,-.3),(s*4.22,1.50),(s*3.45,2.80),(s*1.20,2.3)]
        blade('Swept burgundy sail',wing,-.01,.28,1)
        edge=[(s*3.12,.40),(s*4.65,-.3),(s*4.40,-1.60),(s*4.92,.10),(s*4.42,1.92),(s*3.50,2.95),(s*3.0,2.4),(s*3.90,1.6)]
        blade('Hooked outer wing talon',edge,.06,.32,0)
        blade('Forward split jaw',[(s*.10,-length-.3),(s*.30,-length+1.5),(s*.86,-length+2.2),(s*.68,-length+.4)],-.05,.25,0)
        cyl('Jaw reactor slit',(s*.22,-length+.7,.13),(s*.38,-length+1.75,.21),.055,6,8)
        engine_x=s*(.82 if small else 1.20)
        cyl('Engine hot inner core',(engine_x,.65,.05),(engine_x,3.40,.24),.26,5,16,.18)
        # Segmented barrel ribs and ribs around the exposed flank.
        ribs=10 if not COARSE else (2 if COARSE==2 else 4)
        for j in range(ribs):
            y=-length*.62+j*(length*.62+2.4)/ribs
            x=s*width*(.78 if y<-.6 else .98)
            cyl('Exposed flank rib',(x,y,-.17),(x+s*.15,y,.36),.027 if not COARSE else .04,13,6)
        collars=7 if not COARSE else (1 if COARSE==2 else 3)
        for j in range(collars):
            y=1.0+j*(2.1/collars)
            o=ring('Engine rib collar',.29,0,[(-.015,-.025),(.02,-.025),(.02,.025),(-.015,.025)],13,24)
            o.rotation_euler.x=math.pi/2;o.location=(engine_x,y,.15)
        # Raised scythe fins swept back above the main silhouette.
        for x,y,h in [(s*1.0,1.8,.9),(s*3.72,1.9,.6)]:
            o=blade('Swept dorsal blade',[(0,0),(.15,-.50),(.20,1.3),(-.05,.82)],0,.08,0)
            for v in o.data.vertices:
                px,py,pz=v.co;v.co=(x+pz,y+py,.2+px+h*(py+.5)/1.8)
        if not COARSE:
            for y in (.55,1.35,2.1):
                turret(s*2.9,y,.19,False)
            for y in (-2.5,-1.3,.1):
                cyl('Flank red reactor conduit',(s*width*.84,y,.01),(s*width*.91,y+.45,.04),.04,6,8)
    if not small:
        # Tall aft armoured command tower, a defining Cruiser/Battleship feature.
        for j in range(4 if heavy else 3):
            z=.54+j*.33;w=.76-j*.11;y=1.45+j*.22
            blade('Citadel overlapping armour',[(-w,y-.65),(w,y-.65),(w*.85,y+.8),(0,y+1.25),(-w*.85,y+.8)],z,.29,0)
            block('Citadel red slit',(0,y-.65,z+.05),(w*1.35,.05,.07),6,.006)
        for s in (-1,1):
            cyl('Citadel spire',(s*.36,2.0,1.15),(s*.29,3.65,2.38 if heavy else 1.91),.15,0,5,.007)
    if heavy:
        for s in (-1,1):
            blade('Broad secondary fore armour',[(s*.7,-4.0),(s*1.35,-3.65),(s*1.9,.5),(s*.96,1.8)],.2,.48,0)
            if not COARSE: turret(s*1.70,-.65,.58,True)

def author():
    if name=='alliance_starbase':station(False)
    elif name=='alliance_research_outpost':station(True)
    elif name.startswith('alliance_'):alliance_ship(name.split('_')[1])
    else:dynasty_ship(name.split('_')[1])

def export(suffix):
    bpy.ops.object.select_all(action='DESELECT')
    for p in parts:p.select_set(True)
    bpy.context.view_layer.objects.active=parts[0]
    bpy.ops.object.join();ship=bpy.context.object;ship.name=name+suffix
    ship.data.materials.clear();ship.data.materials.append(material)
    for p in ship.data.polygons:p.material_index=0
    bm=bmesh.new();bm.from_mesh(ship.data)
    bmesh.ops.triangulate(bm,faces=bm.faces[:]);bm.to_mesh(ship.data);bm.free()
    ship.data.update()
    bpy.ops.export_scene.gltf(filepath=str(OUT/f'{name}{suffix}.glb'),export_format='GLB',use_selection=True,export_cameras=False,export_lights=False,export_apply=True,export_yup=True,export_tangents=True)
    ship.data.calc_loop_triangles()
    return ship,len(ship.data.loop_triangles)

author()
for obj in parts:
    bpy.context.view_layer.objects.active=obj
    mod=obj.modifiers.new('Area weighted armour normals','WEIGHTED_NORMAL');mod.keep_sharp=True
    bpy.ops.object.modifier_apply(modifier=mod.name)
for img in bpy.data.images:
    if img.source=='FILE':img.pack()
bpy.ops.wm.save_as_mainfile(filepath=str(OUT/f'{name}_recreated.blend'))
part_count=len(parts)
ship,triangles=export('_recreated')
bpy.data.objects.remove(ship,do_unlink=True);parts.clear()
COARSE=True;author()
ship,coarse_triangles=export('_lod_source')
bpy.data.objects.remove(ship,do_unlink=True);parts.clear()
COARSE=2;author()
ship,far_triangles=export('_far_source')
bpy.data.objects.remove(ship,do_unlink=True);parts.clear()
(OUT/'metrics.json').write_text(json.dumps({'model':name,'concept':spec['concept']+'.png','triangles':triangles,'coarse_triangles':coarse_triangles,'far_triangles':far_triangles,'parts':part_count},indent=2)+'\n')
# Review a fresh import, so exported normals/materials are actually inspected.
bpy.ops.import_scene.gltf(filepath=str(OUT/f'{name}_recreated.glb'))
scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=24
scene.cycles.use_denoising=True
scene.render.resolution_x=1200;scene.render.resolution_y=900;scene.render.resolution_percentage=100
scene.render.image_settings.file_format='PNG';scene.render.film_transparent=True
scene.world.color=(.21,.21,.21);scene.view_settings.view_transform='AgX'
def aim(o,p):o.rotation_euler=(Vector(p)-o.location).to_track_quat('-Z','Y').to_euler()
for loc,power,size in [((-5,-6,10),1700,8),((7,-2,6),1100,7),((0,7,8),2100,6)]:
    bpy.ops.object.light_add(type='AREA',location=loc);o=bpy.context.object;o.data.energy=power;o.data.shape='DISK';o.data.size=size;aim(o,(0,0,0))
bpy.ops.object.camera_add(location=(10,-15,10));camera=bpy.context.object;camera.data.type='ORTHO';scene.camera=camera
for view,loc in [('hero',(10,-15,10)),('top',(0,0,20))]:
    camera.location=loc;camera.data.ortho_scale=(17 if 'battleship' in name else 15) if view=='top' else (15 if 'battleship' in name else 12.4)
    aim(camera,(0,-.35,.4));scene.render.filepath=str(OUT/f'preview_{view}.png');bpy.ops.render.render(write_still=True)
print('FLEET_COMPLETE',name,triangles,coarse_triangles)
