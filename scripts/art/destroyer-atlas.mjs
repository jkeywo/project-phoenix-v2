// Deterministic, model-specific trim atlas. Run from the repository root.
import sharp from 'sharp';
import { mkdir, writeFile } from 'node:fs/promises';
const out = process.env.PHOENIX_ART_OUT || 'raw/models/PPAllianceDestroyer/recreated';
const dynasty = process.env.PHOENIX_ART_FACTION === 'dynasty';
const title = process.env.PHOENIX_ART_TITLE || 'HORIZON';
const fleet = Boolean(process.env.PHOENIX_ART_OUT);
await mkdir(out, { recursive: true });
const S = 1024, T = 256;
const colors = ['#adb7c2','#657783','#1c2935','#45515e','#101b26','#123448','#11729d','#061522','#bcc6d0','#bcc6d0','#273441','#202d38','#0d151d','#75838b','#b6c1c9','#bf8244'];
if (dynasty) colors.splice(0,16,'#343637','#632b30','#191c20','#625349','#100f10','#441a15','#913128','#160d0c','#6a5946','#393635','#514334','#292524','#151214','#85705a','#432f31','#c57832');
const rough = [165,160,155,105,185,72,90,100,150,155,170,140,165,90,150,125];
const metal = [80,95,155,220,120,120,110,20,65,70,100,185,120,230,80,90];
let layers = { base: [], emissive: [], height: [], orm: [] };
const rect = (x,y,w,h,c,rx=0) => `<rect x="${x}" y="${y}" width="${w}" height="${h}" fill="${c}" rx="${rx}"/>`;
const line = (x1,y1,x2,y2,c,w=1) => `<path d="M${x1} ${y1}L${x2} ${y2}" stroke="${c}" stroke-width="${w}"/>`;
let seed=3287;
const rand=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed/4294967296;};
for (let i=0;i<16;i++) {
  let b=rect(0,0,T,T,colors[i]), e=rect(0,0,T,T,'black'), h=rect(0,0,T,T,'#808080');
  if ([0,1,2,3,14].includes(i)) {
    // Offset armour courses; no directional lighting painted into the albedo.
    const ink=i<2||i===14?'#596a78':'#111d27';
    for (let row=0;row<3;row++) {
      const y=10+row*81;
      b+=line(7,y,249,y,ink,1.5); h+=line(7,y,249,y,'#505050',2);
      const x= row%2 ? 86:168;
      b+=line(x,y,x,y+78,ink,1.5); h+=line(x,y,x,y+78,'#505050',2);
      for (const xx of [15,x-7,x+7,240]) {
        b+=rect(xx,y+5,2,3,'#465661'); h+=rect(xx,y+5,2,3,'#484848');
      }
      if (i===0||i===14) b+=rect(25,y+55,22,3,'#6d7e8b')+rect(25,y+61,12,1,'#6d7e8b');
    }
    if(i===14) b+=rect(110,0,27,256,'#4b606f')+rect(141,0,6,256,'#4b606f');
    // Fine staggered maintenance panels, access hatches, fasteners and stencils.
    // A seeded recipe keeps wear repeatable without painting in studio lighting.
    for(let row=0;row<7;row++){
      const y=9+row*35;
      let x=8;
      while(x<240){
        const w=Math.min(33+rand()*43,248-x),hh=31;
        const pts=`${x},${y+4} ${x+4},${y} ${x+w-5},${y} ${x+w},${y+5} ${x+w},${y+hh} ${x},${y+hh}`;
        const shade=rand()>.5?'#dce6ee':'#374d5c';
        b+=`<polygon points="${pts}" fill="${shade}" fill-opacity="${.055+rand()*.09}" stroke="${ink}" stroke-width=".6"/>`;
        h+=`<polygon points="${pts}" fill="#808080" stroke="#484848" stroke-width=".7"/>`;
        for(const dx of [3,w-4])for(const dy of [7,hh-3]){
          b+=`<circle cx="${x+dx}" cy="${y+dy}" r=".65" fill="${ink}"/>`;
          h+=`<circle cx="${x+dx}" cy="${y+dy}" r=".65" fill="#414141"/>`;
        }
        if(rand()>.45){
          b+=rect(x+8,y+10,11,7,ink,1)+rect(x+9,y+11,9,5,colors[i],.7);
          b+=`<text x="${x+8}" y="${y+24}" font-family="Arial" font-size="2.7" fill="${ink}">SVC ${row}${Math.floor(x)}</text>`;
        }
        if(rand()>.72)b+=rect(x+w-11,y+9,5,2,'#aa813c')+rect(x+w-11,y+13,5,.6,ink);
        x+=w+2;
      }
    }
    for(let k=0;k<950;k++){
      const x=8+rand()*240,y=8+rand()*240,l=.25+rand()*2.8;
      b+=`<path d="M${x} ${y}l${l} ${rand()*.45}" stroke="${rand()>.65?'#edf3f7':'#344957'}" stroke-opacity="${.12+rand()*.22}" stroke-width="${.15+rand()*.25}"/>`;
    }
  }
  if ([4,11,13].includes(i)) {
    for(let y=14;y<244;y+=18) {
      b+=rect(12,y,232,8,i===13?'#a0abb1':'#465762',2);
      h+=rect(12,y,232,8,'#b0b0b0',2);
    }
  }
  if(i===5) {
    b+=rect(12,20,232,216,'#0a1a29',24);
    for(let y=44;y<230;y+=42) {
      b+=rect(23,y,210,16,'#54cfff',7)+rect(28,y+4,200,5,'#c1f5ff',2);
      e+=rect(23,y,210,16,'#2e9be1',7)+rect(28,y+4,200,5,'#8ee4ff',2);
      for(let x=34;x<232;x+=14) b+=rect(x,y+2,3,12,'#368db3');
    }
  }
  if(i===6) {b+=rect(7,48,242,160,'#52d0ff',8)+rect(10,96,236,64,'#b5f0ff'); e+=rect(7,48,242,160,'#1681cf',8)+rect(10,96,236,64,'#64d0ff');}
  if(i===7) {
    for(let x=12;x<240;x+=40) {
      b+=rect(x,28,32,198,'#184561',4)+rect(x+3,31,26,5,'#7addff');
      e+=rect(x+3,31,26,5,'#298bc6')+rect(x+3,36,2,135,'#105184');
    }
  }
  if(i===9) {
    b+=`<text x="128" y="126" text-anchor="middle" font-family="Arial" font-size="${fleet?32:44}" font-weight="700" fill="#253643">${title}</text><text x="128" y="160" text-anchor="middle" font-family="Arial" font-size="24" fill="#334956">${fleet?'AEV / 0741':'NX-3287'}</text>`;
    b+=line(30,179,226,179,'#778b98',2);
  }
  if(i===10) for(let x=-220;x<270;x+=48) b+=`<path d="M${x} 0h20l256 256h-20z" fill="#c49b4f"/>`;
  if(i===15) {b+=rect(12,12,232,232,'#ffc88a',20);e+=rect(12,12,232,232,'#d48736',20);}
  if(i===12 && fleet) {
    b+=rect(8,8,240,240,'#132b48');
    for(let x=10;x<249;x+=15) { b+=line(x,8,x,248,'#778f9e',.7); h+=line(x,8,x,248,'#555555',1); }
    for(let y=10;y<249;y+=24) b+=line(8,y,248,y,'#94a0a6',.7);
  }
  if(dynasty && [0,1,2,14].includes(i)) {
    // Irregular overlapping dark plates with exposed bronze edges.
    for(let y=-12;y<260;y+=43) for(let x=-16;x<260;x+=57) {
      const dx=rand()*16,dy=rand()*10;
      const pts=`${x+dx},${y+dy} ${x+39},${y-2} ${x+57},${y+20} ${x+41},${y+43} ${x+5},${y+38}`;
      b+=`<polygon points="${pts}" fill="${colors[i]}" fill-opacity=".7" stroke="#857058" stroke-width="1.1"/>`;
      h+=`<polygon points="${pts}" fill="#888888" stroke="#454545" stroke-width="1.3"/>`;
      b+=line(x+8,y+34,x+30,y+37,'#b39975',.6);
    }
  }
  for (const [name,body] of Object.entries({base:b,emissive:e,height:h,orm:rect(0,0,T,T,`rgb(255,${rough[i]},${metal[i]})`)})) layers[name].push(`<g transform="translate(${i%4*T},${Math.floor(i/4)*T})">${body}</g>`);
}
const svg = content => {
  let body=content.join('');
  if(dynasty) for(const [from,to] of Object.entries({'#54cfff':'#ed4625','#c1f5ff':'#ffd39a','#2e9be1':'#cf2812','#8ee4ff':'#ff982d','#52d0ff':'#dd361a','#b5f0ff':'#ffc071','#1681cf':'#d3260d','#64d0ff':'#ff7130','#7addff':'#f38a42','#298bc6':'#a52c12','#105184':'#5b1209'})) body=body.replaceAll(from,to);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${S}" height="${S}">${body}</svg>`;
};
const sizes = { base:2048, emissive:512, orm:512, normal:1024 };
for (const name of ['base','emissive','orm']) await sharp(Buffer.from(svg(layers[name])),{density:name==='base'?144:72}).resize(sizes[name],sizes[name]).png().toFile(`${out}/destroyer_${name}.png`);
const height=await sharp(Buffer.from(svg(layers.height))).greyscale().raw().toBuffer();
const normal=Buffer.alloc(S*S*3);
for(let y=0;y<S;y++)for(let x=0;x<S;x++){
  const at=(xx,yy)=>height[Math.max(0,Math.min(S-1,yy))*S+Math.max(0,Math.min(S-1,xx))]/255;
  let nx=(at(x-1,y)-at(x+1,y))*1.5, ny=(at(x,y+1)-at(x,y-1))*1.5;
  const length=Math.hypot(nx,ny,1), p=(y*S+x)*3;
  normal[p]=Math.round((nx/length*.5+.5)*255);normal[p+1]=Math.round((ny/length*.5+.5)*255);normal[p+2]=Math.round((1/length*.5+.5)*255);
}
const reduced=await sharp(normal,{raw:{width:S,height:S,channels:3}}).resize(sizes.normal,sizes.normal).raw().toBuffer();
for(let p=0;p<reduced.length;p+=3){
  const n=[reduced[p]/127.5-1,reduced[p+1]/127.5-1,reduced[p+2]/127.5-1];
  const len=Math.hypot(...n);
  for(let c=0;c<3;c++)reduced[p+c]=Math.round((n[c]/len*.5+.5)*255);
}
await sharp(reduced,{raw:{width:sizes.normal,height:sizes.normal,channels:3}}).png().toFile(`${out}/destroyer_normal.png`);
await writeFile(`${out}/destroyer_atlas.svg`,svg(layers.base));
console.log(`Wrote PBR atlases ${JSON.stringify(sizes)} to ${out}`);
