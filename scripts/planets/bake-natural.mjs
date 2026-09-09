// Bake supplied gas/ice source layers into compact, browser-portable KTX2 maps.
// node scripts/planets/bake-natural.mjs [raw/planets]
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import sharp from 'sharp';
const here=path.dirname(fileURLToPath(import.meta.url));
const toktx=createRequire(import.meta.url).resolve('ktx2tools/toktx.js');
const raw=path.resolve(process.argv[2] ?? 'raw/planets');
const temporary=path.resolve('target/natural-planet-bake');
await fs.mkdir(temporary,{recursive:true});
const clamp=x=>Math.max(0,Math.min(1,x));
const byte=x=>Math.round(clamp(x)*255);
const smooth=x=>{x=clamp(x);return x*x*(3-2*x);};
// Remove the supplied dark frame consistently; preserve packed alpha as data.
async function read(kind,name,width=1024,enhanced=false){
 const file=enhanced?path.join(here,'sources',`${kind}_albedo_enhanced.png`):path.join(raw,`${kind}_shader_textures_1k`,`${kind}_${name}_1k.png`);
 const meta=await sharp(file).metadata();
 const trim=Math.round(meta.width*24/1024), top=Math.round(meta.height*6/512);
 const input=sharp(file).extract({left:trim,top,width:meta.width-2*trim,height:meta.height-2*top}).ensureAlpha();
 const data=Buffer.alloc(width*width/2*4);
 for(let c=0;c<4;c++){
  const channel=await input.clone().extractChannel(c).resize(width,width/2).raw().toBuffer();
  for(let p=0;p<channel.length;p++)data[p*4+c]=channel[p];
 }
 return data;
}
async function resize(data,width,out){
 const result=Buffer.alloc(out*out/2*4);
 for(let c=0;c<4;c++){
  const v=await sharp(data,{raw:{width,height:width/2,channels:4}}).extractChannel(c).resize(out,out/2).raw().toBuffer();
  for(let p=0;p<v.length;p++)result[p*4+c]=v[p];
 }
 return result;
}
function repair(data,w,normal=false){
 const h=w/2, band=Math.max(2,Math.round(w/64));
 for(let y=0;y<h;y++){
  const polar=1-smooth(Math.min(y,h-1-y)/(h*0.07));
  const mean=[0,0,0,0];
  for(let x=0;x<w;x++)for(let c=0;c<4;c++)mean[c]+=data[(y*w+x)*4+c]/w;
  for(let x=0;x<w;x++)for(let c=0;c<4;c++){
   const i=(y*w+x)*4+c;
   const cap=normal?[128,128,255,255][c]:mean[c];
   data[i]=Math.round(data[i]*(1-polar)+cap*polar);
  }
  for(let x=0;x<band;x++)for(let c=0;c<4;c++){
   const a=(y*w+x)*4+c,b=(y*w+w-1-x)*4+c, avg=(data[a]+data[b])/2,t=smooth(x/(band-1));
   data[a]=Math.round(avg+(data[a]-avg)*t);data[b]=Math.round(avg+(data[b]-avg)*t);
  }
 }
}
async function write(out,name,data,w,srgb=false,normal=false,scalar=false){
 repair(data,w,normal);
 const png=path.join(temporary,`${path.basename(out)}_${name}.png`);
 const source=sharp(data,{raw:{width:w,height:w/2,channels:4}});
 await (scalar?source.extractChannel(0):source).png().toFile(png);
 const args=['--t2','--genmipmap','--zcmp','9','--assign_oetf',srgb?'srgb':'linear'];
 if(normal)args.push('--normal_mode');if(scalar)args.push('--target_type','R');
 execFileSync(process.execPath,[toktx,...args,path.join(out,name+'.ktx2'),png],{stdio:'pipe'});
 console.log(`${path.basename(out)}/${name}: ${w}x${w/2} ${scalar?'R8':'RGBA8'}`);
}
async function optical(out,outer){
 const w=256,h=128,data=Buffer.alloc(w*h*4);
 for(let y=0;y<h;y++)for(let x=0;x<w;x++){
  const r=1+(outer-1)*y/(h-1),mu=x/(w-1)*2-1;
  const distance=-r*mu+Math.sqrt(r*r*mu*mu+outer*outer-r*r);
  let molecular=0,aerosol=0;
  for(let s=0;s<64;s++){
   const t=distance*(s+.5)/64,alt=Math.max(0,(Math.sqrt(r*r+t*t+2*r*mu*t)-1)/(outer-1));
   molecular+=Math.exp(-alt*6)*distance/64/(outer-1);aerosol+=Math.exp(-alt*12)*distance/64/(outer-1);
  }
  const i=(y*w+x)*4;data[i]=byte(1-Math.exp(-molecular/8));data[i+1]=byte(1-Math.exp(-aerosol/8));data[i+3]=255;
 }
 await sharp(data,{raw:{width:w,height:h,channels:4}}).png().toFile(path.join(out,'optical_depth.png'));
}
for(const kind of ['gas_giant','ice_moon']){
 const ice=kind==='ice_moon',out=path.resolve('assets/planets',kind);
 await fs.mkdir(out,{recursive:true});
 const names=ice?['normal','frost_detail_normal','roughness','ao','frost_mask','exposed_rock_mask','thin_ice_mask','subsurface_ocean_mask','fracture_mask','thermal_emissive_mask','cryogeyser_mask','nightglow_colour','nightglow_mask','aurora_colour','aurora_mask']:
 ['normal','roughness','height','storm_mask','band_zone_mask','lightning_mask','nightglow_colour','nightglow_mask','aurora_colour','aurora_mask'];
 const maps={};for(const name of names)maps[name]=await read(kind,name);
 const normal=Buffer.from(maps.normal),material=Buffer.alloc(1024*512*4),effects=Buffer.alloc(material.length),aurora=Buffer.from(maps.aurora_colour);
 for(let p=0;p<1024*512;p++){
  const i=p*4,at=name=>maps[name][i]/255;
  if(ice){
   const frost=at('frost_mask'),thin=clamp(at('thin_ice_mask')*.65+at('subsurface_ocean_mask')*.35);
   material[i]=byte(clamp(at('roughness')*.5+frost*.4+at('exposed_rock_mask')*.2-thin*.2+.12));
   material[i+1]=byte(.4+.6*at('ao'));material[i+2]=byte(thin);material[i+3]=128+Math.round(frost*127);
   const nx=(normal[i]/255*2-1)+(maps.frost_detail_normal[i]/255*2-1)*frost*.28;
   const ny=(normal[i+1]/255*2-1)+(maps.frost_detail_normal[i+1]/255*2-1)*frost*.28;
   const nz=Math.max(.25,normal[i+2]/255*2-1),len=Math.hypot(nx,ny,nz);
   normal[i]=byte(nx/len*.5+.5);normal[i+1]=byte(ny/len*.5+.5);normal[i+2]=byte(nz/len*.5+.5);
   effects[i]=byte(at('nightglow_mask')*.2);
   effects[i+1]=byte(Math.pow(at('cryogeyser_mask'),3)*at('fracture_mask'));
   effects[i+2]=byte(Math.pow(at('thermal_emissive_mask'),2)*at('fracture_mask'));
  }else{
   material[i]=byte(.7+.25*at('roughness'));material[i+1]=maps.band_zone_mask[i];material[i+2]=maps.storm_mask[i];material[i+3]=128+Math.round(at('height')*127);
   effects[i]=byte(at('nightglow_mask')*.25);effects[i+1]=byte(at('lightning_mask')*at('storm_mask'));effects[i+2]=0;
  }
  normal[i+3]=255;effects[i+3]=255;
  const latitude=Math.abs(2*Math.floor(p/1024)/511-1);
  const polar=smooth((latitude-.58)/.22)*(1-smooth((latitude-.94)/.06));
  for(let c=0;c<3;c++)aurora[i+c]=byte(aurora[i+c]/255*at('aurora_mask')*polar);
  aurora[i+3]=255;
 }
 await write(out,'surface_colour',await read(kind,'albedo',4096,true),4096,true);
 await write(out,'surface_normal',await resize(normal,1024,2048),2048,false,true);
 await write(out,'surface_material',material,1024);
 await write(out,'surface_effects',effects,1024);
 await write(out,'emission_colour',await resize(maps.nightglow_colour,1024,512),512,true);
 await write(out,'aurora',await resize(aurora,1024,512),512,true);
 const cloudPrefix=ice?'haze':'cloud';
 await write(out,'upper_colour',await read(kind,ice?'haze_albedo':'cloud_colour',1024),1024,true);
 await write(out,'upper_normal',await read(kind,`${cloudPrefix}_normal`,512),512,false,true);
 const opacity=await read(kind,`${cloudPrefix}_opacity`,512);
 await write(out,'upper_opacity',opacity,512,false,false,true);
 await write(out,'atmosphere_density',await read(kind,`${cloudPrefix}_opacity`,256),256,false,false,true);
 await optical(out,ice?1.025:1.045);
}
