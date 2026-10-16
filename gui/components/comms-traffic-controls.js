import {t} from '../strings.js';
import {LISTEN_ORDER_ACTION, REPORT_ALERT_ACTION} from '../stations/comms-traffic-actions.js';
export function mountCommsTrafficControls(doc) {
  const node = doc.createElement('section'), title = doc.createElement('h4'), status = doc.createElement('p'), list = doc.createElement('ol'), report = doc.createElement('button');
  title.textContent = t('comms.traffic.title'); report.type = 'button'; report.textContent = t('comms.traffic.report'); node.append(title,status,list,report);
  let current = null, disposed = false, reportOwner = null;
  const rows = new Map();
  const signature = () => JSON.stringify([current?.system, current?.priority]);
  const emit = (action, detail) => { if (!disposed && current?.controllable) doc.defaultView?.activateSemanticAction?.(action.id, {context:'comms-traffic',source:'control',detail}); };
  const armReport = () => { reportOwner = current?.system; };
  report.addEventListener('mousedown',armReport); report.addEventListener('pointerdown',armReport);
  report.addEventListener('keydown',event => { if (event.key === 'Enter' || event.key === ' ') armReport(); });
  report.addEventListener('click', () => {
    const armed = reportOwner; reportOwner = null;
    if (!report.disabled && current?.report_supported && (armed === null || armed === current.system)) emit(REPORT_ALERT_ACTION, {system:current.system});
  });
  return {node, update(view) {
    if (disposed) return; current = view; node.hidden = !view;
    status.textContent = view ? t('comms.traffic.coverage', {count:view.simultaneous_channels, channels:view.monitored.join(', ') || t('comms.traffic.not_monitoring')}) : '';
    report.hidden = !view?.controllable; report.disabled = !view?.controllable || !view.report_supported;
    const order = view?.priority.length ? view.priority : view?.channels || [], present = new Set(order);
    for (const [channel,row] of rows) if (!present.has(channel)) { row.armed = null; row.up.disabled = row.down.disabled = true; row.node.remove(); rows.delete(channel); }
    for (const [index,channel] of order.entries()) {
      let row = rows.get(channel);
      if (!row) {
        const item = doc.createElement('li'), label = doc.createElement('span'), up = doc.createElement('button'), down = doc.createElement('button');
        up.type = down.type = 'button'; up.textContent = t('comms.traffic.up'); down.textContent = t('comms.traffic.down'); item.append(label,up,down);
        row = {node:item,label,up,down,armed:null}; rows.set(channel,row);
        for (const [button,offset] of [[up,-1],[down,1]]) {
          const arm = () => { row.armed = signature(); };
          button.addEventListener('pointerdown', arm); button.addEventListener('mousedown', arm); button.addEventListener('keydown', event => { if (event.key === 'Enter' || event.key === ' ') arm(); });
          button.addEventListener('click', () => {
            const armed = row.armed; row.armed = null;
            if (disposed || button.disabled || !current?.controllable || (armed !== null && armed !== signature())) return;
            const next = [...current.priority], from = next.indexOf(channel), to = from+offset;
            if (from < 0 || to < 0 || to >= next.length) return;
            [next[from],next[to]] = [next[to],next[from]];
            emit(LISTEN_ORDER_ACTION, {system:current.system, expected_priority:[...current.priority], priority:next});
          });
        }
      }
      row.label.textContent = channel;
      row.up.hidden = row.down.hidden = !view?.controllable || !view.priority.length;
      row.up.disabled = !view?.controllable || !view.priority.length || index === 0;
      row.down.disabled = !view?.controllable || !view.priority.length || index === order.length-1;
      if (list.children[index] !== row.node) list.insertBefore(row.node,list.children[index] || null);
    }
  }, dispose() { disposed = true; current = null; reportOwner = null; report.disabled = true; for (const row of rows.values()) { row.armed = null; row.up.disabled = row.down.disabled = true; } rows.clear(); node.remove(); }};
}
