import '../strings-boot.js';
import {t, wireText} from '../strings.js';
import {PhElement, phDefine} from './ph-element.js';
import {COMMS_SEND_TEXT_ACTION_ID, COMMS_SEND_REQUEST_ACTION_ID} from '../stations/comms-actions.js';

/** Local draft only. The inbox is the sole owner of sent/received facts. */
export class PhCommsTextComposer extends PhElement {
  template() {
    return `<style>:host{display:block}fieldset{border:1px solid var(--edge);margin:0;padding:6px;display:flex;gap:6px;flex-wrap:wrap}label{display:flex;gap:4px;align-items:center}textarea{flex:1 1 100%;min-height:44px;max-height:100px;resize:vertical}select{max-width:180px}button{min-height:44px}</style>
      <p id="interference" role="status" hidden>${t('jamming.receiver_interference')}</p>
      <fieldset id="controls"><legend>${t('comms.radio.title')}</legend>
      <label>${t('comms.radio.channel')}<select id="channel"></select></label>
      <label>${t('comms.radio.audience')}<select id="audience"></select></label>
      <textarea id="body" aria-label="${t('comms.radio.body')}" maxlength="4096"></textarea>
      <button id="send" type="button">${t('comms.send_text')}</button>
      <button id="request" type="button">${t('comms.request.report_status')}</button>
      <button id="audio-call" type="button" hidden>${t('comms.request.media_audio')}</button>
      <button id="video-call" type="button" hidden>${t('comms.request.media_video')}</button></fieldset>
      <details><summary>${t('comms.request.form_up')}</summary>
      <fieldset id="formation-controls"><legend>${t('comms.request.formation_slot_title')}</legend>
      <label>X<input id="slot-x" type="number" step="1" min="-1000000" max="1000000" value="-40"></label>
      <label>Z<input id="slot-z" type="number" step="1" min="-1000000" max="1000000" value="0"></label>
      <button id="form-up" type="button">${t('comms.request.form_up')}</button></fieldset></details>
      <fieldset id="planned-controls" hidden><legend>${t('comms.request.planned_title')}</legend>
      <label>${t('comms.request.planned_choice')}<select id="planned"></select></label>
      <button id="send-planned" type="button">${t('comms.request.report_status')}</button></fieldset>
      <label><input id="cleared" type="checkbox">${t('comms.request.show_cleared')}</label>`;
  }
  onTemplate() {
    this.channel = this.$('channel'); this.audience = this.$('audience');
    this.body = this.$('body'); this.controls = this.$('controls'); this.pending = null;
    this.audience.addEventListener('change', () => {if (this.state) this.render(this.state);});
    this.$('send').addEventListener('click', () => this.submit());
    this.$('request').addEventListener('click', () => {
      if (!this.controls.disabled) window.activateSemanticAction?.(COMMS_SEND_REQUEST_ACTION_ID,
        {source:'control', detail:{...this.draft(), request:'report_status'}});
    });
    for (const [id, mode] of [['audio-call', 'audio'], ['video-call', 'audio_video']]) {
      this.$(id).addEventListener('click', () => {
        if (this.$(id).disabled || this.$(id).hidden) return;
        window.activateSemanticAction?.(COMMS_SEND_REQUEST_ACTION_ID,
          {source:'control', detail:{...this.draft(), request:{media_call:{mode}}}});
      });
    }
    this.$('form-up').addEventListener('click', () => {
      if (this.$('formation-controls').disabled) return;
      const draft = this.draft();
      window.activateSemanticAction?.(COMMS_SEND_REQUEST_ACTION_ID, {source:'control',detail:{
        channel:draft.channel,audience:draft.audience,
        request:{form_up:{slot:[this.$('slot-x').valueAsNumber,this.$('slot-z').valueAsNumber]}},
      }});
    });
    this.audience.addEventListener('change', () => this.render(this.state));
    this.$('send-planned').addEventListener('click', () => {
      if (this.$('planned-controls').disabled) return;
      const row = (this.state?.planned_requests || []).find(row => row.id === this.$('planned').value);
      if (row) window.activateSemanticAction?.(COMMS_SEND_REQUEST_ACTION_ID, {source:'control', detail:{
        channel:row.channel, audience:{kind:'contact',target_uuid:row.recipient_uuid}, request:row.request, planned_request:row.id,
      }});
    });
    this.$('cleared').addEventListener('change', () => this.dispatchEvent(new CustomEvent('comms-history-change',
      {bubbles:true, detail:{show:this.$('cleared').checked}})));
    this.addEventListener('keydown', event => {
      // Text entry never leaks movement/Station shortcuts to the document.
      if (event.composedPath().includes(this.body)) {
        event.stopPropagation();
        if (event.code === 'Enter' && event.ctrlKey) { event.preventDefault(); this.submit(); }
      }
    });
  }
  options(select, entries) {
    const key = JSON.stringify(entries);
    if (select.dataset.options === key) return;
    const previous = select.value;
    select.replaceChildren(...entries.map(([value, label]) => {
      const option = document.createElement('option'); option.value = value; option.textContent = label; return option;
    }));
    if (entries.some(([value]) => value === previous)) select.value = previous;
    select.dataset.options = key;
  }
  draft() {
    const target = this.audience.value;
    return {channel: this.channel.value, body: this.body.value,
      audience: target.startsWith('contact:') ? {kind:'contact', target_uuid:target.slice(8)} : {kind:target}};
  }
  submit() {
    if (this.controls.disabled || !this.body.value.trim()) return;
    const draft = this.draft();
    const result = window.activateSemanticAction?.(COMMS_SEND_TEXT_ACTION_ID, {source:'control', detail:draft});
    if (result?.handled) this.pending = {draft:JSON.stringify(draft), ids:new Set((this.state?.messages || []).map(message => message.id))};
  }
  render(state) {
    if (!state) return;
    this.$('interference').hidden = state.receiver_interference !== true;
    this.options(this.channel, (state.channels || []).map(channel => [channel.id, channel.id]));
    this.options(this.audience, [
      ['fleet', t('comms.radio.fleet')], ['all', t('comms.radio.all')],
      ...(state.contacts || []).filter(contact => contact.uuid || contact.id)
        .map(contact => [`contact:${contact.uuid || contact.id}`, contact.name || contact.uuid || contact.id]),
    ]);
    this.controls.disabled = state.comms_auto || state.comms_controllable === false || !state.channels?.length;
    for (const [id, capable] of [['audio-call', ['audio', 'audio_video'].includes(state.media_capability)],
      ['video-call', state.media_capability === 'audio_video']]) {
      this.$(id).hidden = !capable;
      this.$(id).disabled = this.controls.disabled || !this.audience.value.startsWith('contact:');
    }
    this.$('formation-controls').disabled = this.controls.disabled || !this.audience.value.startsWith('contact:');
    const planned = (state.planned_requests || []).filter(row => row.request === 'report_status'
      && (state.channels || []).some(channel => channel.id === row.channel)
      && (state.contacts || []).some(contact => (contact.uuid || contact.id) === row.recipient_uuid));
    this.options(this.$('planned'), planned.map(row => {
      const contact = state.contacts.find(contact => (contact.uuid || contact.id) === row.recipient_uuid);
      return [row.id, t('comms.request.planned_target', {target:wireText(contact.name || row.recipient_uuid)})];
    }));
    this.$('planned-controls').hidden = !planned.length;
    this.$('planned-controls').disabled = state.comms_auto || !planned.length;

    if (this.pending && JSON.stringify(this.draft()) === this.pending.draft) {
      const sent = (state.messages || []).some(message => !this.pending.ids.has(message.id)
        && message.transmission && message.transmission.received_tick == null
        && JSON.stringify({channel:message.transmission.channel, body:message.body, audience:message.transmission.audience}) === this.pending.draft);
      if (sent) { this.body.value = ''; this.pending = null; }
    }
  }
}
phDefine('ph-comms-text-composer', PhCommsTextComposer);
