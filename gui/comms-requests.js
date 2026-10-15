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
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).join() !== 'form_up') return null;
  const body = value.form_up;
  if (!body || typeof body !== 'object' || Array.isArray(body) || Object.keys(body).join() !== 'slot'
    || !Array.isArray(body.slot) || body.slot.length !== 2
    || !body.slot.every(axis => Number.isInteger(axis) && Math.abs(axis) <= 1000000)) return null;
  return {form_up:{slot:[...body.slot]}};
}
