import { t, wireText } from '../strings.js';
import { RADAR_SHARING_ACTION_ID } from '../stations/radar-sharing-actions.js';

/** Persistent controls for actual radar owners; a foreign feed stays information. */
export function mountRadarSharingControls(doc) {
  const node = doc.createElement('section'), rows = new Map(); let disposed=false; node.dataset.radarSharing = '';
  const title = doc.createElement('h4'); title.textContent = t('sensing.sharing.title'); node.append(title);
  return { node, update(values) {
    if (disposed) return;
    const present = new Set();
    for (const value of values || []) {
      if (!value?.owner || !['sensor_radar','tactical_radar'].includes(value.role)) continue;
      present.add(value.owner); let row = rows.get(value.owner);
      if (!row) {
        const fieldset = doc.createElement('fieldset'), legend = doc.createElement('legend'), summary = doc.createElement('p');
        const inputs = {};
        fieldset.append(legend);
        for (const [key,label] of [['share_own_faction','share'],['receive','receive']]) {
          const text = doc.createElement('label'), control = doc.createElement('input'); control.type = 'checkbox'; control.dataset.sharingIntent = key;
          text.append(control,doc.createTextNode(t('sensing.sharing.'+label))); fieldset.append(text); inputs[key] = control;
          control.addEventListener('keydown', event => { if (!['Tab','Escape'].includes(event.key)) event.stopPropagation(); });
          control.addEventListener('change', () => {
            const current = row.value;
            if (disposed || !current?.controllable || !current.can_operate || control.disabled) return;
            doc.defaultView?.activateSemanticAction?.(RADAR_SHARING_ACTION_ID, { source:'control',
              context:current.role==='tactical_radar'?'tactical':'sensors', detail:{owner:current.owner,
                share_own_faction:inputs.share_own_faction.checked,receive:inputs.receive.checked} });
          });
        }
        const hint = doc.createElement('p'); hint.textContent = t('sensing.sharing.hint'); fieldset.append(summary,hint); node.append(fieldset);
        row = {fieldset,legend,summary,inputs,value:null}; rows.set(value.owner,row);
      }
      row.value = value; row.legend.textContent = wireText(value.owner);
      for (const key of ['share_own_faction','receive']) {
        row.inputs[key].checked = value[key] === true; row.inputs[key].disabled = !value.controllable || !value.can_operate;
        row.inputs[key].parentElement.hidden = !value.controllable;
      }
      row.summary.textContent = t('sensing.sharing.summary',{share:t(value.share_own_faction?'sensing.sharing.on':'sensing.sharing.off'),
        receive:t(value.receive?'sensing.sharing.on':'sensing.sharing.off'),count:Number.isSafeInteger(value.received_samples)?value.received_samples:0});
    }
    for (const [owner,row] of rows) if (!present.has(owner)) { row.value=null;row.fieldset.remove(); rows.delete(owner); }
    node.hidden = rows.size===0;
  }, dispose() { disposed=true;node.remove();for(const row of rows.values())row.value=null;rows.clear(); } };
}
