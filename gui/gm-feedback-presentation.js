import { gmActionEntryKey } from './gm-action-feedback.js';
import { GM_ACTION_REFUSAL_REASON_LABELS, LOCAL_INGRESS_REFUSAL } from './gm-action-reasons.js';

/** Common result identity; panels still validate and project their typed fields. */
export function isGmResultEnvelope(value) {
  return !!value && typeof value === 'object'
    && typeof value.operator_id === 'string' && value.operator_id.length > 0
    && typeof value.correlation === 'string' && value.correlation.length > 0
    && ['applied', 'no-op', 'refused'].includes(value.outcome)
    && Number.isSafeInteger(value.tick) && value.tick >= 0
    && (value.reason == null || typeof value.reason === 'string');
}

/** Identity and feed presentation, with action meaning retained by adapters. */
export function createGmFeedbackPresentation({ doc, log, rowClass, feed, t,
  getOperator, getOperatorName, reasonPrefix, paintResult, paintLocal,
  beforeRender = () => {}, onAppend = () => {},
}) {
  function operator() {
    try {
      const value = typeof getOperator === 'function' ? getOperator() : null;
      return value && typeof value.id === 'string' && value.id.length > 0 ? value : null;
    } catch (_) { return null; }
  }
  function operatorName(id) {
    try {
      const value = typeof getOperatorName === 'function' ? getOperatorName(id) : id;
      return typeof value === 'string' && value.length > 0 ? value : id;
    } catch (_) { return id; }
  }
  function refusalText(reason) {
    if (!reason) return t(`${reasonPrefix}.reason_unspecified`);
    if (reason === LOCAL_INGRESS_REFUSAL) return t('server.gm.session.reason.ingress_rejected');
    const labelId = GM_ACTION_REFUSAL_REASON_LABELS[reason];
    return labelId ? t(labelId) : t(`${reasonPrefix}.reason_unknown`, { reason });
  }
  function appendRow(operatorId, correlation) {
    if (!log) return null;
    const row = doc.createElement('li');
    row.className = rowClass;
    row.dataset.entryKey = gmActionEntryKey(operatorId, correlation);
    row.dataset.operatorId = operatorId;
    row.dataset.correlation = correlation;
    onAppend(row);
    log.appendChild(row);
    return row;
  }
  function renderLog() {
    if (!log) return;
    log.replaceChildren();
    beforeRender();
    for (const { kind, value } of feed.entries()) {
      if (kind === 'result') paintResult(value);
      else paintLocal(value, kind === 'pending' ? 'pending' : value.outcome,
        kind === 'pending' ? null : value.reason);
    }
  }
  return { operator, operatorName, refusalText, appendRow, renderLog };
}
