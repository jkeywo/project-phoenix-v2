export class PhPowerControls extends HTMLElement {
  #state = null;

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
    .group { border: 1px solid #282c38; background: #0e1117; padding: 0.5rem; display: flex; flex-direction: column; gap: 0.4rem; }
    .group-top { display: flex; justify-content: space-between; align-items: center; }
    .group-label { font-size: 0.65rem; font-weight: 600; letter-spacing: 0.2em; color: #cce; }
    .pip-row { display: flex; align-items: center; gap: 0.4rem; justify-content: center; }
    .pip { width: 1.2rem; height: 1.2rem; border-radius: 50%; border: 2px solid #282c38; background: #05080e; cursor: pointer; transition: all 0.15s ease; }
    .pip:hover:not(.disabled) { border-color: #6a7178; }
    .pip.active { background: #4ec870; border-color: #4ec870; box-shadow: 0 0 6px rgba(78,200,112,0.5); }
    .pip.inactive { background: transparent; border-color: #282c38; }
    .pip.disabled { cursor: default; opacity: 0.3; }
    .pip-btn-row { display: flex; align-items: center; gap: 0.5rem; justify-content: center; }
    .step-btn { font-family: 'Chakra Petch', sans-serif; font-size: 0.8rem; font-weight: 700; padding: 0.15rem 0.6rem; cursor: pointer; border: 2px solid #4ec870; color: #4ec870; background: #0e1117; transition: all 0.15s ease; }
    .step-btn:hover:not(:disabled) { background: #16281d; }
    .step-btn:disabled { opacity: 0.3; border-color: #6a7178; color: #6a7178; cursor: default; }
    .level-text { font-size: 0.6rem; color: #6a7178; letter-spacing: 0.1em; min-width: 1.5rem; text-align: center; }
    .empty { font-size: 0.65rem; color: #6a7178; text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header">
    <span>POWER ALLOCATION</span>
    <span class="auto-badge" id="auto-badge" style="display:none">AUTO</span>
  </div>
  <div id="groups-container"></div>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    const groups = Array.isArray(s.groups) ? s.groups : [];
    const auto = !!s.auto;
    const container = this.shadowRoot.getElementById('groups-container');
    const badge = this.shadowRoot.getElementById('auto-badge');
    badge.style.display = auto ? 'inline' : 'none';

    if (groups.length === 0) {
      container.innerHTML = '<div class="empty">NO POWER GROUPS</div>';
      return;
    }

    const newIds = new Set(groups.map(g => g.id));
    Array.from(container.children).forEach(child => {
      if (!newIds.has(child.dataset.groupId)) {
        child.remove();
      }
    });

    groups.forEach((group, idx) => {
      let el = container.querySelector(`[data-group-id="${group.id}"]`);
      if (!el) {
        el = document.createElement('div');
        el.className = 'group';
        el.dataset.groupId = group.id;
        el.innerHTML = `
          <div class="group-top">
            <span class="group-label"></span>
            <span class="level-text"></span>
          </div>
          <div class="pip-row"></div>
          <div class="pip-btn-row">
            <button class="step-btn" data-action="decr">−</button>
            <button class="step-btn" data-action="incr">+</button>
          </div>
        `;
        const pipRow = el.querySelector('.pip-row');
        pipRow.addEventListener('click', e => {
          const pip = e.target.closest('.pip');
          if (!pip || auto) return;
          const level = Number(pip.dataset.level);
          if (!isNaN(level) && this.sendAction) {
            this.sendAction('set_power', { group_id: group.id, level });
          }
        });
        const incrBtn = el.querySelector('.step-btn[data-action="incr"]');
        const decrBtn = el.querySelector('.step-btn[data-action="decr"]');
        incrBtn.addEventListener('click', () => {
          if (auto) return;
          const cur = this.#currentLevel(group.id);
          const max = group.max_level != null ? group.max_level : 4;
          if (cur < max && this.sendAction) {
            this.sendAction('set_power', { group_id: group.id, level: cur + 1 });
          }
        });
        decrBtn.addEventListener('click', () => {
          if (auto) return;
          const cur = this.#currentLevel(group.id);
          const min = group.min_level != null ? group.min_level : 0;
          if (cur > min && this.sendAction) {
            this.sendAction('set_power', { group_id: group.id, level: cur - 1 });
          }
        });
        if (idx < container.children.length) {
          container.insertBefore(el, container.children[idx]);
        } else {
          container.appendChild(el);
        }
      }

      const level = group.level != null ? group.level : 0;
      const minLevel = group.min_level != null ? group.min_level : 0;
      const maxLevel = group.max_level != null ? group.max_level : 4;

      el.querySelector('.group-label').textContent = group.label || group.id;
      el.querySelector('.level-text').textContent = 'LVL ' + level;

      const pipRow = el.querySelector('.pip-row');
      pipRow.innerHTML = '';
      for (let i = minLevel; i <= maxLevel; i++) {
        const pip = document.createElement('div');
        pip.className = 'pip' + (i <= level ? ' active' : ' inactive') + (auto ? ' disabled' : '');
        pip.dataset.level = i;
        pipRow.appendChild(pip);
      }

      const incrBtn = el.querySelector('.step-btn[data-action="incr"]');
      const decrBtn = el.querySelector('.step-btn[data-action="decr"]');
      incrBtn.disabled = auto || level >= maxLevel;
      decrBtn.disabled = auto || level <= minLevel;
    });
  }

  #currentLevel(groupId) {
    const s = this.#state || {};
    const groups = Array.isArray(s.groups) ? s.groups : [];
    const g = groups.find(x => x.id === groupId);
    return g ? (g.level != null ? g.level : 0) : 0;
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-power-controls')) {
  customElements.define('ph-power-controls', PhPowerControls);
}
