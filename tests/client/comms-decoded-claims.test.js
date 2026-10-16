/** @vitest-environment jsdom */
import {expect,it,vi} from 'vitest';
import {decodedClaim,decodedClaimText} from '../../gui/comms-decoded-claims.js';
import {capturedPayload} from '../../gui/comms-capture.js';
import {mountCommsCaptureControls} from '../../gui/components/comms-capture-controls.js';
const outcome=()=>({kind:'outcome',outcome:{request:{kind:'report_status'},request_emitted_tick:2,heard_tick:3,
  read_tick:5,decision:{choice:'report',tick:6},report:{available:false,observed_tick:6}}});
it('projects only closed supported claim parameters and original transmitted events',()=>{
  for(const request of [{kind:'report_status'},{kind:'form_up',slot:[-15,22]},
    {kind:'media_call',mode:'audio_video'},{kind:'receive_craft'}]){
    const value={kind:'request',request};expect(decodedClaim(value,'request',12)).toEqual(value);
    expect(decodedClaim({...value,origin_id:'private'},'request',12)).toBeNull();
  }
  expect(decodedClaim(outcome(),'outcome',12)).toEqual(outcome());
  for(const change of [value=>value.outcome.report.system='private',value=>value.outcome.request_emitted_tick=4,
    value=>value.outcome.read_tick=2,value=>value.outcome.decision.tick=13,value=>value.outcome.report.observed_tick=7,
    value=>value.outcome.request={kind:'form_up',slot:[0,0]},value=>value.outcome.reception={job:'private'}]){
    const value=outcome();change(value);expect(decodedClaim(value,'outcome',12)).toBeNull();
  }
  expect(decodedClaim({kind:'request',request:{kind:'form_up',slot:[1e6+1,0]}},'request',12)).toBeNull();
  expect(decodedClaim({kind:'request',request:{kind:'receive_craft',craft:'private'}},'request',12)).toBeNull();
});
it('reveals typed details only with completed evidence and retains no private routing fields',()=>{
  const header={capture:1,channel:'fleet',level:1,captured_tick:12,feasible:true,selected:false,progress_per_mille:1000,decoded_tick:14};
  const evidence={sender_name:'Reported source',body:'comms.request.returned_outcome',literal_body:false,kind:'outcome',emitted_tick:12,claim:outcome()};
  const value={...header,evidence};
  expect(capturedPayload({rows:[{...value,decoded_tick:null}]},'comms')).toBeNull();
  expect(capturedPayload({rows:[{...header,decoded_tick:null,evidence:null,claim:outcome()}]},'comms').rows[0]).not.toHaveProperty('claim');
  const projected=capturedPayload({rows:[{...value,source:'private',audience:'private'}]},'comms');
  expect(projected.rows[0].evidence.claim).toEqual(outcome());expect(JSON.stringify(projected)).not.toContain('private');
  const mismatched={...evidence,kind:'request'};
  expect(capturedPayload({rows:[{...header,evidence:mismatched}]},'comms')).toBeNull();
});
it('renders foreign claims as read-only content with no reply, coordination or media action',()=>{
  const widget=mountCommsCaptureControls(document);window.activateSemanticAction=vi.fn();
  const claim=outcome(),text=decodedClaimText(claim);expect(text).toContain('Intercepted claim');expect(text).toContain('tick 6');
  widget.update({controllable:true,rows:[{capture:1,channel:'fleet',level:1,captured_tick:12,feasible:true,
    selected:false,progress_per_mille:1000,decoded_tick:14,evidence:{sender_name:'<img src=x>',body:'comms.request.returned_outcome',literal_body:false,kind:'outcome',emitted_tick:12,claim}}]});
  expect(widget.node.textContent).toContain('Comms unavailable');expect(widget.node.querySelector('img')).toBeNull();
  for(const button of widget.node.querySelectorAll('button')){expect(button.disabled).toBe(true);button.click();}
  expect(window.activateSemanticAction).not.toHaveBeenCalled();widget.dispose();delete window.activateSemanticAction;
});
