/** Closed own-Repair personnel projection; no inferred people or movement. */
const index = value => Number.isSafeInteger(value) && value >= 0 && value <= 0xffffffff;
const text = value => typeof value === 'string' && value.length > 0;
const competencies = values => Array.isArray(values) && values.length > 0
  && values.every(value => ['mechanical_repair', 'medical_care', 'personnel_transport'].includes(value)) && new Set(values).size === values.length;
export function personnelPayload(value, owner, controllable) {
  if (!value || !text(owner) || !Array.isArray(value.teams) || !Array.isArray(value.officers)) return null;
  const teams = value.teams.filter(team => team && text(team.id) && text(team.label) && index(team.team_idx)
    && competencies(team.competencies) && Number.isFinite(team.active_work_multiplier) && team.active_work_multiplier >= 1);
  if (teams.length !== value.teams.length || new Set(teams.map(team => team.team_idx)).size !== teams.length) return null;
  const known = new Set(teams.map(team => team.team_idx));
  const carried = value.carried_destinations ?? [];
  if (!Array.isArray(carried) || carried.some(row => !row || !text(row.craft) || !(row.name == null || text(row.name)) || !index(row.free_berths))
    || new Set(carried.map(row => row.craft)).size !== carried.length) return null;
  const destination = row => row?.kind === 'home' || row?.kind === 'craft' && text(row.craft);

  const officers = value.officers.filter(officer => officer && text(officer.id) && text(officer.label)
    && competencies(officer.competencies) && Number.isSafeInteger(officer.changed_tick) && officer.changed_tick >= 0
    && Number.isFinite(officer.repair_work_multiplier) && officer.repair_work_multiplier >= 1
    && ['fit','minor','severe'].includes(officer.condition ?? 'fit')
    && Array.isArray(officer.treatment_teams ?? []) && (officer.treatment_teams ?? []).every(team => index(team) && known.has(team))
    && (!officer.treatment || index(officer.treatment.team_idx) && known.has(officer.treatment.team_idx) && ['travelling','working','returning'].includes(officer.treatment.phase) && Number.isInteger(officer.treatment.progress) && officer.treatment.progress >= 0 && officer.treatment.progress <= 1000 && typeof officer.treatment.waiting_medication === 'boolean')
    && new Set(officer.treatment_teams ?? []).size === (officer.treatment_teams ?? []).length
    && (!officer.destination_request || destination(officer.destination_request.destination) && Number.isSafeInteger(officer.destination_request.requested_tick) && officer.destination_request.requested_tick >= 0)
    && Array.isArray(officer.transfer_teams ?? []) && (officer.transfer_teams ?? []).every(team => index(team) && known.has(team))
    && new Set(officer.transfer_teams ?? []).size === (officer.transfer_teams ?? []).length
    && (!officer.transfer || index(officer.transfer.team_idx) && known.has(officer.transfer.team_idx) && ['collecting','escorting','returning'].includes(officer.transfer.stage) && typeof officer.transfer.completed === 'boolean' && typeof officer.transfer.blocked === 'boolean')
    && typeof officer.can_release === 'boolean' && Array.isArray(officer.available_teams)
    && officer.available_teams.every(team => index(team) && known.has(team))
    && new Set(officer.available_teams).size === officer.available_teams.length
    && (officer.location?.kind === 'at_compartment' && text(officer.location.compartment)
      || officer.location?.kind === 'with_repair_team' && index(officer.location.team_idx) && known.has(officer.location.team_idx)
      || officer.location?.kind === 'with_transfer' && text(officer.location.actor) && index(officer.location.team_idx) && known.has(officer.location.team_idx) && Number.isSafeInteger(officer.location.job) && officer.location.job >= 0
      || officer.location?.kind === 'aboard_craft' && text(officer.location.berth) && carried.some(craft => craft.craft === officer.location.craft)));
  if (officers.length !== value.officers.length || new Set(officers.map(officer => officer.id)).size !== officers.length) return null;
  return { carried_destinations: carried.map(row => ({...row})), system_id: owner, controllable: controllable === true, teams: teams.map(team => ({ ...team })),
    officers: officers.map(officer => ({ ...officer, condition: officer.condition ?? 'fit', transfer_teams: [...(officer.transfer_teams ?? [])], treatment_teams: [...(officer.treatment_teams ?? [])], location: { ...officer.location }, available_teams: [...officer.available_teams] })) };
}

export function personnelPosting(view, detail) {
  if (!view || view.controllable !== true || !detail || detail.control_system_id !== view.system_id) return null;
  const officer = view.officers?.find(row => row.id === detail.officer);
  if (!officer || detail.changed_tick !== officer.changed_tick) return null;
  if (detail.team_idx === null ? !officer.can_release
    : !index(detail.team_idx) || !officer.available_teams.includes(detail.team_idx)) return null;
  return { officer: officer.id, team_idx: detail.team_idx };
}

export function personnelTreatment(view, detail) {
  if (!view || view.controllable !== true || !detail || detail.control_system_id !== view.system_id) return null;
  const officer = view.officers?.find(row => row.id === detail.officer);
  if (!officer || detail.changed_tick !== officer.changed_tick || !index(detail.team_idx)
    || officer.condition === 'fit' || !officer.treatment_teams?.includes(detail.team_idx)) return null;
  return { officer: officer.id, team_idx: detail.team_idx };
}

function sameDestinationRequest(a,b) {
  return a?.requested_tick === b?.requested_tick && a?.destination?.kind === b?.destination?.kind
    && (b?.destination?.kind === 'home' || a?.destination?.craft === b?.destination?.craft);
}
/** Visible choices are information only; the ordinary owner rechecks actual custody. */
export function personnelDestination(view, detail) {
  if (!view || view.controllable !== true || !detail || detail.control_system_id !== view.system_id) return null;
  const officer = view.officers?.find(row => row.id === detail.officer);
  if (!officer || detail.changed_tick !== officer.changed_tick) return null;
  const destination = detail.destination;
  if (destination !== null && !(destination?.kind === 'home' || destination?.kind === 'craft'
    && view.carried_destinations?.some(craft => craft.craft === destination.craft))) return null;
  return {officer:officer.id,destination:destination === null ? null : destination.kind === 'home' ? {kind:'home'} : {kind:'craft',craft:destination.craft}};
}
export function personnelTransfer(view, detail) {
  if (!view || view.controllable !== true || !detail || detail.control_system_id !== view.system_id) return null;
  const officer = view.officers?.find(row => row.id === detail.officer);
  if (!officer || detail.changed_tick !== officer.changed_tick || !index(detail.team_idx)
    || !officer.destination_request || !sameDestinationRequest(detail.request, officer.destination_request)
    || !officer.transfer_teams?.includes(detail.team_idx)) return null;
  return {officer:officer.id,team_idx:detail.team_idx,request:{destination:{...officer.destination_request.destination},requested_tick:officer.destination_request.requested_tick}};
}
