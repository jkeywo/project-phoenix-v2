// Assemble review renders and measured costs into a local review artifact.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import path from 'node:path';
import sharp from 'sharp';
const specs=JSON.parse(await readFile('scripts/art/fleet.json','utf8'));
const validation=JSON.parse(await readFile('scripts/art/fleet-validation.json','utf8'));
const out='raw/models/fleet-recreated';await mkdir(out,{recursive:true});
const cells=[];const width=640,height=520;
for(const [i,s] of specs.entries()){
  const render=await sharp(`raw/models/${s.concept}/recreated/preview_hero.png`).resize(640,480).png().toBuffer();
  const r=validation.find(v=>v.model===s.name&&v.variant==='model');
  const title=s.name.split('_').map(w=>w[0].toUpperCase()+w.slice(1)).join(' ');
  const label=Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="640" height="64"><rect width="640" height="64" fill="#192330"/><text x="20" y="26" font-family="Arial" font-size="20" fill="#edf5fc">${title}</text><text x="20" y="49" font-family="Arial" font-size="14" fill="#b4c9db">LOD triangles: ${r.levels.map(l=>l.triangles.toLocaleString('en-GB')).join(' / ')}</text></svg>`);
  const left=(i%4)*width,top=Math.floor(i/4)*height;
  cells.push({input:render,left,top});cells.push({input:label,left,top:top+456});
}
await sharp({create:{width:width*4,height:height*Math.ceil(specs.length/4),channels:4,background:'#263343'}}).composite(cells).png().toFile(`${out}/fleet-preview.png`);
const abs=p=>path.resolve(p).replaceAll('\\','/');
const rows=validation.filter(v=>v.variant==='model').map(r=>`| ${r.model} | ${r.originalTriangles.toLocaleString('en-GB')} | ${r.levels.map(l=>l.triangles.toLocaleString('en-GB')).join(' | ')} | ${(r.levels[0].bytes/1048576).toFixed(2)} |`).join('\n');
const md=`# Combat Test / Falling Skyway fleet recreation\n\n![Fleet overview](${abs(`${out}/fleet-preview.png`)})\n\n| Model | Previous triangles | Near | Middle | Far | Near MiB |\n|---|---:|---:|---:|---:|---:|\n${rows}\n\nThe ${specs.length} base models have eight-view billboards beyond 400 units. The courier docking variant preserves its original separate scale and three mesh tiers. Near models have one primitive/material and embedded PBR textures.\n\nThe original rendered bounds, all ${validation.reduce((n,r)=>n+r.markers,0)} migrated markers (including the docking variant), and ${validation.reduce((n,r)=>n+r.targetPoints,0)} anonymous target points are checked numerically. These are interpretations of the concept sheets; single-view references do not specify unseen surfaces. Geometry and byte costs are measured; FPS is not benchmarked.\n\n${specs.map(s=>`## ${s.name}\n\n[Open in Phoenix](http://127.0.0.1:8081/?model=assets%2Fmodels%2F${s.name}_recreated.glb&lighting=directional)\n\n![${s.name}](${abs(`raw/models/${s.concept}/recreated/preview_hero.png`)})\n\nConcept: [${s.concept}.png](${abs(`raw/models/${s.concept}.png`)})\n`).join('\n')}`;
await writeFile(`${out}/review.md`,md);
console.log(abs(`${out}/review.md`));
