import { t } from './strings.js';

const finite = value => Number.isFinite(value) ? value.toFixed(2) : '—';
/** Displays actual bounded trials; no demand/endurance estimator lives here. */
export function paintFittingRehearsal(root, state) {
  const doc = root.ownerDocument, el = (tag, text) => {
    const node = doc.createElement(tag); if (text != null) node.textContent = text; return node;
  };
  root.replaceChildren();
  root.append(el('h3', t('fitting.rehearsal.title')));
  if (state.error) { root.append(el('p', t(state.error.message))); return; }
  if (state.busy) { root.append(el('p', t('fitting.rehearsal.running'))); return; }
  const pair = state.results;
  if (!pair) { root.append(el('p', t('fitting.rehearsal.hint'))); return; }
  root.append(el('p', t('fitting.rehearsal.measured-hint')));
  const table = el('table'), head = el('tr');
  head.append(el('th', t('fitting.rehearsal.metric')), el('th', t('fitting.rehearsal.baseline')), el('th', t('fitting.rehearsal.selected')));
  table.append(head);
  const rows = [
    ['workload', r => r.profile.workload], ['source', r => r.revision],
    ['content', r => r.hull_content_id], ['profile-source', r => r.workload_revision],
    ['seed', r => String(r.selection.seed)], ['slot', r => r.selection.slot],
    ['duration', r => finite(r.measured_intervals * r.fixed_period_secs)],
    ['generated', r => finite(r.generated_energy)],
    ['paid', r => finite(Object.values(r.buffers).reduce((sum, b) => sum + b.paid_energy, 0))],
    ['reserve-change', r => finite(Object.values(r.buffers).reduce((sum, b) => sum + b.final_energy - b.initial_energy, 0))],
    ['coolant', r => finite(Object.values(r.initial_coolant).reduce((a,b) => a+b, 0) - Object.values(r.final_coolant).reduce((a,b) => a+b, 0))],
    ['cuts', r => String(r.flow_cut_intervals)],
    ['duty', r => `${r.achieved_engagement_intervals}/${r.engagement_intervals}`],
    ['subject-loss', r => finite(r.subject_structural_hp_lost)],
    ['activity', r => t(r.insufficient_activity ? 'fitting.rehearsal.insufficient' : 'fitting.rehearsal.observed')],
  ];
  for (const [key, read] of rows) {
    const row = el('tr'); row.append(el('th', t('fitting.rehearsal.' + key)), el('td', read(pair.baseline)), el('td', read(pair.selected))); table.append(row);
  }
  root.append(table);
  for (const [name, report] of Object.entries(pair)) {
    const details = el('details'), summary = el('summary', t('fitting.rehearsal.' + name)); details.append(summary);
    const systems = el('table'); const titles = el('tr');
    for (const title of ['system', 'start', 'end', 'minimum', 'heat', 'trips', 'recoveries', 'paid']) titles.append(el('th', t('fitting.rehearsal.' + title)));
    systems.append(titles);
    for (const [id, row] of Object.entries(report.buffers)) {
      const line = el('tr'); line.append(el('th', id));
      for (const value of [row.initial_energy, row.final_energy, row.minimum_energy, row.maximum_heat, row.thermal_trips, row.thermal_recoveries, row.paid_energy]) line.append(el('td', finite(value)));
      systems.append(line);
    }
    details.append(systems);
    const assumptions = el('pre', JSON.stringify({ phases: report.profile, fixed_period_secs: report.fixed_period_secs,
      initial_power: report.authored_initial_power, nominal_equipment: report.nominal_equipment,
      authored_initial_coolant: report.authored_initial_coolant,
      authored_initial_ammunition: report.authored_initial_ammunition,
      measured_start_coolant: report.initial_coolant, choices: report.choices }, null, 2));
    details.append(el('p', t('fitting.rehearsal.assumptions')), assumptions);
    const svg = doc.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('viewBox', '0 0 600 160'); svg.setAttribute('role', 'img');
    const title = doc.createElementNS(svg.namespaceURI, 'title'); title.textContent = t('fitting.rehearsal.chart'); svg.append(title);
    const max = Math.max(1, ...report.charts.map(row => row.reserve));
    const line = doc.createElementNS(svg.namespaceURI, 'polyline');
    line.setAttribute('points', report.charts.map(row => `${10 + 580 * row.measured_intervals / report.measured_intervals},${150 - 140 * row.reserve / max}`).join(' '));
    line.setAttribute('fill', 'none'); line.setAttribute('stroke', '#6ecbff'); line.setAttribute('stroke-width', '2'); svg.append(line);
    details.append(svg); root.append(details);
  }
}
