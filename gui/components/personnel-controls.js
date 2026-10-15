import { t } from '../strings.js';
import { activateEngineeringAction } from '../stations/engineering-action-control.js';
import { PERSONNEL_POST_ACTION_ID, PERSONNEL_TREAT_ACTION_ID } from '../stations/personnel-actions.js';
const mounts = new WeakMap();
/** Stable native pickers within the existing Repair surface. */
export function renderPersonnelControls(container, state, owner) {
  const view = state?.personnel;
  container.hidden = !view?.officers?.length;
  if (container.hidden) { container.replaceChildren(); mounts.delete(container); return; }
  let mount = mounts.get(container);
  if (!mount) {
    const heading = document.createElement('h3'); heading.textContent = t('personnel.title');
    const list = document.createElement('div'); list.className = 'personnel-list';
    container.append(heading, list); mount = { list, rows: new Map(), view, owner }; mounts.set(container, mount);
  }
  mount.view = view; mount.owner = owner;
  for (const [id, row] of mount.rows) if (!view.officers.some(officer => officer.id === id)) { row.root.remove(); mount.rows.delete(id); }
  for (const officer of view.officers) {
    let row = mount.rows.get(officer.id);
    if (!row) {
      const root = document.createElement('div'); root.className = 'personnel-row';
      const name = document.createElement('strong'), status = document.createElement('span');
      const select = document.createElement('select'), post = document.createElement('button'), release = document.createElement('button');
      const careSelect = document.createElement('select'), treat = document.createElement('button');
      root.append(name, status, select, post, release, careSelect, treat); mount.list.append(root);
      row = { root, name, status, select, post, release, careSelect, treat }; mount.rows.set(officer.id, row);
      const activate = team_idx => {
        const current = mount.view.officers.find(item => item.id === officer.id);
        if (!current || !mount.view.controllable) return;
        if (row.armed && (row.armed.owner !== mount.view.system_id || row.armed.tick !== current.changed_tick)) { row.armed = null; return; }
        row.armed = null;
        activateEngineeringAction(mount.owner, PERSONNEL_POST_ACTION_ID,
          { officer: current.id, team_idx, changed_tick: current.changed_tick, control_system_id: mount.view.system_id }, 'set_duty_officer_posting');
      };
      post.addEventListener('click', () => { if (select.value !== '') activate(Number(select.value)); });
      release.addEventListener('click', () => activate(null));
      const arm = () => { const current = mount.view.officers.find(item => item.id === officer.id); row.armed = { owner: mount.view.system_id, tick: current?.changed_tick }; };
      treat.addEventListener('click', () => {
        const current = mount.view.officers.find(item => item.id === officer.id);
        if (!current || !mount.view.controllable || careSelect.value === '') return;
        if (row.armed && (row.armed.owner !== mount.view.system_id || row.armed.tick !== current.changed_tick)) { row.armed = null; return; }
        row.armed = null;
        activateEngineeringAction(mount.owner, PERSONNEL_TREAT_ACTION_ID,
          { officer: current.id, team_idx: Number(careSelect.value), changed_tick: current.changed_tick, control_system_id: mount.view.system_id }, 'treat_duty_officer');
      });
      for (const button of [post, release, treat]) button.addEventListener('pointerdown', arm);
      for (const input of [select, post, release, careSelect, treat]) input.addEventListener('keydown', event => {
        if (event.key === ' ' || event.key === 'Enter') arm();
        event.stopPropagation();
      });
    }
    row.name.textContent = t(officer.label);
    const current = officer.location.kind === 'with_repair_team' ? officer.location.team_idx : null;
    const team = view.teams.find(item => item.team_idx === current);
    const room = state.interior?.compartment?.find(room => room.id === officer.location.compartment);
    row.status.textContent = t('personnel.condition.' + (officer.condition || 'fit')) + ' · ' + t(current === null ? 'personnel.at_home' : 'personnel.with_team',
      { location: current === null ? t(room?.label || officer.location.compartment) : t(team?.label || ''),
        multiplier: team?.active_work_multiplier ?? 1 });
    if (officer.treatment) {
      const care = officer.treatment, team = view.teams.find(item => item.team_idx === care.team_idx);
      row.status.textContent += ' · ' + t('personnel.treatment_status', {team:t(team?.label || ''),phase:t('job.phase.' + care.phase),progress:Math.round(care.progress / 10)});
      if (care.waiting_medication) row.status.textContent += ' · ' + t('personnel.waiting_medication');
    }
    const choices = officer.available_teams.map(index => view.teams.find(team => team.team_idx === index)).filter(Boolean);
    const signature = JSON.stringify(choices.map(team => [team.team_idx, team.label]));
    if (row.select.dataset.choices !== signature) {
      const held = row.select.value;
      row.select.replaceChildren();
      const empty = document.createElement('option'); empty.value = ''; empty.textContent = t('personnel.choose_team'); row.select.append(empty);
      for (const team of choices) { const option = document.createElement('option'); option.value = String(team.team_idx); option.textContent = t(team.label); row.select.append(option); }
      row.select.value = choices.some(team => String(team.team_idx) === held) ? held : current === null ? '' : String(current);
      row.select.dataset.choices = signature;
    }
    row.select.setAttribute('aria-label', t('personnel.choose_for', { officer: t(officer.label) }));
    row.select.disabled = !view.controllable || choices.length === 0;
    row.post.textContent = t('personnel.action.post'); row.post.disabled = row.select.disabled || row.select.value === '';
    row.select.onchange = () => { row.post.disabled = row.select.disabled || row.select.value === ''; };
    row.release.textContent = t('personnel.action.release'); row.release.disabled = !view.controllable || !officer.can_release;
    const careChoices = (officer.treatment_teams || []).map(index => view.teams.find(team => team.team_idx === index)).filter(Boolean);
    const careSignature = JSON.stringify(careChoices.map(team => [team.team_idx, team.label]));
    if (row.careSelect.dataset.choices !== careSignature) {
      const held = row.careSelect.value;
      row.careSelect.replaceChildren();
      const empty = document.createElement('option'); empty.value = ''; empty.textContent = t('personnel.choose_team'); row.careSelect.append(empty);
      for (const team of careChoices) { const option = document.createElement('option'); option.value = String(team.team_idx); option.textContent = t(team.label); row.careSelect.append(option); }
      row.careSelect.value = careChoices.some(team => String(team.team_idx) === held) ? held : '';
      row.careSelect.dataset.choices = careSignature;
    }
    row.careSelect.setAttribute('aria-label', t('personnel.choose_care_for', { officer: t(officer.label) }));
    row.careSelect.disabled = !view.controllable || careChoices.length === 0;
    row.treat.textContent = t('personnel.action.treat'); row.treat.disabled = row.careSelect.disabled || row.careSelect.value === '';
    row.careSelect.onchange = () => { row.treat.disabled = row.careSelect.disabled || row.careSelect.value === ''; };

  }
}
