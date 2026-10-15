import { t, wireText } from '../strings.js';
import { GUIDANCE_ACTION_IDS } from '../stations/guidance-actions.js';

/** Source launch/order knowledge only. Heard never means accepted or alive. */
export function mountGuidanceControls(doc) {
  const node = doc.createElement('fieldset'), title = doc.createElement('legend'), rounds = doc.createElement('select'), targets = doc.createElement('select');
  const roundLabel = doc.createElement('label'), targetLabel = doc.createElement('label'), status = doc.createElement('p'), intent = doc.createElement('p');
  title.textContent = t('guidance.title'); roundLabel.textContent = t('guidance.round'); targetLabel.textContent = t('guidance.target');
  roundLabel.append(rounds); targetLabel.append(targets); node.dataset.guidance = '';
  const apply = doc.createElement('button'), stop = doc.createElement('button'); apply.type = stop.type = 'button';
  apply.textContent = t('guidance.send'); stop.textContent = t('guidance.stop');
  node.append(title, roundLabel, status, intent, targetLabel, apply, stop);
  let value = null, owner = null, readonly = true;
  const selected = () => value?.rounds?.find(row => row.round_uuid === rounds.value);
  const emit = (kind, extra = {}) => doc.defaultView.activateSemanticAction?.(GUIDANCE_ACTION_IDS[kind], {
    context: 'guidance', source: 'control', detail: { owner, round_uuid: rounds.value, ...extra },
  });
  function paint() {
    const round = selected();
    status.textContent = !round ? t('guidance.empty') : t(round.revision === 0 ? 'guidance.launched' : round.unresolved ? 'guidance.unresolved' : 'guidance.heard');
    const target = value?.targets?.find(row => row.contact === round?.target);
    intent.textContent = !round ? '' : round.target ? t('guidance.intent', { target: target?.name ? wireText(target.name) : t('guidance.held') }) : t('guidance.stopped');
    apply.disabled = readonly || !round?.can_retarget || !value?.targets?.some(row => row.contact === targets.value);
    stop.disabled = readonly || !round?.can_stop; targets.disabled = readonly || !round?.can_retarget;
  }
  for (const picker of [rounds, targets]) {
    picker.addEventListener('change', paint);
    picker.addEventListener('keydown', event => { if (!['Tab', 'Escape'].includes(event.key)) event.stopPropagation(); });
  }
  apply.onclick = () => { if (!apply.disabled) emit('target', { target_uuid: targets.value }); };
  stop.onclick = () => { if (!stop.disabled) emit('stop'); };
  function choices(picker, rows) {
    const signature = JSON.stringify(rows.map(row => row.id));
    if (picker.dataset.choices !== signature) {
      const previous = picker.value; picker.dataset.choices = signature;
      picker.replaceChildren(...rows.map(row => { const option = doc.createElement('option'); option.value = row.id; return option; }));
      if (rows.some(row => row.id === previous)) picker.value = previous;
    }
    for (const [index, row] of rows.entries()) if (picker.options[index].textContent !== row.name) picker.options[index].textContent = row.name;
  }
  return { node, update(system, next, delegated = false) {
    if (owner !== system) { rounds.dataset.choices = targets.dataset.choices = ''; }
    owner = system; value = next; readonly = delegated; node.hidden = !next; if (!next) return;
    choices(rounds, next.rounds.map((row, index) => ({ id: row.round_uuid, name: t('guidance.round_number', { number: index + 1 }) })));
    choices(targets, next.targets.map(row => ({ id: row.contact, name: row.name ? wireText(row.name) : t('console.sensors.basic_contact') })));
    rounds.disabled = !next.rounds.length;
    for (const control of [targets, apply, stop]) control.hidden = readonly;
    paint();
  }, dispose() { node.remove(); } };
}
