// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';
import { securityTeamRows, securityActionRows } from '../security-dispatch.js';

/**
 * `<ph-security-teams>` — the Security console panel (issue #1346, PRD #1337).
 *
 * The repair-team-style interface the issue asks for, in one element: the team
 * list with each team's state, assignment, progress and risk; the eligible
 * targets with the actions each of them offers, their duration, risk, priority
 * and authored warning; a dispatch control per (team, target, action) and a
 * recall per team; and the one refusal the server last returned.
 *
 * It DECIDES nothing. Every rule — the reach, whether a team is free, whether a
 * target offers an action — lives server-side and arrives on the blackboard
 * already answered (`in_range`, `canDispatch`, `canRecall`, `refusal`), because
 * a console that re-derived any of them would eventually offer a dispatch the
 * server refuses for a reason the panel computed differently. The projection
 * itself is `gui/security-dispatch.js`, shared with the tests.
 *
 * No English: every label is a `strings.csv` id resolved through `t()`, and the
 * state/action/priority maps below are written as literal `t('…')` calls rather
 * than composed ids so `check-strings.mjs` can see every one of them.
 */
export class PhSecurityTeams extends PhElement {
  template() {
    return `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); min-height: 0; }
    :host * { box-sizing: border-box; }
    /* Teams and available work render as two blocks side by side by default
       (issue #1382) — the inverse of engineering's stacked-pair (default
       column, portrait forces row): here default is row, and portrait forces
       column because a phone has no width to spare for both. */
    .blocks { display: flex; flex-direction: row; gap: 0.6rem; flex: 1; min-height: 0; }
    .block { display: flex; flex-direction: column; min-height: 0; flex: 1; min-width: 0; }
    .block + .block { border-left: 1px solid var(--line-faint); padding-left: 0.6rem; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; flex-shrink: 0; }
    #teams, #jobs { flex: 1; min-height: 0; overflow-y: auto; }
    .card { border: 1px solid var(--line-faint); background: var(--bg-card); padding: 0.5rem; display: flex; flex-direction: column; gap: 0.3rem; margin-bottom: 0.4rem; }
    .card-top { display: flex; justify-content: space-between; align-items: center; gap: 0.4rem; }
    .team-label { font-size: var(--text-xs); font-weight: 600; letter-spacing: 0.15em; }
    .status-badge { font-size: var(--text-xs); padding: 0.05rem 0.3rem; letter-spacing: 0.15em; border: 1px solid; }
    .status-badge.available { color: var(--ink-dim); border-color: var(--ink-dim); }
    .status-badge.deploying { color: var(--reloading); border-color: var(--reloading); }
    .status-badge.working { color: var(--loaded); border-color: var(--loaded); }
    .status-badge.withdrawing { color: var(--cyan); border-color: var(--cyan); }
    .status-badge.unavailable { color: var(--fire); border-color: var(--fire); }
    .assignment { font-size: var(--text-xs); color: var(--ink-dim); }
    .progress-wrap { width: 100%; height: 0.4rem; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; }
    .progress-fill { height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); }
    .progress-fill.deploying { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .progress-fill.withdrawing { background: linear-gradient(90deg, var(--cyan-dim), var(--cyan)); }
    .row { display: flex; flex-wrap: wrap; gap: 0.3rem; }
    .job {
      display: flex; align-items: center; gap: 0.4rem; width: 100%;
      background: var(--bg-card); border: 1px solid var(--line-faint);
      color: inherit; font: inherit; text-align: left; cursor: pointer;
      padding: 0.25rem 0.35rem; min-height: var(--control-hit-min);
    }
    .job:disabled { cursor: default; opacity: 0.5; }
    .job .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--text-xs); }
    .job .chip { font-size: var(--text-xs); letter-spacing: 0.15em; border: 1px solid; padding: 0.02rem 0.25rem; flex-shrink: 0; }
    .job .chip.life_safety { color: var(--fire); border-color: var(--fire); }
    .job .chip.evacuation_underway { color: var(--fire); border-color: var(--fire); }
    .job .chip.urgent_objective { color: var(--reloading); border-color: var(--reloading); }
    .job .chip.threat_containment { color: var(--cyan); border-color: var(--cyan); }
    .job .chip.optional { color: var(--ink-dim); border-color: var(--ink-dim); }
    .job .risk { font-size: var(--text-xs); color: var(--ink-dim); flex-shrink: 0; }
    .warning { font-size: var(--text-xs); color: var(--reloading); font-style: italic; }
    .refusal { font-size: var(--text-xs); color: var(--fire); letter-spacing: 0.1em; }
    .refusal[hidden] { display: none; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    @media (orientation: portrait) {
      .blocks { flex-direction: column; }
      .block + .block { border-left: none; padding-left: 0; border-top: 1px solid var(--line-faint); padding-top: 0.5rem; }
    }
  </style>
  <div class="blocks" id="blocks">
    <div class="block teams-block">
      <div class="header"><span>${t('component.security_teams.title')}</span></div>
      <div id="teams"></div>
    </div>
    <div class="block jobs-block">
      <div class="header"><span>${t('component.security_teams.jobs_title')}</span></div>
      <div id="jobs"></div>
    </div>
  </div>
  <div class="refusal" id="refusal" hidden></div>
`;
  }

  onTemplate() {
    this.teamsEl = this.$('teams');
    this.jobsEl = this.$('jobs');
    this.refusalEl = this.$('refusal');
  }

