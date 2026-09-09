import fs from 'node:fs/promises';
import assert from 'node:assert/strict';
import {parse} from 'smol-toml';
for(const [entity,kind,scale] of [['planet_gas_giant','gas_giant',1.045],['moon_ice','ice',1.025]]){
 const {planet}=parse(await fs.readFile(`assets/entities/${entity}.toml`,'utf8'));
 assert.equal(planet.surface.natural.kind,kind);
 assert.equal(planet.surface.city,undefined);
 assert.equal(planet.atmosphere.scattering.scale,scale);
 assert.ok(planet.clouds.scale<scale);
 const s=planet.surface,c=planet.clouds,a=planet.atmosphere.scattering;
 const maps=[[s.albedo,4096,43],[s.normal,2048,37],[s.roughness,1024,37],[s.emissive_mask,1024,37],[s.emissive_colour,512,43],[c.albedo,1024,43],[c.normal,512,37],[c.opacity,512,9],[a.haze,256,9],[a.skyglow,512,43]];
 let disk=0,gpu=0;
 for(const [file,w,format] of maps){
  const b=await fs.readFile(file),channels=format===9?1:4;
  assert.equal(b.subarray(1,7).toString(),'KTX 20',file);
  assert.equal(b.readUInt32LE(12),format,file);
  assert.equal(b.readUInt32LE(20),w,file);assert.equal(b.readUInt32LE(24),w/2,file);
  const mips=Math.floor(Math.log2(w))+1;
  assert.equal(b.readUInt32LE(40),mips,file);assert.equal(b.readUInt32LE(44),2,file);
  for(let m=0;m<mips;m++){
   const bytes=Math.max(1,w>>m)*Math.max(1,(w/2)>>m)*channels;
   assert.equal(Number(b.readBigUInt64LE(96+m*24)),bytes,file);gpu+=bytes;
  }
  disk+=b.length;
 }
 const lut=await fs.readFile(a.optical_depth);
 assert.equal(lut.readUInt32BE(16),256);assert.equal(lut.readUInt32BE(20),128);
 console.log(JSON.stringify({entity,maps:11,downloadMiB:(disk+lut.length)/1048576,residentMiB:(gpu+256*128*4)/1048576}));
}
