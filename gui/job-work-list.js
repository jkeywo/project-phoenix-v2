import { t } from './strings.js';

const phases = Object.freeze({
  travelling: 'job.phase.travelling',
  working: 'job.phase.working',
  returning: 'job.phase.returning',
  external: 'job.phase.external',
  finished: 'job.phase.finished',
});
const outcomes = Object.freeze({
  completed: 'job.outcome.completed',
  cancelled: 'job.outcome.cancelled',
  failed: 'job.outcome.failed',
});
const rendered = new WeakMap();

/** Display permitted assignments only. A work-list row never offers a command. */
export function renderJobWorkList(container, jobs, teams) {
  const rows = [];
  for (const job of Array.isArray(jobs) ? jobs : []) {
    const phase = phases[job.phase];
    if (!phase || !Number.isInteger(job.team_idx)) continue;
    const team = teams.find(team => team.id === job.team_idx);
    let text = t('job.work_item', {
      id: job.id, team: team?.label || String(job.team_idx + 1),
      phase: t(outcomes[job.outcome] || phase),
    });
    if (job.workflow_error) text += ` · ${t('job.workflow_error')}`;
    else if (job.paused) text += ` · ${t('job.paused')}`;
    if (job.target_work) text+=` · ${job.target_work.compartment || `${job.target_work.system}/${job.target_work.component}`}`;
    if (job.work_progress!=null) text+=` · ${Math.round(job.work_progress/10)}%`;
    if (job.waiting_parts) text+=` · ${t('damage.repair.waiting_parts')}`;
    rows.push([String(job.id), text]);
  }
  const key = JSON.stringify(rows);
  if (rendered.get(container) === key) return;
  rendered.set(container, key);
  container.replaceChildren();
  for (const [id, text] of rows) {
    const row = container.ownerDocument.createElement('div');
    row.dataset.jobId = id;
    row.textContent = text;
    container.append(row);
  }
}
