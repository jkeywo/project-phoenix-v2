import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { createNativeFleetPeer } from '../../gui/native-fleet-peer.js';

const bounds = {maxBytes:32768,maxBufferedBytes:65536};
function rig(sendSnapshot = vi.fn(() => true), nowMs) {
  let options;
  const send = vi.fn();
  const handle = {close:vi.fn(),sendMedia:vi.fn(() => true)};
  const peer = createNativeFleetPeer({send,sendSnapshot,...(nowMs?{nowMs}:{}),createOwner:opts => {options=opts;return handle;}});
  peer.configure({base:'https://fleet.test',credentials:['one']});
  const socket = options.factories.socket();
  const receive = (generation,event,extra={}) => peer.receive(JSON.stringify({native_wire:{generation,event,next_sequence:1,...extra}}));
  return {peer,socket,send,sendSnapshot,receive,options,handle};
}
describe('native fleet drop-only admission',()=>{
  it('refuses an unadvertised/old path and requires actual generation readiness',async()=>{
    const {socket,receive,sendSnapshot}=rig();
    await Promise.resolve();
    expect(socket.bufferedAmount).toBe(Infinity);
    expect(socket.trySendSnapshot('frame',bounds)).toBe(false);
    receive(0,'snapshot-receipt',{sequence:null,supported:true});
    receive(1,'snapshot-ready',{supported:true});
    expect(socket.trySendSnapshot('frame',bounds)).toBe(false);
    receive(0,'snapshot-ready',{supported:false});
    expect(socket.trySendSnapshot('frame',bounds)).toBe(false);
    receive(0,'snapshot-ready',{supported:true});
    expect(socket.trySendSnapshot('frame',bounds)).toBe(true);
    expect(sendSnapshot).toHaveBeenCalledExactlyOnceWith({kind:'fleet_wire_snapshot',generation:0,sequence:1,frame:'frame',max_buffered_bytes:65536});
  });
  it('retains one admitted frame until matching attempt receipt, never retries',async()=>{
    const {socket,receive,sendSnapshot,send}=rig();await Promise.resolve();
    receive(0,'snapshot-ready',{supported:true});
    expect(socket.trySendSnapshot('first',bounds)).toBe(true);
    expect(socket.trySendSnapshot('second',bounds)).toBe(false);
    receive(0,'snapshot-receipt',{sequence:2,supported:true});
    expect(socket.trySendSnapshot('second',bounds)).toBe(false);
    socket.send('ordinary command');socket.send('ordinary gameplay snapshot');
    expect(send.mock.calls.slice(-2).map(([row])=>row.frame)).toEqual(['ordinary command','ordinary gameplay snapshot']);
    receive(0,'snapshot-receipt',{sequence:1,accepted:false,supported:true});
    expect(sendSnapshot).toHaveBeenCalledTimes(1);
    expect(socket.trySendSnapshot('second',bounds)).toBe(true);
    expect(sendSnapshot).toHaveBeenCalledTimes(2);
    socket.close();receive(0,'snapshot-receipt',{sequence:2,supported:true});
    expect(socket.trySendSnapshot('late',bounds)).toBe(false);
  });
  it('refuses failing/full staging and complete UTF8 wire bounds before sending',async()=>{
    const {socket,receive,sendSnapshot}=rig(vi.fn(()=>false));await Promise.resolve();
    receive(0,'snapshot-ready',{supported:true});
    expect(socket.trySendSnapshot('local drop',bounds)).toBe(false);
    expect(socket.trySendSnapshot('x'.repeat(32769),bounds)).toBe(false);
    expect(socket.trySendSnapshot('😀'.repeat(9000),bounds)).toBe(false);
    expect(socket.trySendSnapshot('wire envelope',{maxBytes:4,maxBufferedBytes:65536})).toBe(false);
    expect(socket.trySendSnapshot('wire envelope',{maxBytes:32768,maxBufferedBytes:4})).toBe(false);
    expect(sendSnapshot).toHaveBeenCalledTimes(1);
  });
  it('an old bridge or a late receipt cannot grant the replacement generation',async()=>{
    const {socket,receive,options}=rig(undefined);await Promise.resolve();
    socket.close();
    const replacement=options.factories.socket('wss://fleet.test/v1/host');
    receive(1,'open');
    receive(0,'snapshot-ready',{supported:true});
    receive(0,'snapshot-receipt',{sequence:1,supported:true});
    expect(replacement.trySendSnapshot('late',bounds)).toBe(false);
    let oldOptions;
    const old=createNativeFleetPeer({send:vi.fn(),createOwner:value=>{oldOptions=value;return {close(){}};}});
    old.configure({base:'https://fleet.test',credentials:['one']});
    const oldSocket=oldOptions.factories.socket();await Promise.resolve();
    old.receive(JSON.stringify({native_wire:{generation:0,event:'snapshot-ready',supported:true,next_sequence:1}}));
    expect(oldSocket.trySendSnapshot('unimplemented provider',bounds)).toBe(false);
  });
  it('expires a downstream-dropped admission without retry and ignores its late receipt',async()=>{
    let now=0;
    const {socket,receive,sendSnapshot}=rig(vi.fn(()=>true),()=>now);await Promise.resolve();
    receive(0,'snapshot-ready',{supported:true});
    expect(socket.trySendSnapshot('dropped downstream',bounds)).toBe(true);
    // Native downstream staging can discard this local admission without a receipt.
    now=249;expect(socket.trySendSnapshot('future',bounds)).toBe(false);
    now=250;expect(socket.trySendSnapshot('future',bounds)).toBe(true);
    expect(sendSnapshot.mock.calls.map(([row])=>[row.sequence,row.frame])).toEqual([[1,'dropped downstream'],[2,'future']]);
    receive(0,'snapshot-receipt',{sequence:1,supported:true,next_sequence:2});
    expect(socket.trySendSnapshot('must remain busy',bounds)).toBe(false);
    receive(0,'snapshot-receipt',{sequence:2,accepted:false,supported:true,next_sequence:3});
    expect(socket.trySendSnapshot('next packet',bounds)).toBe(true);
  });
  it('recreates page-side sequence above the actual native high-water floor',async()=>{
    const first=rig();await Promise.resolve();
    first.receive(0,'snapshot-ready',{supported:true,next_sequence:41});
    expect(first.socket.trySendSnapshot('before page rebuild',bounds)).toBe(true);
    expect(first.sendSnapshot.mock.calls[0][0].sequence).toBe(41);
    const rebuilt=rig();await Promise.resolve();
    rebuilt.peer.receive(JSON.stringify({native_wire:{generation:0,event:'snapshot-ready',supported:true}}));
    expect(rebuilt.socket.trySendSnapshot('old capability',bounds)).toBe(false);
    rebuilt.receive(0,'snapshot-ready',{supported:true,next_sequence:42});
    expect(rebuilt.socket.trySendSnapshot('after page rebuild',bounds)).toBe(true);
    expect(rebuilt.sendSnapshot.mock.calls[0][0].sequence).toBe(42);
    rebuilt.receive(0,'snapshot-receipt',{sequence:41,supported:true,next_sequence:42});
    expect(rebuilt.socket.trySendSnapshot('stale release',bounds)).toBe(false);
  });
  it('uses the bounded ephemeral callback for authenticated media and never the command callback',()=>{
    const {peer,options,send,sendSnapshot,handle}=rig();
    expect(options.onMediaFrame('packet',3)).toBe(true);
    expect(sendSnapshot).toHaveBeenCalledWith({kind:'fleet_media_frame',frame:'packet',authenticated_slot:3});
    expect(options.onMediaFrame('x'.repeat(20481),3)).toBe(false);
    expect(options.onMediaFrame('packet',0)).toBe(false);
    expect(peer.sendMedia('packet')).toBe(true);
    expect(handle.sendMedia).toHaveBeenCalledExactlyOnceWith('packet');
    expect(send).not.toHaveBeenCalledWith(expect.objectContaining({kind:'fleet_frame'}));
  });
});

