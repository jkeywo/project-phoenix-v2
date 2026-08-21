// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';

export class PhRepairTeams extends PhElement {
  // Own state accessors kept (not the base's): `set state` also kicks the
  // progress-bar animation loop, which the base setter has no hook for.
  #state = null;
  #animFrame = null;
  #displayProgress = new Map();
  #emptyEl = null;

  template() {
    return `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .auto-badge { font-size: var(--text-xs); color: var(--reloading); border: 1px solid var(--reloading); padding: 0.05rem 0.3rem; letter-spacing: 0.2em; }
    .card { border: 1px solid var(--line-faint); background: var(--bg-card); padding: 0.5rem; display: flex; flex-direction: column; gap: 0.3rem; }
    .card-top { display: flex; justify-content: space-between; align-items: center; }
    .team-label { font-size: var(--text-xs); font-weight: 600; letter-spacing: 0.15em; }
    .status-badge { font-size: var(--text-xs); padding: 0.05rem 0.3rem; letter-spacing: 0.15em; border: 1px solid; }
    .status-badge.idle { color: var(--ink-dim); border-color: var(--ink-dim); }
    .status-badge.travelling { color: var(--reloading); border-color: var(--reloading); }
    .status-badge.repairing { color: var(--loaded); border-color: var(--loaded); }
    .status-badge.returning { color: var(--cyan); border-color: var(--cyan); }
    .target-label { font-size: var(--text-xs); color: var(--ink-dim); }
    .progress-wrap { width: 100%; height: 0.4rem; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; }
    .progress-fill { height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: none; }
    .progress-fill.repairing { background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); }
    .progress-fill.travelling { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .progress-fill.returning { background: linear-gradient(90deg, var(--cyan-dim), var(--cyan)); }
    .dispatch-row { display: flex; flex-wrap: wrap; gap: 0.3rem; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    /* Damaged-systems list (issue #1015). Rows are buttons: tapping one asks
       the host to make that system the next job of whichever team is already
       sweeping its station. */
    .damaged { display: flex; flex-direction: column; gap: 0.3rem; }
    .damaged[hidden] { display: none; }
    .damaged-list { display: flex; flex-direction: column; gap: 0.15rem; }
    .dmg-row {
      display: flex; align-items: center; gap: 0.4rem; width: 100%;
      background: var(--bg-card); border: 1px solid var(--line-faint);
      color: inherit; font: inherit; text-align: left; cursor: pointer;
      padding: 0.25rem 0.35rem; min-height: var(--control-hit-min);
    }
    .dmg-row:disabled { cursor: default; opacity: 0.5; }
    .dmg-row.prioritised { border-color: var(--cyan); }
    .dmg-row .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--text-xs); }
    .dmg-row .tier-chip { font-size: var(--text-xs); letter-spacing: 0.15em; border: 1px solid; padding: 0.02rem 0.25rem; flex-shrink: 0; }
    .dmg-row .tier-chip.damaged { color: var(--reloading); border-color: var(--reloading); }
    .dmg-row .tier-chip.disabled { color: var(--fire); border-color: var(--fire); }
    .dmg-row .tier-chip.destroyed { color: var(--fire); border-color: var(--fire); background: var(--fire-dim); }
    .dmg-row .pct { font-size: var(--text-xs); color: var(--ink-dim); min-width: 2.4rem; text-align: right; flex-shrink: 0; }
    .dmg-row .flag { font-size: var(--text-xs); letter-spacing: 0.15em; color: var(--cyan); flex-shrink: 0; }
  </style>
  <div class="header">
    <span>${t('component.repair_teams.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <div id="teams-container"></div>
  <div class="damaged" id="damaged" hidden>
    <div class="header"><span>${t('component.repair_teams.damaged_title')}</span></div>
    <div class="damaged-list" id="damaged-list"></div>
  </div>
`;
  }

