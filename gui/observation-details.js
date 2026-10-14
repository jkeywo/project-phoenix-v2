import { t } from './strings.js';

const SOURCES = Object.freeze({ unknown: 'observation.source.unknown', scan: 'observation.source.scan', evidence: 'observation.source.evidence', sensors_report: 'observation.source.sensors_report', system_report: 'observation.source.system_report', repair_team: 'observation.source.repair_team', repair_diagnosis: 'observation.source.repair_diagnosis' });
const validTick = value => Number.isSafeInteger(value) && value >= 0;
/** A restored clock must not age a publication still held from a prior continuation. */
export function observationTickFor(state, source) {
  if (Number.isSafeInteger(state.observationGeneration)
    && state.blackboardPresentation?.[source] !== state.observationGeneration) return null;
  return state.observationTick ?? null;
}
/** Format only metadata delivered with this permitted record. Receipt never renews age. */
export function observationRows(metadata, now, visibleFields = {}) {
  const m = metadata || {}, provenance = m.provenance || {}, unknown = t('observation.unknown');
  const sample = validTick(m.observed_tick) ? m.observed_tick : null;
  const receipt = validTick(m.received_tick) ? m.received_tick : null;
  const source = [t(SOURCES[provenance.kind] || SOURCES.unknown), provenance.label ? t(provenance.label) : '', provenance.system || ''].filter(Boolean).join(' · ');
  let precision = unknown;
  if (m.uncertainty?.kind === 'qualitative') precision = t('observation.qualitative');
  else if (m.uncertainty?.kind === 'quantized') {
    const { field, step } = m.uncertainty;
    // Unknown/hidden fields cannot smuggle measurement names or precision into details.
    if (Object.hasOwn(visibleFields, field) && Number.isFinite(step) && step > 0) {
      precision = t('observation.quantized', { field: t(visibleFields[field]), step });
    }
  }
  return [
    ['source', t('observation.source'), source],
    ['sample', t('observation.sample'), sample === null ? unknown : t('observation.tick', { tick: sample })],
    ['receipt', t('observation.receipt'), receipt === null ? unknown : t('observation.tick', { tick: receipt })],
    ['age', t('observation.age'), sample !== null && validTick(now) ? t('observation.ticks', { ticks: Math.max(0, now - sample) }) : unknown],
    ['precision', t('observation.precision'), precision],
  ];
}

/** Native disclosure, safe text nodes, no commands, timers or extra information lookup. */
export function observationDetails(doc, metadata, now, visibleFields = {}) {
  const details = doc.createElement('details'); details.className = 'observation-details';
  const summary = doc.createElement('summary'); summary.textContent = t('observation.details'); details.append(summary);
  const list = doc.createElement('dl'); details.append(list);
  for (const [key, label, value] of observationRows(metadata, now, visibleFields)) {
    const term = doc.createElement('dt'), description = doc.createElement('dd');
    term.textContent = label; description.textContent = value; description.dataset.observation = key;
    list.append(term, description);
  }
  return details;
}
