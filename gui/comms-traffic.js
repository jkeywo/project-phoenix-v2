const keys = ['channels', 'priority', 'simultaneous_channels', 'monitored', 'observed_tick', 'report_supported', 'last_report', 'controllable'];
const closed = (value, names) => value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).every(key => names.includes(key));
const tick = value => Number.isSafeInteger(value) && value >= 0;
const channel = value => typeof value === 'string' && /^[a-z0-9-]{1,128}$/.test(value);
export const sameOrder = (a, b) => Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((value, index) => value === b[index]);
export function exactOrder(order, channels) {
  return Array.isArray(order) && Array.isArray(channels) && order.length === channels.length
    && new Set(order).size === order.length && order.every(value => channel(value) && channels.includes(value));
}
export function trafficPayload(raw, system, controllable = false) {
  if (!closed(raw, keys) || typeof system !== 'string' || !system || !Array.isArray(raw.channels)
    || !raw.channels.length || raw.channels.length > 65535 || !exactOrder(raw.channels, raw.channels)
    || !Array.isArray(raw.priority) || (raw.priority.length && !exactOrder(raw.priority, raw.channels))
    || !Number.isSafeInteger(raw.simultaneous_channels) || raw.simultaneous_channels < 1 || raw.simultaneous_channels > raw.channels.length
    || (!raw.priority.length && raw.simultaneous_channels !== raw.channels.length)
    || typeof raw.report_supported !== 'boolean' || !Array.isArray(raw.monitored)
    || (raw.observed_tick !== null && !tick(raw.observed_tick))) return null;
  const intended = (raw.priority.length ? raw.priority : raw.channels).slice(0, raw.simultaneous_channels);
  if (raw.monitored.length && (!sameOrder(raw.monitored, intended) || raw.observed_tick === null)) return null;
  const last = raw.last_report;
  if (last !== null && (!closed(last, ['semantic', 'emitted_tick']) || !['routine', 'red_alert'].includes(last.semantic) || !tick(last.emitted_tick))) return null;
  return {system, controllable: controllable === true, channels: [...raw.channels], priority: [...raw.priority],
    simultaneous_channels: raw.simultaneous_channels, monitored: [...raw.monitored], observed_tick: raw.observed_tick,
    report_supported: raw.report_supported, last_report: last === null ? null : {semantic: last.semantic, emitted_tick: last.emitted_tick}};
}
