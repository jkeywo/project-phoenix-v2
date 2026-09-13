/** Authored data only. Neither validation nor equivalents read live world state. */
import stockAssets from './sound-cue-inventory.js';
const CATEGORIES = ['music','ambience','effects','alerts','interface'];
const AUDIENCES = ['viewscreen','station','gm'];
const URGENCIES = ['info','advisory','warning','critical'];
const object = value => value && typeof value === 'object' && !Array.isArray(value);
const text = value => typeof value === 'string' && value.trim() && [...value].length <= 512;
const keys = (value, allowed) => object(value) && Object.keys(value).every(key => allowed.includes(key));
export const SOUND_CUES_PATH = 'assets/audio/sound-cues.toml';
export const SOUND_CUES_JSON = 'assets/audio/sound-cues.json';
const assetPath = value => typeof value === 'string' && value.startsWith('assets/sounds/') && /\.(mp3|ogg|wav)$/.test(value)
  && value.split('/').every(part=>part && part!=='.' && part!=='..' && !/[. ]$/.test(part) && !/[\\:%?#<>"|*\x00-\x1f\x7f-\x9f]/.test(part));
function validAssets(assets) {
  return Array.isArray(assets) && assets.length <= 128 && new Set(assets.map(a=>a?.file)).size===assets.length
    && assets.every(a=>keys(a,['file','category','informative']) && assetPath(a.file)
      && CATEGORIES.includes(a.category) && typeof a.informative==='boolean');
}
export function validateSoundDefinition(value, assets, floors=stockAssets) {
  if (!keys(value,['id','label','file','category','audience','volume','equivalent'])
      || !text(value.id) || !/^[a-z0-9][a-z0-9-]*$/.test(value.id) || !text(value.label)) return 'definition';
  const asset = Array.isArray(assets) && assets.find(asset => asset.file === value.file);
  if (!asset || !assetPath(value.file)) return 'asset';
  if (floors.some(known=>known.file===asset.file && (known.category!==asset.category || (known.informative&&!asset.informative)))) return 'category';
  if (!CATEGORIES.includes(value.category) || asset.category !== value.category) return 'category';
  if (!AUDIENCES.includes(value.audience)
      || (value.audience !== 'viewscreen' && !['alerts','interface'].includes(value.category))) return 'audience';
  if (!Number.isFinite(value.volume) || value.volume < 0 || value.volume > 1) return 'volume';
  const equivalent=value.equivalent;
  if (!equivalent) return asset.informative || ['alerts','effects'].includes(value.category) ? 'equivalent' : null;
  if (!keys(equivalent,['meaning','source','urgency','bearing','elevation'])
      || !text(equivalent.meaning) || !text(equivalent.source) || !URGENCIES.includes(equivalent.urgency)) return 'equivalent';
  if (equivalent.bearing != null && (!Number.isFinite(equivalent.bearing) || Math.abs(equivalent.bearing)>180)) return 'direction';
  if (equivalent.elevation != null && (!Number.isFinite(equivalent.elevation) || Math.abs(equivalent.elevation)>90 || equivalent.bearing == null)) return 'direction';
  return null;
}

export function validateSoundCatalog(value, floors = stockAssets) {
  const assets=value?.assets;
  if (!keys(value,['version','assets','cues']) || value.version !== 1 || !Array.isArray(value.cues)
      || value.cues.length > 128 || !validAssets(assets) || !validAssets(floors)
      || assets.some(a=>floors.some(b=>a.file===b.file && (a.category!==b.category || (b.informative&&!a.informative))))) return {cues:[],findings:[{id:'',code:'catalog'}]};
  const seen=new Set(),cues=[],findings=[];
  for (const definition of value.cues) {
    const code=seen.has(definition?.id) ? 'duplicate' : validateSoundDefinition(definition,assets,floors);
    seen.add(definition?.id);
    if (code) findings.push({id: typeof definition?.id === 'string' ? definition.id : '',code});
    else cues.push(structuredClone(definition));
  }
  return {cues,findings};
}
