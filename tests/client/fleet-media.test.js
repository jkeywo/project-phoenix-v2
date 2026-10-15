import {describe,it,expect} from 'vitest';
import {mediaHostFrame,createFleetMediaGate} from '../../gui/fleet-media.js';
import {HOST_MESH_PROTOCOL} from '../../gui/host-mesh.js';
const frame=()=>({m:HOST_MESH_PROTOCOL,t:'media',tick:null,d:{from:1,to:2,call_origin_id:'actually-accepted-call',generation:1}});
describe('ephemeral typed host media ingress',()=>{
  it('rejects wrong identities/versions, future ticks and oversized reordered envelopes',()=>{
    expect(mediaHostFrame(frame())).not.toBeNull();
    for(const value of [{...frame(),m:0},{...frame(),tick:1},{...frame(),d:{...frame().d,from:0}},{...frame(),d:{...frame().d,to:1}}])expect(mediaHostFrame(value)).toBeNull();
    expect(mediaHostFrame(JSON.stringify({...frame(),d:{...frame().d,pad:'x'.repeat(20481)}}))).toBeNull();
    expect(mediaHostFrame({...frame(),d:{...frame().d,generation_floor:7}})).not.toBeNull();
    expect(mediaHostFrame({...frame(),d:{...frame().d,generation_floor:0x100000000}})).toBeNull();
    expect(mediaHostFrame({...frame(),d:{...frame().d,generation_floor:7,packet:{}}})).toBeNull();
  });
  it('bounds before continuation staging against the receiver clock',()=>{
    let now=1;const admit=createFleetMediaGate(()=>now);for(let i=0;i<24;i++)expect(admit(frame())).not.toBeNull();
    expect(admit(frame())).toBeNull();now=1001;expect(admit(frame())).not.toBeNull();now=0;expect(admit(frame())).toBeNull();
  });
});
