export class PhRepairTeams extends HTMLElement {
  #state = null;
  #animFrame = null;
  #displayProgress = new Map();
  #emptyEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: #6a7178; text-transform: uppercase; }
    .auto-badge { font-size: 0.55rem; color: #f0c040; border: 1px solid #f0c040; padding: 0.05rem 0.3rem; letter-spacing: 0.2em; }
    .card { border: 1px solid #282c38; background: #0e1117; padding: 0.5rem; display: flex; flex-direction: column; gap: 0.3rem; }
    .card-top { display: flex; justify-content: space-between; align-items: center; }
    .team-label { font-size: 0.65rem; font-weight: 600; letter-spacing: 0.15em; }
    .status-badge { font-size: 0.55rem; padding: 0.05rem 0.3rem; letter-spacing: 0.15em; border: 1px solid; }
    .status-badge.idle { color: #6a7178; border-color: #6a7178; }
    .status-badge.travelling { color: #d8a040; border-color: #d8a040; }
    .status-badge.repairing { color: #4ec870; border-color: #4ec870; }
    .status-badge.returning { color: #6090e0; border-color: #6090e0; }
    .target-label { font-size: 0.6rem; color: #6a7178; }
    .progress-wrap { width: 100%; height: 0.4rem; background: #05080e; border: 1px solid #282c38; overflow: hidden; }
    .progress-fill { height: 100%; background: linear-gradient(90deg, #2a6838, #4ec870); transition: none; }
    .progress-fill.repairing { background: linear-gradient(90deg, #2a6838, #4ec870); }
    .progress-fill.travelling { background: linear-gradient(90deg, #805818, #d8a040); }
    .progress-fill.returning { background: linear-gradient(90deg, #184880, #6090e0); }
    .dispatch-btn { font-family: 'Chakra Petch', sans-serif; font-size: 0.55rem; font-weight: 700; padding: 0.2rem 0.5rem; letter-spacing: 0.15em; text-transform: uppercase; cursor: pointer; border: 2px solid #4ec870; color: #4ec870; background: #0e1117; transition: all 0.15s ease; align-self: flex-end; }
    .dispatch-btn:hover:not(:disabled) { background: #16281d; }
    .dispatch-btn:disabled { opacity: 0.35; border-color: #6a7178; color: #6a7178; cursor: default; }
    .empty { font-size: 0.65rem; color: #6a7178; text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header">
    <span>REPAIR TEAMS</span>
    <span class="auto-badge" id="auto-badge" style="display:none">AUTO</span>
  </div>
  <div id="teams-container"></div>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
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
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = 'NO REPAIR TEAMS'; container.appendChild(this.#emptyEl); }
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
          <div class="progress-wrap"><div class="progress-fill" style="width:0%"></div></div>
          <button class="dispatch-btn">DISPATCH</button>
        `;
        const btn = card.querySelector('.dispatch-btn');
        btn.addEventListener('click', () => {
          if (auto || btn.disabled) return;
          if (this.sendAction) {
            const st = this.#state || {};
            const teamsArr = Array.isArray(st.teams) ? st.teams : [];
            const t = teamsArr.find(x => x.id === team.id);
            this.sendAction('dispatch_repair_team', { team_idx: team.id, target: t && t.target ? t.target : null });
          }
        });
        if (idx < container.children.length) {
          container.insertBefore(card, container.children[idx]);
        } else {
          container.appendChild(card);
        }
      }

      const status = team.status || 'idle';
      card.querySelector('.team-label').textContent = team.label || 'Team ' + team.id;
      const badgeEl = card.querySelector('.status-badge');
      badgeEl.textContent = status.toUpperCase();
      badgeEl.className = 'status-badge ' + status;

      const targetName = team.target || '—';
      card.querySelector('.target-label').textContent = status === 'idle' ? 'Standing by' : 'Target: ' + targetName;

      const fill = card.querySelector('.progress-fill');
      fill.className = 'progress-fill ' + status;
      const targetPct = team.progress_pct != null ? team.progress_pct : 0;
      if (!this.#displayProgress.has(team.id)) {
        this.#displayProgress.set(team.id, targetPct);
      }
      fill.style.width = Math.round(targetPct * 100) + '%';

      const btn = card.querySelector('.dispatch-btn');
      btn.disabled = auto || status === 'idle';
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-repair-teams')) {
  customElements.define('ph-repair-teams', PhRepairTeams);
}
