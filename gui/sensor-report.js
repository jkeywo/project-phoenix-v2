import { has, t } from './strings.js';

const text = value => has(value) ? t(value) : value;

/** One live observation label. It has no event queue, replay or GM metadata. */
export function sensorReportText(report) {
  if (!report || typeof report.name !== 'string' || typeof report.source !== 'string'
    || !Number.isSafeInteger(report.observed_tick) || report.observed_tick < 0
    || !Number.isSafeInteger(report.age_ticks) || report.age_ticks < 0) return '';
  return t('console.sensors.report_label', { name: text(report.name), source: text(report.source),
    age: report.age_ticks, tick: report.observed_tick });
}

export function renderSensorReport(element, report) {
  if (!element) return;
  const value = sensorReportText(report);
  if (element.textContent !== value) element.textContent = value;
  if (element.hidden !== !value) element.hidden = !value;
}

/** Browser and native Viewscreen use this same selected-contact overlay. Age
 * changes are readable without announcing every simulation tick. */
export function mountSensorReport(doc = document) {
  const element = doc.createElement('p');
  element.id = 'viewscreen-sensor-report';
  element.hidden = true;
  element.style.cssText = 'position:fixed;left:5%;bottom:7rem;max-width:90%;box-sizing:border-box;margin:0;padding:.6rem .9rem;color:var(--ink,#e5e7eb);background:var(--bg-card,#111827);border:1px solid var(--line-faint,#526178);font:inherit;overflow-wrap:anywhere;z-index:20;pointer-events:none';
  doc.body.append(element);
  return report => renderSensorReport(element, report);
}
