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
  if (Number.isSafeInteger(heard)) parts.push(t('comms.request.heard_at', {tick:heard}));
  if (Number.isSafeInteger(facts?.read_tick)) parts.push(t('comms.request.read_at', {tick:facts.read_tick}));
  if (facts?.decision) parts.push(t(facts.decision.choice === 'report'
    ? 'comms.request.reported_at' : 'comms.request.declined_at', {tick:facts.decision.tick}));
  if (facts?.report) parts.push(t(facts.report.available ? 'comms.request.available_at'
    : 'comms.request.unavailable_at', {tick:facts.report.observed_tick}));
  return parts.join(' · ');
}
