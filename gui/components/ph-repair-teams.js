import { phAdoptConsoleStyles } from './ph-console-styles.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';

export class PhRepairTeams extends HTMLElement {
  #state = null;
  #animFrame = null;
  #displayProgress = new Map();
  #emptyEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .auto-badge { font-size: 0.55rem; color: var(--reloading); border: 1px solid var(--reloading); padding: 0.05rem 0.3rem; letter-spacing: 0.2em; }
    .card { border: 1px solid var(--line-faint); background: var(--bg-card); padding: 0.5rem; display: flex; flex-direction: column; gap: 0.3rem; }
    .card-top { display: flex; justify-content: space-between; align-items: center; }
    .team-label { font-size: 0.65rem; font-weight: 600; letter-spacing: 0.15em; }
    .status-badge { font-size: 0.55rem; padding: 0.05rem 0.3rem; letter-spacing: 0.15em; border: 1px solid; }
    .status-badge.idle { color: var(--ink-dim); border-color: var(--ink-dim); }
    .status-badge.travelling { color: var(--reloading); border-color: var(--reloading); }
    .status-badge.repairing { color: var(--loaded); border-color: var(--loaded); }
    .status-badge.returning { color: var(--cyan); border-color: var(--cyan); }
    .target-label { font-size: 0.6rem; color: var(--ink-dim); }
    .progress-wrap { width: 100%; height: 0.4rem; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; }
    .progress-fill { height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: none; }
    .progress-fill.repairing { background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); }
    .progress-fill.travelling { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .progress-fill.returning { background: linear-gradient(90deg, var(--cyan-dim), var(--cyan)); }
    .dispatch-row { display: flex; flex-wrap: wrap; gap: 0.3rem; }
    .empty { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header">
    <span>${t('component.repair_teams.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <div id="teams-container"></div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
    phAdoptConsoleStyles(this.shadowRoot);
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
  }

  disconnectedCallback() {
    if (this.#animFrame) {
      cancelAnimationFrame(this.#animFrame);
      this.#animFrame = null;
    }
  }

  set state(val) {
    this.#state = val;
    this.#render();
    if (val) this.#startAnimLoop();
  }

  get state() { return this.#state; }

  #startAnimLoop() {
    if (this.#animFrame) return;
    const step = () => {
      const s = this.#state || {};
      const teams = Array.isArray(s.teams) ? s.teams : [];
      let needsUpdate = false;
      teams.forEach(t => {
        const target = t.progress_pct != null ? t.progress_pct : 0;
        const current = this.#displayProgress.get(t.id) ?? target;
        if (Math.abs(current - target) > 0.001) {
          const next = current + (target - current) * 0.15;
          this.#displayProgress.set(t.id, next);
          needsUpdate = true;
        } else {
          this.#displayProgress.set(t.id, target);
        }
      });
      if (needsUpdate) {
        const container = this.shadowRoot.getElementById('teams-container');
        teams.forEach(t => {
          const bar = container.querySelector(`[data-team-id="${t.id}"] .progress-fill`);
          if (bar) {
            const disp = this.#displayProgress.get(t.id) ?? 0;
            bar.style.width = Math.round(disp * 100) + '%';
          }
        });
      }
      this.#animFrame = requestAnimationFrame(step);
    };
    this.#animFrame = requestAnimationFrame(step);
  }

  #render() {
    const s = this.#state || {};
    const teams = Array.isArray(s.teams) ? s.teams : [];
    const auto = !!s.auto;
    const container = this.shadowRoot.getElementById('teams-container');
    const badge = this.shadowRoot.getElementById('auto-badge');
    badge.style.display = auto ? 'inline' : 'none';

    if (teams.length === 0) {
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = t('component.repair_teams.empty'); container.appendChild(this.#emptyEl); }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    const newIds = new Set(teams.map(t => t.id));
    Array.from(container.children).forEach(child => {
      if (!newIds.has(Number(child.dataset.teamId))) {
        child.remove();
      }
    });

    teams.forEach((team, idx) => {
      let card = container.querySelector(`[data-team-id="${team.id}"]`);
      if (!card) {
        card = document.createElement('div');
        card.className = 'card';
        card.dataset.teamId = team.id;
        card.innerHTML = `
          <div class="card-top">
            <span class="team-label"></span>
            <span class="status-badge"></span>
          </div>
          <span class="target-label"></span>
          <div class="dispatch-row"></div>
          <div class="progress-wrap"><div class="progress-fill" style="width:0%"></div></div>
        `;
        if (idx < container.children.length) {
          container.insertBefore(card, container.children[idx]);
        } else {
          container.appendChild(card);
        }
      }

      const status = team.status || 'idle';
      card.querySelector('.team-label').textContent = team.label || t('component.repair_teams.team', { n: team.id });
      const badgeEl = card.querySelector('.status-badge');
      badgeEl.textContent = t('component.repair_teams.status.' + status);
      badgeEl.className = 'status-badge ' + status;

      const isIdle = status === 'idle';
      // Every station that owns a damageable system gets a target/dispatch
      // entry regardless of current damage, so teams can be pre-positioned.
      const targets = Array.isArray(s.targets) ? s.targets : [];
      const label = card.querySelector('.target-label');
      const drow = card.querySelector('.dispatch-row');

      if (isIdle) {
        const hasTargets = targets.length > 0;
        label.style.display = hasTargets ? 'none' : 'block';
        if (!hasTargets) label.textContent = t('component.repair_teams.no_targets');
        drow.style.display = hasTargets ? 'flex' : 'none';

        const sig = targets.map(t => t.id).join('|');
        if (drow.dataset.sig !== sig) {
          drow.innerHTML = '';
          targets.forEach(t => {
            const b = document.createElement('button');
            b.type = 'button';
            b.className = 'btn armed';
            b.dataset.target = t.id;
            b.innerHTML = '<span class="btn-bg"></span><span class="led on"></span><span class="label"></span>';
            b.querySelector('.label').textContent = t.label;
            b.addEventListener('click', () => {
              if (b.disabled) return;
              if (this.sendAction) {
                // target is a station id (lowercase) or 'core'; action-map
                // wraps it into RepairTarget::{Station|Core}.
                this.sendAction('dispatch_repair_team', { team_idx: team.id, target: t.id });
              }
            });
            drow.appendChild(b);
          });
          drow.dataset.sig = sig;
        }
        drow.querySelectorAll('.btn').forEach(b => { b.disabled = auto; });
      } else {
        // Busy team: show its current target; dispatch is not offered.
        drow.style.display = 'none';
        label.style.display = 'block';
        label.textContent = t('component.repair_teams.target', { target: team.target || '—' });

        // Priority controls only for on-site (repairing) teams (issue #739).
        const isRepairing = status === 'repairing';
        let priorityRow = card.querySelector('.priority-row');
        if (isRepairing) {
          if (!priorityRow) {
            priorityRow = document.createElement('div');
            priorityRow.className = 'priority-row';
            priorityRow.style.cssText = 'display:flex;gap:0.3rem;align-items:center;margin-top:0.2rem;';
            priorityRow.innerHTML = `
              <span style="font-size:0.55rem;letter-spacing:0.15em;color:var(--ink-dim);">${t('component.repair_teams.priority')}</span>
              <button type="button" class="btn priority-btn" data-priority="0" style="font-size:0.55rem;padding:0.1rem 0.3rem;">1</button>
              <button type="button" class="btn priority-btn" data-priority="1" style="font-size:0.55rem;padding:0.1rem 0.3rem;">2</button>
              <button type="button" class="btn priority-btn" data-priority="2" style="font-size:0.55rem;padding:0.1rem 0.3rem;">3</button>
            `;
            priorityRow.querySelectorAll('.priority-btn').forEach(btn => {
              btn.addEventListener('click', () => {
                if (btn.disabled || auto) return;
                if (this.sendAction) {
                  this.sendAction('set_repair_priority', { team_idx: team.id, priority: parseInt(btn.dataset.priority) + 1 });
                }
              });
            });
            card.appendChild(priorityRow);
          }
          priorityRow.style.display = auto ? 'none' : 'flex';
          priorityRow.querySelectorAll('.priority-btn').forEach(btn => {
            btn.disabled = auto;
          });
        } else if (priorityRow) {
          priorityRow.style.display = 'none';
        }
      }

      const fill = card.querySelector('.progress-fill');
      fill.className = 'progress-fill ' + status;
      const targetPct = team.progress_pct != null ? team.progress_pct : 0;
      if (!this.#displayProgress.has(team.id)) {
        this.#displayProgress.set(team.id, targetPct);
      }
      fill.style.width = Math.round(targetPct * 100) + '%';
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-repair-teams')) {
  customElements.define('ph-repair-teams', PhRepairTeams);
}
