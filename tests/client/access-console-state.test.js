import {describe,it,expect} from 'vitest';
import {buildConsoleView} from '../../gui/console-state.js';
import {familySystemId} from '../../gui/console-payload.js';
function state() {return {
  consoleDefinitions:[
    {id:'engine',home_station:'seat',systems:['hatch','repair'],feeds:[]},
    {id:'watch',home_station:'seat',systems:['other-hatch'],feeds:[{source:'repair',kind:'Repair'}]},
  ],
  consoleHosts:{engine:{host:'seat'},watch:{host:'seat'}},
  systemConsoleFamilies:{hatch:'access','other-hatch':'access',repair:'repair'},
  systemKinds:{hatch:'access-gate','other-hatch':'access-gate',repair:'repair'},
  controlSources:{hatch:'Human','other-hatch':'Human',repair:'Human'},
  blackboardKinds:{repair:'Repair'},
  blackboards:{repair:{teams:[],system_hull:[],damageable_systems:[],boundaries:[
    {system:'hatch',phase:'open',seal_requested:false,actuator_sealed:false},
    {system:'other-hatch',phase:'sealed',seal_requested:true,actuator_sealed:true},
  ]}},
};}
describe('access Console ownership',()=>{
  it('shows physical passage progress instead of the obsolete phase timer',()=>{
    const input=state();const board=input.blackboards.repair;
    board.interior={connection:[{id:'hall',travel_secs:4}]};
    board.navigation=[{location:{kind:'crossing',connection:'hall',elapsed:1}}];
    board.teams=[{Travelling:{system_id:'a',display_name:'A',elapsed:99}}];
    expect(buildConsoleView('engine',input).systems.repair.teams[0].progress_pct).toBe(0.25);
  });
  it('does not let a gate become the representative Repair command target',()=>{
    const payload=buildConsoleView('engine',state());
    expect(familySystemId(payload,'repair')).toBe('repair');
    expect(payload.systems.repair.repair_auto).toBe(false);
    expect(payload.systems.hatch.repair_auto).toBe(true);
    expect(payload.systems.repair.boundaries.map(row=>row.controllable)).toEqual([true,false]);
  });
  it('typed Repair information and cohosting grant no foreign gate or team control',()=>{
    const input=state();const payload=buildConsoleView('watch',input);
    expect(payload.boundaries.map(row=>row.controllable)).toEqual([false,true]);
    expect(payload.repair_auto).toBe(true);
    input.consoleDefinitions[1].feeds=[];
    expect(buildConsoleView('watch',input).boundaries || []).toEqual([]);
  });
});
