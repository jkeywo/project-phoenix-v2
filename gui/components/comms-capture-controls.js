import { t, wireText } from '../strings.js';
import { CAPTURE_SELECT_ACTION } from '../stations/comms-capture-actions.js';
export function mountCommsCaptureControls(doc) {
  const node = doc.createElement('section'), title = doc.createElement('h4'), rows = new Map();
  title.textContent = t('comms.capture.title'); node.append(title); node.dataset.commsCapture = '';
  let current = null, disposed = false;
  const activate = capture => { if (!disposed && current?.controllable) doc.defaultView?.activateSemanticAction?.(
    CAPTURE_SELECT_ACTION.id, { source: 'control', context: 'comms-capture', detail: { capture } }); };
  return { node, update(view) {
    if (disposed) return; current = view; node.hidden = !view;
    const present = new Set();
    for (const value of view?.rows || []) {
      present.add(value.capture); let row = rows.get(value.capture);
      if (!row) {
        const box = doc.createElement('fieldset'), header = doc.createElement('legend'), body = doc.createElement('p'), button = doc.createElement('button');
        button.type = 'button'; box.dataset.capture = value.capture; box.append(header, body, button); node.append(box);
        row = { box, header, body, button, value: null }; rows.set(value.capture, row);
        button.addEventListener('click', () => { if (!button.disabled && row.value) activate(row.value.selected ? null : row.value.capture); });
      }
      row.value = value;
      row.header.textContent = t('comms.capture.header', { channel: value.channel, level: value.level, tick: value.captured_tick });
      row.body.textContent = value.evidence ? t('comms.capture.evidence', { sender: wireText(value.evidence.sender_name),
        body: value.evidence.literal_body ? value.evidence.body : wireText(value.evidence.body) })
        : t(value.feasible ? 'comms.capture.work' : 'comms.capture.unsupported', { progress: (value.progress_per_mille / 10).toFixed(1) });
      row.button.textContent = t(value.selected ? 'comms.capture.pause' : 'comms.capture.select');
      row.button.hidden = !view.controllable || value.evidence != null;
      row.button.disabled = !view.controllable || !value.feasible || value.evidence != null;
    }
    for (const [id, row] of rows) if (!present.has(id)) { row.value = null; row.button.disabled = true; row.box.remove(); rows.delete(id); }
  }, dispose() { disposed = true; current = null; for (const row of rows.values()) { row.value = null; row.button.disabled = true; } rows.clear(); node.remove(); } };
}