  connectedCallback() {
    super.connectedCallback();
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

  /**
   * Render the damaged-systems list (issue #1015).
   *
   * `state.damaged` is `RepairConsolePayload.damaged_systems` — the visible hull
   * rows that are broken, already worst-first, each flagged with the host's own
   * verdict (`prioritised`, `in_progress`). Nothing here re-derives any of that:
   * a tap sends the row's `system_id` and the host decides which team, if any,
   * can act on it, so a highlight always reflects a choice the server made.
   *
   * A row is only rendered as a CONTROL when tapping it could actually do
   * something. Rows a team is already on site at are the exception the host's
   * own candidate rule creates, and they are shown disabled — see the comment
   * at the `noop` flag below.
   *
   * The whole section hides when nothing is damaged, so an intact ship's repair
   * panel looks exactly as it did before this list existed.
   */
  #renderDamaged(s, auto) {
    const rows = Array.isArray(s.damaged) ? s.damaged : [];
    const section = this.shadowRoot.getElementById('damaged');
    const list = this.shadowRoot.getElementById('damaged-list');
    section.hidden = rows.length === 0;
    if (rows.length === 0) { list.innerHTML = ''; return; }

    const live = new Set(rows.map(r => r.system_id));
    Array.from(list.children).forEach(child => {
      if (!live.has(child.dataset.systemId)) child.remove();
    });

    rows.forEach((row, idx) => {
      let el = list.querySelector(`[data-system-id="${row.system_id}"]`);
      if (!el) {
        el = document.createElement('button');
        el.type = 'button';
        el.className = 'dmg-row';
        el.dataset.systemId = row.system_id;
        el.innerHTML = '<span class="name"></span><span class="flag"></span><span class="tier-chip"></span><span class="pct"></span>';
        el.addEventListener('click', () => {
          if (el.disabled) return;
          if (this.sendAction) {
            // System-targeted on purpose: the ordinal the sweep consumes is
            // resolved host-side, because #737 hides most of the candidates
            // this console would have to rank. See repair-dispatch.js.
            this.sendAction('set_repair_target_priority', { system_id: row.system_id });
          }
        });
        list.appendChild(el);
      }
      // Keep DOM order in step with the host's worst-first ordering.
      if (list.children[idx] !== el) list.insertBefore(el, list.children[idx] || null);

      const tier = String(row.tier || '').toLowerCase();
      const name = row.display_name || row.system_id;
      // A row a team is already standing on is the one system the host's sweep
      // never offers as a candidate — `sweep_candidates` excludes every on-site
      // system — so a tap on it resolves to nothing at all. Render it (it is
      // real damage, and the [ON IT] flag is the point of showing it) but do not
      // dress it as a control: an enabled button promising "fix this next" that
      // is structurally incapable of doing so is a lie the player pays for in
      // taps. A prioritised row stays live regardless — that highlight IS a
      // choice the host made.
      const noop = !!row.in_progress && !row.prioritised;
      el.className = 'dmg-row' + (row.prioritised ? ' prioritised' : '');
      el.disabled = auto || noop;
      el.querySelector('.name').textContent = name;
      el.querySelector('.tier-chip').className = 'tier-chip ' + tier;
      el.querySelector('.tier-chip').textContent = t('component.repair_teams.tier.' + tier);
      el.querySelector('.pct').textContent = Math.round((row.damage_pct || 0) * 100) + '%';
      const flag = el.querySelector('.flag');
      flag.textContent = row.prioritised
        ? t('component.repair_teams.next')
        : row.in_progress ? t('component.repair_teams.working') : '';
      el.title = row.prioritised
        ? t('component.repair_teams.prioritised_title', { name })
        : noop
          ? t('component.repair_teams.working_title', { name })
          : t('component.repair_teams.prioritise_title', { name });
    });
  }

  #render() {
    const s = this.#state || {};
    const teams = Array.isArray(s.teams) ? s.teams : [];
    const auto = !!s.auto;
    const container = this.shadowRoot.getElementById('teams-container');
    const badge = this.shadowRoot.getElementById('auto-badge');
    badge.style.display = auto ? 'inline' : 'none';

    // Before the empty-teams bail-out: the damage list is a readout in its own
    // right, and a ship with no repair teams still has systems worth naming.
    this.#renderDamaged(s, auto);

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
        // Ordering the team's work is not offered HERE either since issue
        // #1015 — the per-team 1/2/3 ordinal buttons are gone, replaced by the
        // damaged-systems list below, which lets the player name the system
        // instead of guessing its rank in a list the console cannot see.
        drow.style.display = 'none';
        label.style.display = 'block';
        label.textContent = t('component.repair_teams.target', { target: team.target || '—' });
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

phDefine('ph-repair-teams', PhRepairTeams);
