"""Concept interpretations; one material/primitive per export.

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
from fleet_geometry import bpy, bmesh, Vector, material, parts, finish, block, slab, cyl, atlas_uv
COARSE=False

def fair_sections(sections, steps):
    """Shape-preserving cubic loft stations; no width/height overshoot."""
    out=[]
    for i in range(len(sections)-1):
        a,b=sections[i:i+2]
        for j in range(steps):
            t=j/steps;row=[a[0]+(b[0]-a[0])*t]
            for k in range(1,len(a)):
                d=b[k]-a[k]
                prev=a[k]-sections[max(0,i-1)][k]
                nxt=sections[min(len(sections)-1,i+2)][k]-b[k]
                m0=0 if prev*d<=0 else math.copysign(min(abs(d),abs(prev)),d)
                m1=0 if nxt*d<=0 else math.copysign(min(abs(d),abs(nxt)),d)
                row.append((2*t**3-3*t*t+1)*a[k]+(t**3-2*t*t+t)*m0+(-2*t**3+3*t*t)*b[k]+(t**3-t*t)*m1)
            out.append(row)
    return out+[sections[-1]]

def shell(label, sections, tile=0, sides=24):
    """Longitudinal elliptical loft: (y, half width, centre z, half height)."""
    sides=(8 if COARSE==2 else 12) if COARSE else sides
    if name!='alliance_battleship':
        sections=fair_sections(sections,2 if COARSE else 4)
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
        # Scale the aperture about its centre, not the ship's origin.
    block('Nacelle spine trim',(x,y+.1,z+.235),(width*.48,length*.66,.045),2,.01)
    if not COARSE:
        for sy in (-.25,.2):
            block('Nacelle service hatch',(x,y+length*sy,z+.265),(width*.65,.26,.025),11,.006)
    block('Nacelle aft radiator',(x,y+length*.37,z+.08),(width*.36,.38,.09),6,.02)

def spinal_laser():
    # Fixed longitudinal weapon embedded in the forward hull, under two armour
    # shoulders. The muzzle shares the same straight axis as the power chamber.
    block('Recessed spinal weapon channel',(0,-2.6,.61),(.53,6.9,.15),2,.025)
    cyl('Spinal laser barrel',(0,.75,.68),(0,-6.02,.68),.14,3,16)
    for s in (-1,1):
        move(shell('Integrated laser armour shoulder',[(-6.08,.11,.45,.24),(-5.6,.22,.48,.33),(-.4,.27,.53,.34),(.95,.08,.51,.20)],0,20),(s*.39,0,0))
        block('Spinal cooling bus',(s*.27,-2.55,.77),(.07,5.5,.045),6,.009)
    # A hollow bore, with a recessed blue focusing lens rather than a gun turret.
    muzzle=ring('Armoured spinal muzzle',.195,0,[(-.040,-.08),(.065,-.08),(.085,.04),(.045,.12),(-.04,.12)],0,32)
    muzzle.rotation_euler.x=math.pi/2;muzzle.location=(0,-6.04,.68)
    cyl('Recessed laser focusing lens',(0,-6.01,.68),(0,-6.07,.68),.147,5,24)
    for y in ((-4.8,-3.8,-2.8,-1.8,-.8,.15) if not COARSE else (-4.6,-2.3,0)):
        collar=ring('Spinal field coil',.15,0,[(-.02,-.06),(.035,-.06),(.035,.06),(-.02,.06)],13,16)
        collar.rotation_euler.x=math.pi/2;collar.location=(0,y,.68)
        if not COARSE:
            for s in (-1,1):block('Spinal heat exchanger',(s*.75,y,.62),(.29,.40,.055),11,.008)

def alliance_ship(kind):
    if kind=='courier':
        sections=[(-4.8,.025,-.04,.025),(-3.6,.48,0,.17),(-1.8,.86,.02,.32),(.3,1.02,-.08,.51),(1.65,.85,-.16,.62),(2.7,.56,-.04,.41),(3.6,.07,.12,.07)]
        shell('Needle hull',sections,0,32)
        shell('Long dark canopy',[(-3.6,.12,.13,.04),(-2.3,.47,.32,.12),(-.8,.52,.37,.14),(.2,.29,.40,.07)],7,24)
        for s in (-1,1):
            slab('Blended fighter wing root',[(s*.35,-1.55),(s*.91,-.55),(s*2.60,1.1),(s*2.94,2.55),(s*1.9,2.3),(s*.42,2.6)],-.14,.22,0,.12)
            move(shell('Courier root fairing',[(-1.5,.05,.02,.05),(-.4,.26,.06,.23),(1.4,.34,.02,.30),(2.7,.06,.05,.06)],0,20),(s*.77,0,0))
            pod(s*2.8,1.5,.08,3.7,.34)
            block('Courier flank recess',(s*.75,-.5,.02),(.10,2.0,.12),2,.015)
        bridge(1.65,.48,.48)
        return
    heavy=kind=='battleship'
    length=6.0 if heavy else 4.8
    width=2.0 if heavy else 1.62
    sections=[(-length,.32,0,.10),(-length+.42,width*.77,0,.27),(-length+1.4,width,0,.38),(.5,width*.97,.04,.36),(length*.7,width*.76,.08,.30),(length,width*.2,.12,.14)]
    if not heavy:
        # Elliptical spoon bow, broad across its shoulders and round at the lip.
        sections=[(-length,.035,0,.035),(-length+.12,.55,0,.14),(-length+.42,1.08,0,.25),(-length+.90,1.43,0,.34),(-length+1.5,width,0,.38),(.5,width*.97,.04,.36),(3.45,1.17,.08,.29),(4.55,.48,.12,.18),(length,.04,.12,.04)]
    shell('Continuous rounded lower hull',sections,2,32)
    shell('Rolled upper hull',[(y,w*1.012,z+.19,h*.88) for y,w,z,h in sections],0,32)
    # A panoramic bow belt, exposed between the lower and upper armour.
    shell('Bow observation belt',[(y,w*1.019,z+.10,h*.46) for y,w,z,h in sections[:(3 if heavy else 5)]],7,32)
    deck=[(-width*.58,-length+.68),(width*.58,-length+.68),(width*.69,1.4),(width*.42,length*.74),(-width*.42,length*.74),(-width*.69,1.4)]
    dorsal=slab('Inset graphite dorsal deck',deck,.565,.59,2,.04)
    def roof_at(x,y):
        loft=fair_sections(sections,2 if COARSE else 4)
        a,b=next((a,b) for a,b in zip(loft,loft[1:]) if a[0]<=y<=b[0])
        t=(y-a[0])/(b[0]-a[0]);w=a[1]+(b[1]-a[1])*t
        centre=a[2]+(b[2]-a[2])*t;h=a[3]+(b[3]-a[3])*t
        return centre+.19+h*.88*math.sqrt(max(0,1-(x/(w*1.012))**2))
    if not heavy:
        # A tessellated cap follows the roof across its interior as well as its
        # boundary. A single ngon would be buried by the convex white hull.
        parts.remove(dorsal);bpy.data.objects.remove(dorsal,do_unlink=True)
        stations=fair_sections([(-4.0,.035),(-3.6,.70),(-2.7,.92),(1.4,.94),(2.8,.64),(3.5,.035)],2 if COARSE else 4)
        cols=5 if COARSE else 9;verts=[]
        for dz in (.014,-.02):
            for y,w in stations:
                for j in range(cols):
                    x=w*(2*j/(cols-1)-1);verts.append((x,y,roof_at(x,y)+dz))
        rows=len(stations);n=rows*cols;faces=[]
        for r in range(rows-1):
            for c in range(cols-1):
                a=r*cols+c;faces.extend([(a,a+1,a+cols+1,a+cols),(n+a+cols,n+a+cols+1,n+a+1,n+a)])
        boundary=list(range(cols))+[r*cols+cols-1 for r in range(1,rows)]+list(range(n-2,n-cols-1,-1))+[r*cols for r in range(rows-2,0,-1)]
        faces += [(a,b,n+b,n+a) for a,b in zip(boundary,boundary[1:]+boundary[:1])]
        m=bpy.data.meshes.new('Curved dorsal inset');m.from_pydata(verts,[],faces);m.update()
        o=bpy.data.objects.new('Curved graphite dorsal inset',m);bpy.context.collection.objects.link(o);finish(o,2)
    for s in (-1,1):
        wing=[(s*width*.65,-.1),(s*(width+1.8),.8),(s*(width+1.9),2.0),(s*width*.62,2.5)]
        pylon=slab('Swept structural pylon',wing,-.03,.25,0,.10)
        if not heavy:
            for v in pylon.data.vertices:
                v.co.z+=max(0,min(1,(abs(v.co.x)-width*.65)/2.0))*.30
        pod(s*(width+1.85),.9,.18 if heavy else .48,8.1 if heavy else 7.8,.49 if heavy else .38)
        if heavy:
            pod(s*(width+1.23),1.0,-.36,6.3,.39)
        # Raised white longitudinal armour rails frame the dark central deck.
        shoulder=[(-length+.9,.32,.51,.12),(-2,.38,.51,.20),(1.8,.36,.56,.20),(length-.5,.15,.42,.12)] if heavy else [(-4.15,.018,.27,.018),(-3.65,.21,.40,.12),(-2,.38,.51,.20),(1.8,.36,.56,.20),(2.85,.13,.34,.11),(3.3,.012,.20,.018)]
        move(shell('Raised shoulder armour',shoulder,0,16 if heavy else 20),(s*width*.72,0,0))
        block('Blue lateral reactor strip',(s*width*.92,.7,-.01),(.05,2.3,.08),6,.006)
    if heavy:
        ellipse('Citadel lower deck',0,2.1,.71,1.25,1.45,.34,1)
        ellipse('Citadel armoured shoulder',0,2.15,.89,1.20,1.4,.22,0)
        ellipse('Citadel windows',0,2.28,1.07,.95,1.02,.18,7)
        ellipse('Citadel intermediate deck',0,2.34,1.18,1.04,1.13,.17,0)
        bridge(2.65,1.45,.85)
        spinal_laser()
    else:
        ellipse('Bridge hull socket',0,2.1,.49,.94,.78,.30,0)
        bridge(2.1,.72,.91)
    if not COARSE:
        for s in (-1,1):
            for y in (-2.7,-1.6,-.5,.6):
                if not heavy:block('Recessed maintenance bay',(s*.65,y,roof_at(s*.65,y)+.012),(.32,.65,.023),11,.005)
        if not heavy:
            for y in (-2.1,0.4): block('Transverse blue power bus',(0,y,roof_at(0,y)+.019),(.91,.09,.035),6,.006)

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
        profile=[(.48*math.cos(a),.39*math.sin(a)) for a in [i*2*math.pi/16 for i in range(16)]]
        ring('Continuous inhabited annulus',4.2,0,profile,0,128)
        ring('Panoramic annulus glazing',4.2,0,[(.465,-.15),(.485,-.10),(.49,.04),(.48,.07)],7,128)
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
    # Three solar spokes alternate with three radial laboratory/dish spokes.
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
    for i in range(3):
        a=i*2*math.pi/3
        radial_block('Solar radial truss',a,2.05,.08,(1.3,.20,.14),13)
        for offset in (-.51,.51):
            for label,size,z,tile in [('Solar wing frame',(2.35,.84,.11),.15,0),('Solar cell field',(2.18,.70,.015),.218,12)]:
                o=radial_block(label,a,3.20,z,size,tile)
                o.location+=Vector((-math.sin(a)*offset,math.cos(a)*offset,0))
    for i in range(3):
        a=math.pi/3+i*2*math.pi/3
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
    array_angle=math.pi/3;x,y=2.45*math.cos(array_angle),2.45*math.sin(array_angle)
    first=len(parts)
    cyl('Instrument mast',(x,y,.1),(x,y,1.85),.16,0,12,.1)
    disc('Instrument mast collar',(x,y),.82,.28,.12,0)
    for i in range(6):
        a=i*math.pi/3
        petal=block('Sensor array petal',(x+.63*math.cos(a),y,2.2+.63*math.sin(a)),(.60,.10,.60),14,.06)
        petal.rotation_euler.y=-a
        cyl('Sensor petal support',(x,y,2.2),(x+.5*math.cos(a),y,2.2+.5*math.sin(a)),.035,6,6)
    cyl('Sensor hub',(x,y-.11,2.2),(x,y+.11,2.2),.20,3,24)
    rotate_array(parts[first:],(x,y),array_angle+math.pi/2)
    # Offset communications dish, a true shallow bowl with radial ribs.
    dish=ring('Communications dish',.40,0,[(-.35,-.10),(-.15,-.07),(.15,.05),(.20,.12),(.17,.15),(-.15,-.03),(-.35,-.06)],0,48)
    dish.rotation_euler.x=math.radians(65);dish.rotation_euler.z=-math.pi/2;dish.location=(-2.45,0,1.1)
    cyl('Dish mounting mast',(-2.45,0,.1),(-2.45,0,1.1),.075,13,8)

def rotate_array(objects,centre,angle):
    from mathutils import Matrix
    transform=Matrix.Translation(Vector((*centre,0))) @ Matrix.Rotation(angle,4,'Z') @ Matrix.Translation(Vector((-centre[0],-centre[1],0)))
    for o in objects:o.matrix_world=transform @ o.matrix_world

def crescent(s,kind):
    """Two Bezier boundaries form a curved, closed airfoil without concave fans."""
    heavy=kind=='battleship';small=kind=='courier';destroyer=kind=='destroyer'
    if small:
        front=[(.50,-.2),(1.55,.1),(3.6,.5),(4.75,.55)]
        back=[(.52,2.05),(1.9,2.5),(3.9,2.3),(4.78,1.35)]
    else:
        tip=-3.4 if heavy else (-2.75 if destroyer else -2.5)
        front=[(.60,-.20),(2.2,.05),(4.1,1.10),(4.28,tip)]
        back=[(.65,2.55),(5.65 if heavy else 5.2,4.0),(5.8 if heavy else 5.4,.30),(4.31,tip+.025)]
    def bez(points,t):
        return sum((Vector(p)*v for p,v in zip(points,[(1-t)**3,3*t*(1-t)**2,3*t*t*(1-t),t**3])),Vector((0,0)))
    n=10 if COARSE==2 else (16 if COARSE else 32)
    cross=[(0,0),(.16,.70),(.50,1),(.84,.65),(1,0),(.84,-.30),(.5,-.40),(.16,-.30)]
    verts=[]
    for i in range(n+1):
        t=i/n;f=bez(front,t);b=bez(back,t)
        height=(.48 if heavy else .25)*(1-.88*t*t)
        for u,h in cross:
            p=f.lerp(b,u);verts.append((s*p.x,p.y,.05+height*h*(2.2 if heavy and h<0 else 1)))
    k=len(cross)
    faces=[tuple(reversed(range(k))),tuple(range(n*k,(n+1)*k))]
    faces += [(i*k+j,i*k+(j+1)%k,(i+1)*k+(j+1)%k,(i+1)*k+j) for i in range(n) for j in range(k)]
    m=bpy.data.meshes.new('Continuous swept wing');m.from_pydata(verts,[],faces);m.update()
    o=bpy.data.objects.new('Continuous swept wing',m);bpy.context.collection.objects.link(o);finish(o,0)
    # Burgundy sail follows the wing surface; edge armour remains exposed.
    sailverts=[]
    for i in range(n+1):
        t=.07+.70*i/n;f=bez(front,t);b=bez(back,t)
        height=(.48 if heavy else .25)*(1-.88*t*t)
        for u,h in cross[1:4]:
            p=f.lerp(b,u);sailverts.append((s*p.x,p.y,.059+height*h))
    sf=[(i*3+j,i*3+j+1,(i+1)*3+j+1,(i+1)*3+j) for i in range(n) for j in range(2)]
    m=bpy.data.meshes.new('Inset burgundy sail');m.from_pydata(sailverts,[],sf);m.update()
    o=bpy.data.objects.new('Inset burgundy sail',m);bpy.context.collection.objects.link(o)
    # Mirrored sails must both face up with backface culling enabled.
    if s<0:
        for p in m.polygons:p.flip()
    finish(o,1)
    if small:
        blade('Double-ended courier scythe',[(s*4.10,-1.70),(s*4.85,-.45),(s*5.02,1.1),(s*4.55,3.20),(s*4.25,1.67),(s*4.05,.80)],.02,.27,0)
    # Gun platforms are embedded into the airfoil at sampled surface stations.
    if not small:
        for t in ((.25,.43,.60) if heavy else (.30,.52)):
            f=bez(front,t);b=bez(back,t);p=f.lerp(b,.5)
            z=.05+(.48 if heavy else .25)*(1-.88*t*t)
            if heavy:
                slab('Wing battery armoured deck',[(s*(p.x-.43),p.y-.40),(s*(p.x+.43),p.y-.30),(s*(p.x+.39),p.y+.38),(s*(p.x-.40),p.y+.48)],z-.24,z+.015,0,.08)
            disc('Integrated wing gun socket',(s*p.x,p.y),z-.025,.25,.10 if heavy else .16,2)
            if not COARSE:turret(s*p.x,p.y,z+.035)

def blade(label,points,z,depth,tile=0):
    # Sharper planform than Alliance rolled slabs, with a raised central ridge.
    n=len(points);cx=sum(p[0] for p in points)/n;cy=sum(p[1] for p in points)/n
    verts=[(x,y,z) for x,y in points]+[(x*.98+cx*.02,y*.98+cy*.02,z+.045) for x,y in points]+[(cx,cy,z+depth)]
    faces=[tuple(reversed(range(n)))]+[(i,(i+1)%n,n+(i+1)%n,n+i) for i in range(n)]+[(n+i,n+(i+1)%n,2*n) for i in range(n)]
    m=bpy.data.meshes.new(label);m.from_pydata(verts,[],faces);m.update()
    o=bpy.data.objects.new(label,m);bpy.context.collection.objects.link(o)
    return finish(o,tile)

def cut_armour_seams(hull,s):
    """Real shallow cuts in the battery hull, not plates laid over its roof."""
    rows=(-2.7,-1.85,-.95,.05,.95) if not COARSE else (-2.4,-.9,.7)
    lines=[((s*.56,y-.20),(s*1.79,y+.16)) for y in rows]
    lines += [((s*.98,-3.10),(s*1.10,-1.0)),((s*1.10,-1.0),(s*.91,1.48))]
    for a,b in lines:
        delta=Vector(b)-Vector(a);mid=(Vector(a)+Vector(b))*.5
        bpy.ops.mesh.primitive_cube_add(size=1,location=(mid.x,mid.y,.645))
        cutter=bpy.context.object;cutter.name='Temporary armour seam cutter'
        cutter.scale=(delta.length,.043,.15);cutter.rotation_euler.z=math.atan2(delta.y,delta.x)
        bpy.context.view_layer.objects.active=hull
        mod=hull.modifiers.new('Cut recessed armour joint','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=cutter
        bpy.ops.object.modifier_apply(modifier=mod.name)
        bpy.data.objects.remove(cutter,do_unlink=True)
    bm=bmesh.new();bm.from_mesh(hull.data);bmesh.ops.recalc_face_normals(bm,faces=bm.faces);bm.to_mesh(hull.data);bm.free()
    hull.data.update()
    # Machined groove walls need hard normals to preserve their sharp lips.
    for polygon in hull.data.polygons:polygon.use_smooth=False
    atlas_uv(hull,0)

def dynasty_ship(kind):
    heavy=kind=='battleship';small=kind=='courier';destroyer=kind=='destroyer'
    length=5.5 if heavy else (5.65 if destroyer else (4.3 if small else 4.8))
    width=1.2 if heavy else (.65 if destroyer else (.58 if small else .85))
    keel=[(-length,.035,0,.03),(-length*.60,width*.7,.06,.22),(-.3,width,.12,.40),(2.4,width*.75,.18,.34),(3.2,width*.18,.23,.12)]
    if kind=='cruiser':
        keel=[(-length,.05,0,.04),(-3.8,.72,0,.21),(-2.65,1.14,0,.32),(-1.35,1.04,.03,.30),(.1,.85,.12,.32),(2.4,.64,.18,.29),(3.2,.15,.23,.12)]
    if heavy:
        keel=[(-length,.045,0,.05),(-4.4,.75,.03,.22),(-3.2,1.32,.08,.39),(-1.5,1.75,.10,.47),(.8,1.85,.10,.49),(2.4,1.15,.13,.42),(3.2,.18,.20,.12)]
    shell('Exposed mechanical keel',keel,2,24)
    loft=fair_sections(keel,2 if COARSE else 4)
    def station_at(y):
        y=max(loft[0][0],min(loft[-1][0],y))
        a,b=next((a,b) for a,b in zip(loft,loft[1:]) if a[0]<=y<=b[0])
        t=(y-a[0])/(b[0]-a[0])
        return [a[k]+(b[k]-a[k])*t for k in (1,2,3)]
    for s in (-1,1):
        # Split lance tips and overlapping dorsal armour, leaving glowing gaps.
        for j in range(5 if not COARSE else 3):
            if heavy:continue
            count=5 if not COARSE else 3
            y=-length+.25+j*(length+2.1)/count
            w=width*(.32+.62*min(1,(j+1)/2))
            if kind=='cruiser':w=station_at(y+.4)[0]*.96
            plate=blade('Overlapping dorsal carapace',[(s*.04,y-.46),(s*w,y-.15),(s*w*1.12,y+.87),(s*.08,y+1.17)],0 if kind=='cruiser' else .25+j*.027,.13 if kind=='cruiser' else .23,0 if j%3 else 14)
            if kind=='cruiser':
                for v in plate.data.vertices:
                    local_w,z,h=station_at(v.co.y)
                    v.co.x=math.copysign(min(abs(v.co.x),local_w*1.02),v.co.x)
                    v.co.z+=z+h*math.sqrt(max(0,1-(v.co.x/(local_w*1.04))**2))+.018
                plate.data.update()
        crescent(s,kind)
        blade('Forward split jaw',[(s*.10,-length-.3),(s*.30,-length+1.5),(s*.86,-length+2.2),(s*.68,-length+.4)],-.05,.25,0)
        cyl('Jaw reactor slit',(s*.22,-length+.7,.13),(s*.38,-length+1.75,.21),.055,6,8)
        engine_x=s*(.82 if small else 1.20)
        cyl('Engine hot inner core',(engine_x,.65,.05),(engine_x,3.40,.24),.26,5,16,.18)
        # Segmented barrel ribs and ribs around the exposed flank.
        ribs=10 if not COARSE else (2 if COARSE==2 else 4)
        for j in range(ribs):
            y=-length*.62+j*(length*.62+2.4)/ribs
            x=s*width*(.78 if y<-.6 else .98)
            if kind=='cruiser':
                w,z,h=station_at(y)
                angles=[math.radians(a) for a in (-65,-25,15,55)]
                for a,b in zip(angles,angles[1:]):
                    cyl('Exposed curved flank rib',(s*(w+.035)*math.cos(a),y,z+(h+.028)*math.sin(a)),(s*(w+.035)*math.cos(b),y,z+(h+.028)*math.sin(b)),.028,13,6)
            else:cyl('Exposed flank rib',(x,y,-.17),(x+s*.15,y,.36),.027 if not COARSE else .04,13,6)
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
            for y in (-2.5,-1.3,.1):
                if kind=='cruiser':
                    w,z,h=station_at(y);w2,z2,_=station_at(y+.45)
                    cyl('Exposed flank reactor',(s*(w+.015),y,z),(s*(w2+.015),y+.45,z2),.055,6,8)
                    blade('Reactor eyebrow armour',[(s*w,y-.10),(s*(w+.15),y+.1),(s*(w2+.15),y+.5),(s*w2,y+.65)],z+.07,.13,0)
                else:cyl('Flank red reactor conduit',(s*width*.84,y,.01),(s*width*.91,y+.45,.04),.04,6,8)
    if not small:
        # The heavy hull has a broad four-tier citadel; the destroyer keeps a fin.
        cyl('Citadel structural core',(0,1.95,.23),(0,1.95,1.61 if heavy else 1.45),.51,2,8,.29)
        for j in range(4 if heavy else 3):
            z=(.67 if heavy else .54)+j*(.29 if heavy else .33);w=(.91 if heavy else .76)-j*.11;y=1.45+j*.22
            blade('Citadel overlapping armour',[(-w,y-.65),(w,y-.65),(w*.85,y+.8),(0,y+1.25),(-w*.85,y+.8)],z,.29,0)
            block('Citadel red slit',(0,y-.65,z+.05),(w*1.35,.05,.07),6,.006)
        for s in (-1,1):
            cyl('Citadel spire',(s*.36,2.0,1.25 if heavy else 1.15),(s*.29,3.65,2.22 if heavy else (2.1 if destroyer else 1.91)),.15,0,5,.007)
    if heavy:
        for s in (-1,1):
            hull=slab('Recessed armour battery hull',[(s*.40,-3.8),(s*1.15,-3.35),(s*1.90,-1.40),(s*1.94,.8),(s*1.30,1.9),(s*.38,1.6)],-.24,.63,0,.09)
            cut_armour_seams(hull,s)
            disc('Flush fore battery bearing',(s*1.28,-.65),.655,.35,.08,3)
            if not COARSE: turret(s*1.28,-.65,.70,True)

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
    # For the cut-armour LOD sources, let the glTF loader derive tangents from
    # the final simplified topology instead of interpolating opposing UV frames.
    tangents=not (COARSE and name=='dynasty_battleship')
    bpy.ops.export_scene.gltf(filepath=str(OUT/f'{name}{suffix}.glb'),export_format='GLB',use_selection=True,export_cameras=False,export_lights=False,export_apply=True,export_yup=True,export_tangents=tangents)
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
