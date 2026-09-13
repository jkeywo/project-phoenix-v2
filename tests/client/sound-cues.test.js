import { expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { parse, stringify } from 'smol-toml';
import { validateSoundDefinition, validateSoundCatalog, SOUND_CUES_PATH } from '../../gui/sound-cues.js';
import { isAllowedContentPath } from '../../editor/mod-pack-export.js';
import { soundCuesJson, soundCueInventoryJs } from '../../scripts/sound-cues.mjs';
const catalog=JSON.parse(readFileSync(new URL('../../assets/audio/sound-cues.json',import.meta.url),'utf8'));
const cases=JSON.parse(readFileSync(new URL('../fixtures/sound-cue-validation.json',import.meta.url),'utf8'));
it.each(cases)('$name',({cue,expected})=>expect(validateSoundDefinition(cue,catalog.assets)).toBe(expected));
it('keeps generated delivery data identical to authored source and admits only the editable catalog',async()=>{
  const source=readFileSync(new URL('../../assets/audio/sound-cues.toml',import.meta.url),'utf8');
  expect(catalog).toEqual(parse(source));expect(validateSoundCatalog(catalog).findings).toEqual([]);
  expect(JSON.parse(await soundCuesJson(fileURLToPath(new URL('../..',import.meta.url))))).toEqual(catalog);
  expect(await soundCueInventoryJs(fileURLToPath(new URL('../..',import.meta.url)))).toBe(readFileSync(new URL('../../gui/sound-cue-inventory.js',import.meta.url),'utf8'));
  expect(isAllowedContentPath(SOUND_CUES_PATH)).toBe(true);
  expect(isAllowedContentPath('assets/audio/sound-cues.json')).toBe(false);
  expect(isAllowedContentPath('assets/audio/other.toml')).toBe(false);
});
it('refuses duplicate identities, unknown keys, changed inventory floors and invalid directions',()=>{
  const changed=structuredClone(catalog);changed.cues.push(changed.cues[0]);
  expect(validateSoundCatalog(changed).findings[0].code).toBe('duplicate');
  const cue={...catalog.cues[0],speech:true};expect(validateSoundDefinition(cue,catalog.assets)).toBe('definition');
  const inventory=structuredClone(catalog);inventory.assets[3].informative=false;
  expect(validateSoundCatalog(inventory,catalog.assets).findings[0].code).toBe('catalog');
  expect(validateSoundCatalog({...catalog,assets:[...catalog.assets,catalog.assets[0]]}).findings[0].code).toBe('catalog');
  expect(validateSoundDefinition({...catalog.cues[2],equivalent:{meaning:'Shot',source:'Source',urgency:'critical',bearing:181}},catalog.assets)).toBe('direction');
  expect(parse(stringify(catalog))).toEqual(catalog);
});
it('permits a declared nested packaged sound without copying the stock inventory',()=>{
  const asset={file:'assets/sounds/custom/sonar ping.wav',category:'alerts',informative:true};
  const cue={id:'sonar',label:'Sonar',file:asset.file,category:'alerts',audience:'station',volume:0.2,
    equivalent:{meaning:'Report ready',source:'Sensors',urgency:'advisory'}};
  expect(validateSoundCatalog({version:1,assets:[asset],cues:[cue]}).findings).toEqual([]);
  expect(validateSoundCatalog({version:1,assets:[],cues:[cue]}).findings[0].code).toBe('asset');
  expect(validateSoundCatalog({version:1,assets:[{...asset,file:'assets/sounds/../escape.wav'}],cues:[cue]}).findings[0].code).toBe('catalog');
});
