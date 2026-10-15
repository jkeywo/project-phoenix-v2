// @vitest-environment jsdom
import {describe, it, expect, vi, afterEach} from 'vitest';
import '../../gui/components/ph-comms-text-composer.js';
import '../../gui/components/ph-comms-current-message.js';
import {t} from '../../gui/strings.js';
import {createCommsActionRegistry, COMMS_SEND_TEXT_ACTION_ID} from '../../gui/stations/comms-actions.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';

const state = () => ({channels:[{id:'public'}], contacts:[{uuid:'peer', name:'Peer'}], messages:[], comms_auto:false});
function setup() {
  document.body.innerHTML = '<ph-comms-text-composer></ph-comms-text-composer>';
  const el = document.body.firstElementChild; el.state = state(); return el;
}
afterEach(() => {document.body.replaceChildren(); delete window.activateSemanticAction;});
describe('radio text command and local draft', () => {
  it('offers media only from actual published capability and one selected Contact', () => {
    const el = setup();
    const audio = el.shadowRoot.getElementById('audio-call'), video = el.shadowRoot.getElementById('video-call');
    expect(audio.hidden).toBe(true);
    el.state = {...state(), media_capability:'audio_video'};
    expect(audio.disabled).toBe(true);
    el.audience.value = 'contact:peer'; el.audience.dispatchEvent(new Event('change'));
    expect(audio.disabled).toBe(false); expect(video.hidden).toBe(false);
    window.activateSemanticAction = vi.fn(); audio.click(); video.click();
    expect(window.activateSemanticAction.mock.calls.map(args => args[1].detail.request))
      .toEqual([{media_call:{mode:'audio'}}, {media_call:{mode:'audio_video'}}]);
    el.state = {...state(), media_capability:'audio'};
    expect(video.hidden).toBe(true);
    el.state = {...state(), media_capability:'audio_video', comms_auto:true};
    expect(audio.disabled).toBe(true);
  });
  it('renders sent and received facts without interpreting message markup or claiming remote acceptance', () => {
    document.body.innerHTML='<ph-comms-current-message></ph-comms-current-message>';
    const message = {id:'sent', sender_name:'Source', body:'<b>literal text</b>', thread_id:'radio/public/all', responses:[], transmission:{received_tick:null}};
    const el=document.body.firstElementChild;
    el.state={thread:message, messages:[message]};
    expect(el.shadowRoot.querySelector('.radio-status').textContent.trim()).toBe(t('comms.radio.sent'));
    expect(el.shadowRoot.querySelector('.text').textContent).toBe(message.body);
    expect(el.shadowRoot.querySelector('.text b')).toBeNull();
    el.state={thread:{...message, id:'received'}, messages:[{...message, id:'received', transmission:{received_tick:10}}]};
    expect(el.shadowRoot.querySelector('.radio-status').textContent.trim()).toBe(t('comms.radio.received'));
  });
  it('submits exact literal text via a correlated owned channel and refuses stale/delegated/unknown targets', () => {
    let view = state(); const sendAction = vi.fn();
    const registry = createCommsActionRegistry({getState:()=>view, sendAction, actionFeedback:new ActionFeedbackLifecycle()});
    const detail = {channel:'public', body:'<script>literal</script>', audience:{kind:'contact', target_uuid:'peer'}};
    expect(registry.activate(COMMS_SEND_TEXT_ACTION_ID, {context:'comms', source:'control', detail}).handled).toBe(true);
    expect(sendAction).toHaveBeenCalledWith('send_comms_text', expect.objectContaining({...detail, correlation:expect.any(String)}));
    for (const invalid of [{...detail, channel:'unknown'}, {...detail, audience:{kind:'contact', target_uuid:'secret'}}, {...detail, body:'é'.repeat(2049)}]) {
      expect(registry.activate(COMMS_SEND_TEXT_ACTION_ID, {context:'comms', source:'control', detail:invalid}).handled).toBe(false);
    }
    view = {...view, comms_auto:true};
    expect(registry.activate(COMMS_SEND_TEXT_ACTION_ID, {context:'comms', source:'control', detail}).handled).toBe(false);
    expect(sendAction).toHaveBeenCalledTimes(1);
  });
  it('keeps drafts on invocation/refusal and clears only a matching new local emission', () => {
    const el = setup(); const body = el.shadowRoot.getElementById('body'); body.value = 'Ready';
    window.activateSemanticAction = vi.fn(()=>({handled:true})); el.submit();
    expect(body.value).toBe('Ready'); el.state = state(); expect(body.value).toBe('Ready');
    el.state = {...state(), messages:[{id:'received', body:'Ready', transmission:{channel:'public', audience:{kind:'fleet'}, received_tick:5}}]};
    expect(body.value).toBe('Ready');
    el.state = {...state(), messages:[{id:'sent', body:'Ready', transmission:{channel:'public', audience:{kind:'fleet'}, received_tick:null}}]};
    expect(body.value).toBe('');
  });
  it('preserves edited drafts and stable focused controls through snapshots; disables delegation', () => {
    const el = setup(); const body = el.shadowRoot.getElementById('body'); body.value='First';
    window.activateSemanticAction = vi.fn(()=>({handled:true})); el.submit(); body.value='Second'; body.focus();
    const channel = el.shadowRoot.getElementById('channel'); const option = channel.firstElementChild;
    el.state = {...state(), messages:[{id:'sent', body:'First', transmission:{channel:'public', audience:{kind:'fleet'}}}]};
    expect(body.value).toBe('Second'); expect(el.shadowRoot.activeElement).toBe(body); expect(channel.firstElementChild).toBe(option);
    el.state = {...state(), comms_auto:true}; expect(el.shadowRoot.getElementById('controls').disabled).toBe(true);
  });
});

it('shows only the received own interference notice and clears it when unavailable',()=>{
 const el=setup(),notice=el.shadowRoot.getElementById('interference');
 expect(notice.hidden).toBe(true);el.state={...state(),receiver_interference:true};
 expect(notice.hidden).toBe(false);expect(notice.textContent).toBe(t('jamming.receiver_interference'));
 for(const receiver_interference of [false,null,undefined]){el.state={...state(),receiver_interference};expect(notice.hidden).toBe(true);}
 expect(el.shadowRoot.textContent).not.toContain('jam-source');
});
