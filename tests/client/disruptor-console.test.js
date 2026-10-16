/** @vitest-environment jsdom */
import fs from 'node:fs';
import {parse} from 'smol-toml';
import {afterEach,expect,it,vi} from 'vitest';
import {resolveTemplate} from '../../editor/entity-includes.js';
import {buildConsoleView} from '../../gui/console-state.js';
import {renderStation} from '../../gui/destroyer/tactical.console.js';
import {createTacticalActionRegistry,tacticalActionView} from '../../gui/stations/tactical-actions.js';
import {dispatchConsoleAction} from '../../gui/action-map.js';
import {normalizeConsolePayload} from '../../gui/console-payload.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import '../../gui/components/ph-blasters-controls.js';
const read=path=>fs.existsSync(path)?fs.readFileSync(path,'utf8'):null;
function setup(){
  const recipe=resolveTemplate('assets/entities/disruptor_cruiser.toml',{read},parse).resolved.value;
  const console=recipe.console.find(row=>row.id==='disruption');
  const definition={...console,systems:recipe.system.filter(row=>row.console===console.id).map(row=>row.id),feeds:console.feed};
  const state={consoleDefinitions:[definition],consoleHosts:{disruption:{host:'comms',rating:'Std'}},
    systemConsoleFamilies:{'blaster-disruptor':'tactical'},systemKinds:Object.fromEntries(recipe.system.map(row=>[row.id,row.kind])),
    controlSources:{'blaster-disruptor':'Human'},observationTick:12,
    blackboardKinds:{tactical:'Weapons','tactical-radar':'TacticalRadar',viewscreen:'Viewscreen',captain:'Captain'},
    blackboards:{tactical:{banks:[],tubes:[],blasters:[{id:'disruptor',fire_ready:true,on_cooldown:false,charge_progress:0,
      readiness:{ready:true,blocking_reason:'Ready'}}],assignments:{'blaster-disruptor':{individual:{mode:'hold'}}}},
    'tactical-radar':{blips:[{uuid:'earned-alias',name:'Known contact',kind:'ship',selectable:true,predicted:false,x:.1,y:.2}]},
    viewscreen:{combat_lock:null},captain:{red_alert:false}},
    blasterBankConfigs:[{id:'disruptor',cooldown_secs:6}]};
  return {state,recipe,definition};
}
afterEach(()=>{document.body.replaceChildren();delete window.activateSemanticAction;});
it('reuses actual Console ownership, Tactical builder, bank picker and bolt actions at Comms',()=>{
  const {state,definition}=setup();expect(definition.home_station).toBe('comms');
  expect(read(definition.screen)).toContain('ph-blasters-controls');
  let current=normalizeConsolePayload(buildConsoleView('disruption',state));
  expect(current.hosted_systems).toEqual(['blaster-disruptor']);
  let sequence=0;
  const wire=vi.fn(),registry=createTacticalActionRegistry({getState:()=>current,actionFeedback:new ActionFeedbackLifecycle({correlation:()=>`disruptor-${++sequence}`}),
    sendAction:(action,detail)=>dispatchConsoleAction({action,...detail},wire)});
  window.activateSemanticAction=(id,event)=>registry.activate(id,event);
  document.body.innerHTML='<ph-tactical-radar id="tactical-radar"></ph-tactical-radar><ph-blasters-controls id="blasters-controls"></ph-blasters-controls>';
  renderStation(current,document);
  const assignment=document.querySelector('[data-assignment-key="blaster-disruptor"]');
  expect(assignment).not.toBeNull();assignment.querySelector('select').value='earned-alias';assignment.querySelector('button').click();
  expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated',expect.objectContaining({target:'blaster-disruptor',payload:{type:'SetWeaponTarget',data:{uuid:'earned-alias'}}}));
  const fire=document.getElementById('blasters-controls').shadowRoot.querySelector('.btn');expect(fire.disabled).toBe(false);fire.dispatchEvent(new MouseEvent('mousedown'));
  expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated',expect.objectContaining({target:'blaster-disruptor',payload:{type:'ChargeBlasterStart'}}));
  const before=wire.mock.calls.length;state.controlSources['blaster-disruptor']='Ai';current=normalizeConsolePayload(buildConsoleView('disruption',state));renderStation(current,document);
  expect(fire.disabled).toBe(true);fire.dispatchEvent(new MouseEvent('mousedown'));
  registry.activate('tactical.blaster-fire',{context:'tactical',detail:{bank:'disruptor'}});
  expect(wire).toHaveBeenCalledTimes(before);
});
it('withdraws real bank controls when its explicit Weapons feed disappears',()=>{
  const {state,definition}=setup();definition.feeds=definition.feeds.filter(row=>row.kind!=='Weapons');
  const current=buildConsoleView('disruption',state);
  expect(tacticalActionView(current).blasters).toEqual([]);
  expect(tacticalActionView(current).weapon_targets.rows.every(row=>row.system==='blaster-disruptor')).toBe(true);
});
