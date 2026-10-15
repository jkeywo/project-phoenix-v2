/** Closed own-Repair personnel projection; no inferred people or movement. */
const index = value => Number.isSafeInteger(value) && value >= 0 && value <= 0xffffffff;
const text = value => typeof value === 'string' && value.length > 0;
const competencies = values => Array.isArray(values) && values.length > 0
  && values.every(value => ['mechanical_repair', 'medical_care'].includes(value)) && new Set(values).size === values.length;
export function personnelPayload(value, owner, controllable) {
  if (!value || !text(owner) || !Array.isArray(value.teams) || !Array.isArray(value.officers)) return null;
  const teams = value.teams.filter(team => team && text(team.id) && text(team.label) && index(team.team_idx)
    && competencies(team.competencies) && Number.isFinite(team.active_work_multiplier) && team.active_work_multiplier >= 1);
  if (teams.length !== value.teams.length || new Set(teams.map(team => team.team_idx)).size !== teams.length) return null;
  const known = new Set(teams.map(team => team.team_idx));
  const officers = value.officers.filter(officer => officer && text(officer.id) && text(officer.label)
    && competencies(officer.competencies) && Number.isSafeInteger(officer.changed_tick) && officer.changed_tick >= 0
    && Number.isFinite(officer.repair_work_multiplier) && officer.repair_work_multiplier >= 1
    && ['fit','minor','severe'].includes(officer.condition ?? 'fit')
    && Array.isArray(officer.treatment_teams ?? []) && (officer.treatment_teams ?? []).every(team => index(team) && known.has(team))
    && (!officer.treatment || index(officer.treatment.team_idx) && known.has(officer.treatment.team_idx) && ['travelling','working','returning'].includes(officer.treatment.phase) && Number.isInteger(officer.treatment.progress) && officer.treatment.progress >= 0 && officer.treatment.progress <= 1000 && typeof officer.treatment.waiting_medication === 'boolean')
    && new Set(officer.treatment_teams ?? []).size === (officer.treatment_teams ?? []).length
    && typeof officer.can_release === 'boolean' && Array.isArray(officer.available_teams)
    && officer.available_teams.every(team => index(team) && known.has(team))
    && new Set(officer.available_teams).size === officer.available_teams.length
    && (officer.location?.kind === 'at_compartment' && text(officer.location.compartment)
      || officer.location?.kind === 'with_repair_team' && index(officer.location.team_idx) && known.has(officer.location.team_idx)));
  if (officers.length !== value.officers.length || new Set(officers.map(officer => officer.id)).size !== officers.length) return null;
  return { system_id: owner, controllable: controllable === true, teams: teams.map(team => ({ ...team })),
    officers: officers.map(officer => ({ ...officer, condition: officer.condition ?? 'fit', treatment_teams: [...(officer.treatment_teams ?? [])], location: { ...officer.location }, available_teams: [...officer.available_teams] })) };
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