it('native bridge staging is bounded, non-evicting, and drains after ordinary records',()=>{
  const ordinary=[];
  const window={};
  window.__phoenixHostLobbyOutDrain=()=>ordinary.splice(0).join('\n');
  new Function('window',readFileSync(new URL('../../src/native_host/host_lobby/host_lobby_boot.js',import.meta.url),'utf8'))(window);
  const offer=window.__phoenixHostFleetSnapshotSend;
  const record={kind:'fleet_wire_snapshot',frame:'kept',generation:0,sequence:1,max_buffered_bytes:65536};
  expect(offer({kind:'ordinary',frame:'wrong lane'})).toBe(false);
  expect(offer({...record,frame:'"'.repeat(40000)})).toBe(false);
  for(let i=0;i<4;i++)expect(offer({...record,sequence:i+1})).toBe(true);
  expect(offer({...record,frame:'would evict'})).toBe(false);
  ordinary.push('command','gameplay snapshot');
  const rows=window.__phoenixHostLobbyOutDrain().split('\n');
  expect(rows.slice(0,2)).toEqual(['command','gameplay snapshot']);
  expect(rows.slice(2).map(JSON.parse).map(row=>row.frame)).toEqual(['kept','kept','kept','kept']);
  expect(window.__phoenixHostLobbyOutDrain()).toBe('');
  expect(offer(record)).toBe(true);
  window.__phoenixHostLobbyOutDrain();
  const utf8={...record,frame:'😀'.repeat(10000)};
  for(let i=0;i<3;i++)expect(offer(utf8)).toBe(true);
  expect(offer(utf8)).toBe(false); // byte bound, before the four-record count cap
  expect(window.__phoenixHostLobbyOutDrain().split('\n')).toHaveLength(3);
});