  /**
   * The five team states as labels. Literal `t()` calls, not a composed id, so
   * `check-strings.mjs` sees every row a new state would need.
   */
  static stateLabel(state) {
    switch (state) {
      case 'deploying': return t('component.security_teams.state.deploying');
      case 'working': return t('component.security_teams.state.working');
      case 'withdrawing': return t('component.security_teams.state.withdrawing');
      case 'unavailable': return t('component.security_teams.state.unavailable');
      default: return t('component.security_teams.state.available');
    }
  }

  /** The four generic action verbs as labels. */
  static actionLabel(action) {
    switch (action) {
      case 'secure_contain': return t('component.security_teams.action.secure_contain');
      case 'assist_evacuation': return t('component.security_teams.action.assist_evacuation');
      case 'board': return t('component.security_teams.action.board');
      case 'place_charges': return t('component.security_teams.action.place_charges');
      default: return '';
    }
  }

  /** The five priority classes as labels — the same order the AI ranks on. */
  static priorityLabel(priority) {
    switch (priority) {
      case 'life_safety': return t('component.security_teams.priority.life_safety');
      case 'evacuation_underway': return t('component.security_teams.priority.evacuation_underway');
      case 'urgent_objective': return t('component.security_teams.priority.urgent_objective');
      case 'threat_containment': return t('component.security_teams.priority.threat_containment');
      default: return t('component.security_teams.priority.optional');
    }
  }

  render() {
    const bb = this.state || null;
    const teams = securityTeamRows(bb);
    const jobs = securityActionRows(bb);

    this.teamsEl.textContent = '';
    if (teams.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'empty';
      empty.textContent = t('component.security_teams.no_teams');
      this.teamsEl.appendChild(empty);
    }
    for (const team of teams) {
      const card = document.createElement('div');
      card.className = 'card';
      card.dataset.teamIdx = String(team.teamIdx);

      const top = document.createElement('div');
      top.className = 'card-top';
      const label = document.createElement('span');
      label.className = 'team-label';
      label.textContent = t('component.security_teams.team', { n: String(team.teamIdx + 1) });
      const badge = document.createElement('span');
      badge.className = `status-badge ${team.state}`;
      badge.textContent = PhSecurityTeams.stateLabel(team.state);
      top.appendChild(label);
      top.appendChild(badge);
      card.appendChild(top);

      const assignment = document.createElement('div');
      assignment.className = 'assignment';
      assignment.textContent = team.action
        ? `${team.targetName ? t(team.targetName) : (team.target || '')} · ${PhSecurityTeams.actionLabel(team.action)}`
        : '';
      card.appendChild(assignment);

      const wrap = document.createElement('div');
      wrap.className = 'progress-wrap';
      const fill = document.createElement('div');
      fill.className = `progress-fill ${team.state}`;
      fill.style.width = `${Math.round(Math.max(0, Math.min(1, team.progress)) * 100)}%`;
      wrap.appendChild(fill);
      card.appendChild(wrap);

      if (team.canRecall) {
        const recall = document.createElement('button');
        recall.className = 'job';
        recall.dataset.recall = String(team.teamIdx);
        const name = document.createElement('span');
        name.className = 'name';
        name.textContent = t('component.security_teams.recall');
        recall.appendChild(name);
        recall.addEventListener('click', () => {
          if (this.sendAction) {
            this.sendAction('recall_security_team', { team_idx: team.teamIdx });
          }
        });
        card.appendChild(recall);
      }
      this.teamsEl.appendChild(card);
    }

    // The eligible work. A job is offered only while the server says its target
    // is in reach AND some team is free to take it; the FIRST free team is the
    // one the tap sends, so the panel never has to ask the player which.
    const freeTeam = teams.find((team) => team.canDispatch);
    this.jobsEl.textContent = '';
    if (jobs.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'empty';
      empty.textContent = t('component.security_teams.no_jobs');
      this.jobsEl.appendChild(empty);
    }
    for (const job of jobs) {
      const button = document.createElement('button');
      button.className = 'job';
      button.dataset.target = job.target;
      button.dataset.action = job.action;
      button.disabled = !job.eligible || !freeTeam;

      const name = document.createElement('span');
      name.className = 'name';
      name.textContent = `${job.targetName ? t(job.targetName) : job.target} · ${PhSecurityTeams.actionLabel(job.action)}`;
      const chip = document.createElement('span');
      chip.className = `chip ${job.priority}`;
      chip.textContent = PhSecurityTeams.priorityLabel(job.priority);
      const risk = document.createElement('span');
      risk.className = 'risk';
      risk.textContent = t('component.security_teams.risk', {
        pct: String(Math.round(job.risk * 100)),
        secs: String(Math.round(job.durationSecs)),
      });
      button.appendChild(name);
      button.appendChild(chip);
      button.appendChild(risk);
      button.addEventListener('click', () => {
        if (button.disabled || !freeTeam || !this.sendAction) return;
        this.sendAction('dispatch_security_team', {
          team_idx: freeTeam.teamIdx,
          target: job.target,
          action: job.action,
        });
      });
      this.jobsEl.appendChild(button);

      if (job.warning) {
        const warning = document.createElement('div');
        warning.className = 'warning';
        warning.textContent = t(job.warning);
        this.jobsEl.appendChild(warning);
      }
    }

    const refusal = bb && bb.refusal;
    this.refusalEl.hidden = !refusal;
    this.refusalEl.textContent = refusal ? t(refusal) : '';
  }
}

phDefine('ph-security-teams', PhSecurityTeams);
