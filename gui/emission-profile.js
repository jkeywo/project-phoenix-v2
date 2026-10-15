const object=value=>value&&typeof value==='object'&&!Array.isArray(value);
export const emissionIntents=Object.freeze(['normal','low_emission']);
export function emissionIntentPayload(raw,owner,authorised=false) {
  if(!object(raw)||typeof owner!=='string'||!owner||raw.owner!==owner||!emissionIntents.includes(raw.intent)||typeof raw.controllable!=='boolean')return null;
  return {owner,intent:raw.intent,controllable:authorised&&raw.controllable};
}
export function emissionAdvisoryPayload(raw) {
  const rate=value=>typeof value==='string'&&value.length<=64&&/^(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(value)&&Number.isFinite(Number(value));
  if(!object(raw)||!Number.isSafeInteger(raw.observed_tick)||raw.observed_tick<0||!rate(raw.rate)
    ||!rate(raw.advisory_rate)||Number(raw.advisory_rate)<=0||typeof raw.high!=='boolean'
    ||!['unclassified','reactor','propulsion','weapon','active_scan','comms'].includes(raw.cause))return null;
  return {observed_tick:raw.observed_tick,rate:raw.rate,cause:raw.cause,advisory_rate:raw.advisory_rate,high:raw.high};
}
