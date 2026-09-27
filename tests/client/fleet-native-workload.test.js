import { describe, expect, it } from 'vitest';
import vm from 'node:vm';
import { paneWorkloadScript } from '../../scripts/fleet-native-workload.mjs';

describe('native Station workload driver', () => {
  it('waits for its own seat and readiness before issuing correlated ordinary commands', () => {
    const sent = [], reports = [];
    let drive;
    const win = {__phoenixPane:{name:'matrix-captain',token:'private-token'},__phoenixPaneApply:() => 'forwarded',
      phoenixLink:{send:(...args) => sent.push(args)}};
    const context = vm.createContext({window:win,navigator:{userAgent:'test'},Set,JSON,
      setInterval:callback => {drive = callback;},
      fetch:url => {reports.push(JSON.parse(new URL(url).searchParams.get('event')));return Promise.resolve();}});
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
