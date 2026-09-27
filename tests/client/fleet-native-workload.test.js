import { describe, expect, it } from 'vitest';
import vm from 'node:vm';
import { paneWorkloadScript, instrumentNativeGmModule, nativeObserverReporterScript } from '../../scripts/fleet-native-workload.mjs';

describe('native Station workload driver', () => {
  it('waits for its own seat and readiness before issuing correlated ordinary commands', () => {
    const sent = [], reports = [];
    let drive;
    const win = {__phoenixPane:{name:'matrix-captain',token:'private-token'},__phoenixPaneApply:() => 'forwarded',
      phoenixLink:{send:(...args) => sent.push(args)}};
    const context = vm.createContext({window:win,navigator:{userAgent:'test'},Set,JSON,
      setInterval:callback => {drive = callback;},
      fetch:url => {reports.push(JSON.parse(new URL(url).searchParams.get('event')));return Promise.resolve({ok:true});}});
    vm.runInContext(paneWorkloadScript('http://localhost/token'),context);
    const inbound = (type,data={}) => win.__phoenixPaneApply(JSON.stringify({type,data}));
    drive(); expect(sent).toHaveLength(0);
    expect(inbound('Welcome')).toBe('forwarded'); drive();
    expect(sent).toEqual([['SelectStation',{station:'captain'},'reliable']]);
    inbound('StationAssigned',{token:'someone-else',station_id:'captain'}); drive(); expect(sent).toHaveLength(1);
    inbound('StationAssigned',{token:'private-token',station_id:'captain'}); drive();
    expect(sent.at(-1)).toEqual(['SetReady',{ready:true},'reliable']);
    inbound('GameStarted'); drive(); expect(sent).toHaveLength(2);
    inbound('ReadyChanged',{token:'private-token',ready:true}); drive();
    expect(sent.at(-1)).toEqual(['ControlSystemCorrelated',{
      correlation:'native-matrix-captain-1',target:'red-alert',payload:{type:'SetRedAlert',data:{active:false}},
    },'reliable']);
    inbound('ActionFeedback',{correlation:'native-matrix-captain-1',outcome:'Applied'});
    expect(reports.at(-1)).toEqual({kind:'station-feedback',value:{station:'captain',correlation:'native-matrix-captain-1',outcome:'Applied'}});
    expect(JSON.stringify(reports)).not.toContain('private-token');
  });
});


describe('bounded native telemetry and coordinated GM readiness', () => {
  it('turns a rejected telemetry HTTP response into one terminal observer error', async () => {
    const reports = [];
    const context = vm.createContext({fetch:url => {
      const event = JSON.parse(new URL(url).searchParams.get('event')); reports.push(event);
      return Promise.resolve({ok:event.kind === 'observer-error',status:431});
    }});
    vm.runInContext(nativeObserverReporterScript('http://localhost/token') + "; report('sample',{});",context);
    await new Promise(resolve => setImmediate(resolve));
    vm.runInContext("report('late',{});",context);
    expect(reports).toEqual([{kind:'sample',value:{}},{kind:'observer-error',value:{reason:'http-status-431'}}]);
  });

  it('signals oversize data before sending it rather than relying on the HTTP parser', () => {
    const reports = [];
    const context = vm.createContext({fetch:url => {reports.push(JSON.parse(new URL(url).searchParams.get('event')));return Promise.resolve({ok:true});}});
    vm.runInContext(nativeObserverReporterScript('http://localhost/token') + "; report('large','x'.repeat(61000)); report('late',{});",context);
    expect(reports).toEqual([{kind:'observer-overflow',value:{reason:'event-size'}}]);
  });

  it('defers Ready until commanded and connected, and records each GM action once without a growing journal', async () => {
    const reports = [], controls = [{kind:'ready'}], readyCalls = [];
    let subscriber, drive, operator = null;
    const bridge = {subscribe:callback => {subscriber=callback;},getOperator:()=>operator,setReady:value=>readyCalls.push(value)};
    const context = vm.createContext({window:{},navigator:{userAgent:'test'},setInterval:callback=>{drive=callback;},fetch:url=>{
      if(url.endsWith('/control')) return Promise.resolve({ok:true,json:async()=>controls.shift()||null});
      reports.push(JSON.parse(new URL(url).searchParams.get('event')));return Promise.resolve({ok:true});
    }});
    const source='export function mountNativeGmWorkspace(options) {return {unchanged:true};}';
    vm.runInContext(instrumentNativeGmModule(source,'http://localhost/token',{deferReady:true}).replace('export function mountNativeGmWorkspace','function mountNativeGmWorkspace'),context);
    context.mountNativeGmWorkspace({bridge});
    subscriber('metadata',{phase:'Lobby',local_operator_id:null,gms:[]});
    expect(readyCalls).toEqual([]);
    await drive(); // Permission persists while no admitted operator exists.
    expect(readyCalls).toEqual([]);
    operator={id:'gm-2',connected:true,ready:false};
    subscriber('metadata',{phase:'Lobby',local_operator_id:'gm-2',gms:[operator]});
    expect(readyCalls).toEqual([true]);
    operator={...operator,ready:true};
    subscriber('metadata',{phase:'Lobby',local_operator_id:'gm-2',gms:[operator]});
    await drive();
    expect(readyCalls).toEqual([true]);
    const entries=Array.from({length:70},(_,i)=>({detail:{type:'gm_action',data:{operator:{id:'gm-2'},correlation:'action-'+i,outcome:'applied'}}}));
    for(let count=1;count<=70;count++) subscriber('gm_activity',{entries:entries.slice(0,count)});
    subscriber('gm_session',{paused:false,journal:{total:70,entries},factions:[{name:'Alliance',enemies:[]}],results:[]});
    expect(reports.filter(row=>row.kind==='gm-activity')).toHaveLength(70);
    expect(reports.filter(row=>row.kind==='gm-activity').every(row=>row.value.entries.length===1)).toBe(true);
    expect(reports.at(-1)).toEqual({kind:'gm-session',value:{paused:false,journalTotal:70,factions:[{name:'Alliance',enemies:[]}],results:[]}});
    expect(reports.some(row=>row.kind==='observer-overflow')).toBe(false);
  });
});
