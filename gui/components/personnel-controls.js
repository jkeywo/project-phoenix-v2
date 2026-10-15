import { personKey } from '../personnel.js';
import { t } from '../strings.js';
import { activateEngineeringAction } from '../stations/engineering-action-control.js';
import { PERSONNEL_POST_ACTION_ID, PERSONNEL_TREAT_ACTION_ID, PERSONNEL_DESTINATION_ACTION_ID, PERSONNEL_TRANSFER_ACTION_ID } from '../stations/personnel-actions.js';
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
  for (const [id, row] of mount.rows) if (!view.officers.some(officer => personKey(officer.person) === id)) { row.root.remove(); mount.rows.delete(id); }
  for (const officer of view.officers) {
    let row = mount.rows.get(personKey(officer.person));
    if (!row) {
      const root = document.createElement('div'); root.className = 'personnel-row';
      const name = document.createElement('strong'), status = document.createElement('span');
      const select = document.createElement('select'), post = document.createElement('button'), release = document.createElement('button');
      const careSelect = document.createElement('select'), treat = document.createElement('button');
      const destination = document.createElement('select'), request = document.createElement('button'), cancel = document.createElement('button');
      const escort = document.createElement('select'), transfer = document.createElement('button');
      destination.dataset.personnelDestination = ''; escort.dataset.personnelEscort = ''; careSelect.dataset.personnelCare = '';
      root.append(name, status, select, post, release, destination, request, cancel, escort, transfer, careSelect, treat); mount.list.append(root);
      row = { root, name, status, select, post, release, careSelect, treat, destination, request, cancel, escort, transfer }; mount.rows.set(personKey(officer.person), row);
      const activate = team_idx => {
        const current = mount.view.officers.find(item => personKey(item.person) === personKey(officer.person));
        if (!current || !mount.view.controllable) return;
        if (row.armed && (row.armed.owner !== mount.view.system_id || row.armed.tick !== current.changed_tick)) { row.armed = null; return; }
        row.armed = null;
        activateEngineeringAction(mount.owner, PERSONNEL_POST_ACTION_ID,
          { officer: {...current.person}, team_idx, changed_tick: current.changed_tick, control_system_id: mount.view.system_id }, 'set_duty_officer_posting');
      };
      post.addEventListener('click', () => { if (select.value !== '') activate(Number(select.value)); });
      release.addEventListener('click', () => activate(null));
      const arm = () => { const current = mount.view.officers.find(item => personKey(item.person) === personKey(officer.person)); row.armed = { owner: mount.view.system_id, tick: current?.changed_tick, request: JSON.stringify(current?.destination_request), destination:row.destination.value }; };
      treat.addEventListener('click', () => {
        const current = mount.view.officers.find(item => personKey(item.person) === personKey(officer.person));
        if (!current || !mount.view.controllable || careSelect.value === '') return;
        if (row.armed && (row.armed.owner !== mount.view.system_id || row.armed.tick !== current.changed_tick)) { row.armed = null; return; }
        row.armed = null;
        activateEngineeringAction(mount.owner, PERSONNEL_TREAT_ACTION_ID,
          { officer: {...current.person}, team_idx: Number(careSelect.value), changed_tick: current.changed_tick, control_system_id: mount.view.system_id }, 'treat_duty_officer');
      });
      const sendTransfer = (action, wire, detail) => {
        const current = mount.view.officers.find(item => personKey(item.person) === personKey(officer.person));
        if (!current || !mount.view.controllable) return;
        if (row.armed && (row.armed.owner !== mount.view.system_id || row.armed.tick !== current.changed_tick)) { row.armed = null; return; }
        if (row.armed && (row.armed.request !== JSON.stringify(current.destination_request)
          || action === PERSONNEL_DESTINATION_ACTION_ID && detail.destination !== null && row.armed.destination !== row.destination.value)) {row.armed=null;return;}
        row.armed = null;
        activateEngineeringAction(mount.owner, action, {officer:{...current.person},changed_tick:current.changed_tick,control_system_id:mount.view.system_id,...detail},wire);
      };
      request.addEventListener('click', () => { if (destination.value) sendTransfer(PERSONNEL_DESTINATION_ACTION_ID,'set_duty_officer_destination',{destination:destination.value === 'home' ? {kind:'home'} : {kind:'craft',craft:destination.value}}); });
      cancel.addEventListener('click', () => sendTransfer(PERSONNEL_DESTINATION_ACTION_ID,'set_duty_officer_destination',{destination:null}));
      transfer.addEventListener('click', () => { if (escort.value !== '') sendTransfer(PERSONNEL_TRANSFER_ACTION_ID,'dispatch_officer_transfer',{team_idx:Number(escort.value),request:structuredClone(mount.view.officers.find(item=>personKey(item.person)===personKey(officer.person))?.destination_request)}); });
      for (const button of [post, release, treat, request, cancel, transfer]) button.addEventListener('pointerdown', arm);
      for (const input of [select, post, release, careSelect, treat, destination, request, cancel, escort, transfer]) input.addEventListener('keydown', event => {
        if (event.key === ' ' || event.key === 'Enter') arm();
        event.stopPropagation();
      });
    }
    row.name.textContent = t(officer.label);
    const current = officer.location.kind === 'with_repair_team' ? officer.location.team_idx : null;
    const team = view.teams.find(item => item.team_idx === current);
    const room = state.interior?.compartment?.find(room => room.id === officer.location.compartment);
    const locationKey = officer.location.kind === 'aboard_craft' ? 'personnel.aboard_craft' : officer.location.kind === 'with_transfer' ? 'personnel.with_escort' : current === null ? 'personnel.at_home' : 'personnel.with_team';
    const pod = view.carried_destinations?.find(row => row.craft === officer.location.craft);
    row.status.textContent = t('personnel.condition.' + (officer.condition || 'fit')) + ' \u00b7 ' + t(locationKey,
      { location: officer.location.kind === 'aboard_craft' ? t(pod?.name || 'personnel.pod') : officer.location.kind === 'with_transfer' ? t(view.teams.find(row=>row.team_idx===officer.location.team_idx)?.label || '') : current === null ? t(room?.label || officer.location.compartment) : t(team?.label || ''), multiplier: team?.active_work_multiplier ?? 1 });
    if (officer.transfer) row.status.textContent += ' \u00b7 ' + t('personnel.transfer.' + officer.transfer.stage) + (officer.transfer.blocked ? ' \u00b7 ' + t('personnel.transfer.blocked') : '');
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
    const destinations = [{value:'home',label:t('personnel.destination.home')}, ...(view.carried_destinations || []).map(craft => ({value:craft.craft,label:t('personnel.destination.craft',{name:t(craft.name || 'personnel.pod'),free:craft.free_berths})}))];
    const destinationSignature = JSON.stringify(destinations);
    if (row.destination.dataset.choices !== destinationSignature) {
      const held = row.destination.value; row.destination.replaceChildren();
      for (const choice of destinations) { const option=document.createElement('option');option.value=choice.value;option.textContent=choice.label;row.destination.append(option); }
      row.destination.value = destinations.some(choice=>choice.value===held) ? held : 'home'; row.destination.dataset.choices=destinationSignature;
    }
    row.destination.setAttribute('aria-label',t('personnel.choose_destination',{officer:t(officer.label)}));
    row.destination.disabled = !view.controllable;
    row.request.textContent=t('personnel.action.destination');row.request.disabled=!view.controllable;
    row.cancel.textContent=t('personnel.action.cancel_destination');row.cancel.disabled=!view.controllable||!officer.destination_request;
    const escortChoices=(officer.transfer_teams||[]).map(index=>view.teams.find(team=>team.team_idx===index)).filter(Boolean);
    const escortSignature=JSON.stringify(escortChoices.map(team=>[team.team_idx,team.label]));
    if(row.escort.dataset.choices!==escortSignature){const held=row.escort.value;row.escort.replaceChildren();const empty=document.createElement('option');empty.value='';empty.textContent=t('personnel.choose_team');row.escort.append(empty);for(const team of escortChoices){const option=document.createElement('option');option.value=String(team.team_idx);option.textContent=t(team.label);row.escort.append(option);}row.escort.value=escortChoices.some(team=>String(team.team_idx)===held)?held:'';row.escort.dataset.choices=escortSignature;}
    row.escort.setAttribute('aria-label',t('personnel.choose_escort',{officer:t(officer.label)}));row.escort.disabled=!view.controllable||!escortChoices.length;
    row.transfer.textContent=t('personnel.action.transfer');row.transfer.disabled=row.escort.disabled||row.escort.value==='';
    row.escort.onchange=()=>{row.transfer.disabled=row.escort.disabled||row.escort.value==='';};
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
