import {t} from './strings.js';

export function structuredComms(message) {
  return ['request', 'outcome'].includes(message?.transmission?.content?.kind);
}
export function visibleCommsMessages(messages, showCleared = false) {
  return (Array.isArray(messages) ? messages : []).filter(message => showCleared || !message?.request_progress?.dismissed);
}
/** Render only original events on this Console's actual permitted copy. */
export function requestStatus(message) {
  const wire = message?.transmission;
  if (!structuredComms(message)) return '';
  const event = wire.content.kind === 'outcome' ? wire.content.outcome : null;
  const heard = event?.request?.heard_tick ?? wire.received_tick;
  const facts = event?.facts ?? message.request_progress?.facts;
  const parts = [];
  const request = normalizeCommsRequest(event?.request?.request ?? wire.content.request);
  if (request?.receive_craft) parts.push(t('comms.request.reception_claim'));
  if (facts?.reception) parts.push(t('comms.request.received_craft_at', {tick:facts.reception.handoff_tick,count:facts.reception.people?.length || 0}));
  if (request?.form_up) parts.push(t('comms.request.formation_slot', {x:request.form_up.slot[0],z:request.form_up.slot[1]}));
  if (Number.isSafeInteger(heard)) parts.push(t('comms.request.heard_at', {tick:heard}));
  if (Number.isSafeInteger(facts?.read_tick)) parts.push(t('comms.request.read_at', {tick:facts.read_tick}));
  if (facts?.decision) parts.push(t(facts.decision.choice === 'report'
    ? 'comms.request.reported_at' : facts.decision.choice === 'accept' ? 'comms.request.accepted_at' : 'comms.request.declined_at', {tick:facts.decision.tick}));
  if (facts?.report) parts.push(t(facts.report.available ? 'comms.request.available_at'
    : 'comms.request.unavailable_at', {tick:facts.report.observed_tick}));
  return parts.join(' · ');
}

/** Closed wire vocabulary; returns a fresh finite Formation value. */
export function normalizeCommsRequest(value) {
  if (value === 'report_status') return value;
  if (value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).join() === 'receive_craft') {
    const body=value.receive_craft;
    return body && typeof body === 'object' && !Array.isArray(body) && Object.keys(body).join()==='craft'
      && typeof body.craft==='string' && body.craft.trim() && body.craft.length<=128 && !/[\u0000-\u001f\u007f-\u009f]/u.test(body.craft)
      ? {receive_craft:{craft:body.craft}} : null;
  }
  if (value && typeof value === 'object' && !Array.isArray(value)
      && Object.keys(value).join() === 'media_call') {
    const body = value.media_call;
    return body && typeof body === 'object' && !Array.isArray(body)
      && Object.keys(body).join() === 'mode' && ['audio', 'audio_video'].includes(body.mode)
      ? {media_call: {mode: body.mode}} : null;
  }
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).join() !== 'form_up') return null;
  const body = value.form_up;
  if (!body || typeof body !== 'object' || Array.isArray(body) || Object.keys(body).join() !== 'slot'
    || !Array.isArray(body.slot) || body.slot.length !== 2
    || !body.slot.every(axis => Number.isInteger(axis) && Math.abs(axis) <= 1000000)) return null;
  return {form_up:{slot:[...body.slot]}};
}

/** UI candidacy from an actually heard own inbox copy, never remote viability. */
export function mediaCallOrigin(message) {
  const wire = message?.transmission;
  if (!Number.isSafeInteger(wire?.received_tick) || wire.received_tick < 0) return null;
  const outcome = wire.content?.kind === 'outcome' ? wire.content.outcome : null;
  const request = normalizeCommsRequest(outcome?.request?.request ?? wire.content?.request);
  const decision = outcome?.facts?.decision ?? message.request_progress?.facts?.decision;
  if (!request?.media_call || decision?.choice !== 'accept') return null;
  const origin = outcome?.request?.origin_id ?? wire.origin_id;
  return typeof origin === 'string' && origin.length > 0 && origin.length <= 128 ? origin : null;
}

/** Copy only the closed server-projected receiving order; never display its identifiers. */
export function normalizeCraftReception(value) {
  const r=value?.request;
  if (!value || Object.keys(value).sort().join()!=='decision_tick,request' || !r
      || Object.keys(r).sort().join()!=='emitted_tick,heard_tick,origin_id,recipient_ship,request,source_ship'
      || ![value.decision_tick,r.emitted_tick,r.heard_tick].every(n=>Number.isSafeInteger(n)&&n>=0)
      || r.emitted_tick>r.heard_tick || r.heard_tick>value.decision_tick
      || ![r.origin_id,r.source_ship,r.recipient_ship].every(s=>typeof s==='string'&&s.trim()&&s.length<=128&&!/[\u0000-\u001f\u007f-\u009f]/u.test(s))
      || r.source_ship===r.recipient_ship) return null;
  const request=normalizeCommsRequest(r.request);if(!request?.receive_craft)return null;
  return {request:{origin_id:r.origin_id,source_ship:r.source_ship,recipient_ship:r.recipient_ship,
    emitted_tick:r.emitted_tick,heard_tick:r.heard_tick,request},decision_tick:value.decision_tick};
}
