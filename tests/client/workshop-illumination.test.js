import {expect,it} from 'vitest';
import {initialIllumination,validateIlluminationComposition,initialTurret} from '../../editor/workshop-system-composition.js';
import {systemTreeVocabulary} from '../../gui/workshop-state-trees.js';
it('validates exact typed composition and finite mounted equipment before authoring commit',()=>{
 const system={kind:'illumination',power_group:'science'},valid=initialIllumination();
 validateIlluminationComposition(system,valid.components,valid.channels);
 const mounted=[...valid.components,{...initialTurret()[1],power_input:'input'}];validateIlluminationComposition(system,mounted,valid.channels);
 for(let fault=0;fault<7;fault++){
  const candidate=structuredClone(valid);let owner=system;
  if(fault===0)candidate.components[1].config.link_range=1e-100;
  if(fault===1)candidate.channels[1].kind='boolean';
  if(fault===2)candidate.components.push({kind:'power_input',id:'unused'});
  if(fault===3)candidate.components[1].power_input='missing';
  if(fault===4)candidate.components[1].config.reading_ticks=1.5;
  if(fault===5)candidate.components[1].config.mount.range=Infinity;
  if(fault===6)owner={...system,power_group:null};
  expect(()=>validateIlluminationComposition(owner,candidate.components,candidate.channels)).toThrow();
 }
});
it('AI vocabulary follows both actual generic channel names without a second target owner',()=>{
 const schema={hosts:{illumination:{facts:['enabled','has_target','target_available','request_available'],channels:['enabled','target']}},sources:['literal','fact'],actions:['write']};
 const vocabulary=systemTreeVocabulary({kind:'illumination',component:[{kind:'illumination_emitter',enabled_channel:'active',target_channel:'subject'}]},schema);
 expect(vocabulary.channels).toEqual(['active','subject']);expect(vocabulary.facts).toContain('request_available');
});
