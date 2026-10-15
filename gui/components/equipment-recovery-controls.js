import { t, wireText } from '../strings.js';
import { FITTING_ACTION_IDS } from '../stations/fitting-actions.js';
const key = option => JSON.stringify([option.target_uuid, option.slot, option.destination_locker]);

/** Observed source choices only; completing the real paid job moves the item. */
export function mountEquipmentRecoveryControls(doc) {
  const node = doc.createElement('fieldset'), heading = doc.createElement('legend'), picker = doc.createElement('select'), label = doc.createElement('label'), reason = doc.createElement('p'), progress = doc.createElement('p');
  heading.textContent = t('fitting.recovery.title'); label.textContent = t('fitting.recovery.selection'); label.append(picker);
  const start = doc.createElement('button'), cancel = doc.createElement('button'); start.type = cancel.type = 'button'; start.textContent = t('fitting.recovery.start'); cancel.textContent = t('fitting.recovery.cancel');
  node.dataset.equipmentRecovery = ''; node.append(heading, label, reason, start, cancel, progress);
  let current = null, owner = null, delegated = true;
  const selected = () => current?.options?.find(option => key(option) === picker.value);
  function paint() {
    const option = selected(); start.disabled = delegated || !current?.can_start || !option || !!option.blocked_reason;
    cancel.disabled = delegated || !current?.can_cancel;
    reason.textContent = option?.blocked_reason ? t('fitting.recovery.blocked.' + option.blocked_reason) : '';
  }
  picker.addEventListener('change', paint); picker.addEventListener('keydown', event => { if (!['Tab', 'Escape'].includes(event.key)) event.stopPropagation(); });
  const activate = (name, extra = {}) => doc.defaultView.activateSemanticAction?.(FITTING_ACTION_IDS[name], { context: 'fitting', source: 'control', detail: { owner, ...extra } });
  start.addEventListener('click', () => { const option = selected(); if (!start.disabled && option) activate('recover', { target_uuid: option.target_uuid, slot: option.slot, destination_locker: option.destination_locker }); });
  cancel.addEventListener('click', () => { if (!cancel.disabled) activate('recover-cancel'); });
  return { node, update(system, value, readonly = false) {
    const oldOwner = owner; owner = system; current = value; delegated = readonly; node.hidden = !value; if (!value) return;
    const previous = picker.value, options = value.options || [], ids = options.map(key);
    if (picker.dataset.options !== JSON.stringify(ids)) {
      picker.dataset.options = JSON.stringify(ids); picker.replaceChildren();
      for (const option of options) { const row = doc.createElement('option'); row.value = key(option); picker.append(row); }
      if (oldOwner === owner && ids.includes(previous)) picker.value = previous;
      else if (value.selected && ids.includes(key(value.selected))) picker.value = key(value.selected);
    }
    for (const [index, option] of options.entries()) picker.options[index].textContent = t('fitting.recovery.option', { source: wireText(option.target_name || option.target_uuid),
      slot: wireText(option.slot), equipment: wireText(option.equipment), locker: wireText(option.destination_locker) });
    picker.disabled = readonly || !value.can_start;
    for (const control of [picker, start, cancel]) control.hidden = readonly;
    progress.textContent = t('fitting.progress', { phase: t('fitting.recovery.phase.' + value.phase), progress: Math.round((value.progress || 0) / 10) }); paint();
  }, dispose() { node.remove(); } };
}
