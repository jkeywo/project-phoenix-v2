import { t, wireText } from '../strings.js';
import { WEAPON_TARGET_ACTION_IDS } from '../stations/weapon-target-actions.js';
import { activateTacticalAction } from '../stations/tactical-action-control.js';

/** Stable native pickers; incoming firing snapshots never replace focused DOM. */
export function mountWeaponTargetControls(doc) {
  const node = doc.createElement('details'), title = doc.createElement('summary'), body = doc.createElement('div');
  title.textContent = t('weapons.assignments.title'); node.append(title, body);
  const controls = new Map(); let current = null;
  const emit = (kind, detail) => activateTacticalAction({ sendAction: (...args) => doc.defaultView?.sendAction?.(...args) },
    WEAPON_TARGET_ACTION_IDS[kind], detail, { target: 'set_weapon_target', group: 'set_weapon_group_target', rejoin: 'follow_weapon_group', primary: 'follow_primary_target' }[kind]);
  function make(key, label, group) {
    const row = doc.createElement('div'), heading = doc.createElement('span'), status = doc.createElement('span'), picker = doc.createElement('select');
    row.dataset.assignmentKey = key; heading.textContent = label; picker.setAttribute('aria-label', label);
    const apply = doc.createElement('button'); apply.type = 'button'; apply.textContent = t('weapons.assignments.assign');
    apply.onclick = () => {
      if (group) {
        const definition = current?.groups?.find(group => group.id === key);
        // Each actual bank receives its own ordinary correlated activation.
        for (const member of current?.rows || []) if (member.controllable && definition?.members.includes(member.system))
          emit('group', { group: key, system: member.system, uuid: picker.value || null });
      } else emit('target', { system: key, uuid: picker.value || null });
    };
    row.append(heading, status, picker, apply);
    const buttons = [apply];
    if (!group) for (const kind of ['rejoin', 'primary']) {
      const button = doc.createElement('button'); button.type = 'button'; button.textContent = t('weapons.assignments.' + kind);
      button.onclick = () => emit(kind, { system: key }); row.append(button); buttons.push(button);
    }
    for (const button of buttons) button.style.minHeight = '44px';
    picker.style.minHeight = '44px'; body.append(row);
    const result = { row, status, picker, buttons, signature: null }; controls.set((group ? 'group/' : 'system/') + key, result); return result;
  }
  function choices(control, contacts) {
    const options = [{ id: '', name: t('weapons.assignments.hold') }, ...contacts.map(row => ({ id: row.id, name: row.name || t('console.sensors.basic_contact') }))];
    const signature = JSON.stringify(options);
    if (signature === control.signature) return;
    const selected = control.picker.value;
    control.picker.replaceChildren(...options.map(row => { const option = doc.createElement('option'); option.value = row.id; option.textContent = row.name; return option; }));
    if (selected && !options.some(row => row.id === selected)) {
      const stale = doc.createElement('option'); stale.value = selected; stale.textContent = t('weapons.assignments.held_contact'); stale.disabled = true; control.picker.append(stale);
    }
    control.picker.value = selected;
    control.signature = signature;
  }
  return { node, update(view) {
    current = view; node.hidden = !view?.rows?.length;
    if (!view) return;
    const keep = new Set();
    for (const group of view.groups || []) {
      const key = 'group/' + group.id; keep.add(key);
      const control = controls.get(key) || make(group.id, t(group.label), true);
      choices(control, view.contacts || []);
      const available = view.rows.filter(row => row.controllable && group.members.includes(row.system)).length;
      control.status.textContent = t('weapons.assignments.members', { count: available, total: group.members.length });
      for (const button of control.buttons) button.disabled = !available;
      control.picker.disabled = !available;
    }
    for (const row of view.rows) {
      const key = 'system/' + row.system; keep.add(key);
      const control = controls.get(key) || make(row.system, wireText(row.label || row.system), false);
      choices(control, view.contacts || []);
      const individual = row.assignment?.individual, group = row.assignment?.group;
      const target = individual?.mode === 'target' ? individual.contact : !individual ? group?.target : null;
      const name = view.contacts?.find(contact => contact.id === target)?.name || t('weapons.assignments.held_contact');
      const base = group ? t('weapons.assignments.group_base', { group: t(view.groups.find(row => row.id === group.group)?.label || group.group), target: group.unavailable ? t('weapons.assignments.unavailable') : group.target ? (view.contacts?.find(contact => contact.id === group.target)?.name || t('weapons.assignments.held_contact')) : t('weapons.assignments.hold') }) : t('weapons.assignments.primary');
      const selected = individual?.mode === 'hold' ? t('weapons.assignments.hold') : individual?.mode === 'target' ? name : individual?.mode === 'unavailable' ? t('weapons.assignments.unavailable') : null;
      control.status.textContent = (!row.controllable ? t(row.auto ? 'console.common.auto' : 'weapons.assignments.read_only') + ' / ' : '')
        + (selected ? t('weapons.assignments.override', { target: selected }) + ' / ' + base : base);
      for (const button of control.buttons) button.disabled = !row.controllable;
      control.picker.disabled = !row.controllable;
    }
    for (const [key, control] of controls) if (!keep.has(key)) { control.row.remove(); controls.delete(key); }
  }, get value() { return current; } };
}
