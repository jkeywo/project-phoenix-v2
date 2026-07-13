import { phAdoptConsoleStyles } from './ph-console-styles.js';

export class PhPowerControls extends HTMLElement {
  #state = null;
  #pipCache = new Map();
  #emptyEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .auto-badge { font-size: 0.55rem; color: var(--reloading); border: 1px solid var(--reloading); padding: 0.05rem 0.3rem; letter-spacing: 0.2em; }
    .group { border: 1px solid var(--line-faint); background: var(--bg-card); padding: 0.5rem; display: flex; flex-direction: column; gap: 0.4rem; }
    .group-top { display: flex; justify-content: space-between; align-items: center; }
    .group-label { font-size: 0.65rem; font-weight: 600; letter-spacing: 0.2em; color: var(--ink); }
    .pip-row { display: flex; align-items: center; gap: 0.4rem; justify-content: center; }
    .pip { width: 1.2rem; height: 1.2rem; border-radius: 50%; border: 2px solid var(--line-faint); background: var(--bg-deep); cursor: pointer; transition: all 0.15s ease; }
    .pip:hover:not(.disabled) { border-color: var(--ink-dim); }
    .pip.active { background: var(--loaded); border-color: var(--loaded); box-shadow: 0 0 6px rgba(78,200,112,0.5); }
    .pip.inactive { background: transparent; border-color: var(--line-faint); }
    .pip.disabled { cursor: default; opacity: 0.3; }
    .pip-btn-row { display: flex; align-items: center; gap: 0.5rem; justify-content: center; }
    .level-text { font-size: 0.6rem; color: var(--ink-dim); letter-spacing: 0.1em; min-width: 1.5rem; text-align: center; }
    .empty { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header">
    <span>POWER ALLOCATION</span>
    <span class="auto-badge" id="auto-badge" style="display:none">AUTO</span>
  </div>
  <div id="groups-container"></div>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
    phAdoptConsoleStyles(this.shadowRoot);
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
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = 'NO POWER GROUPS'; container.appendChild(this.#emptyEl); }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    const newIds = new Set(groups.map(g => g.id));
    Array.from(container.children).forEach(child => {
      if (!newIds.has(child.dataset.groupId)) {
        child.remove();
        const gid = child.dataset.groupId;
        for (const key of this.#pipCache.keys()) {
          if (key.startsWith(gid + ':')) this.#pipCache.delete(key);
        }
      }
    });

    groups.forEach((group, idx) => {
      const gid = group.id;
      let el = container.querySelector(`[data-group-id="${gid}"]`);
      if (!el) {
        el = document.createElement('div');
        el.className = 'group';
        el.dataset.groupId = gid;
        el.innerHTML = `
          <div class="group-top">
            <span class="group-label"></span>
            <span class="level-text"></span>
          </div>
          <div class="pip-row"></div>
          <div class="pip-btn-row">
            <button type="button" class="mini-btn" data-action="decr"><span class="mini-bg"></span><span class="lbl">−</span></button>
            <button type="button" class="mini-btn" data-action="incr"><span class="mini-bg"></span><span class="lbl">+</span></button>
          </div>
        `;
        const pipRow = el.querySelector('.pip-row');
        pipRow.addEventListener('click', e => {
          const pip = e.target.closest('.pip');
          if (!pip || auto) return;
          const level = Number(pip.dataset.level);
          if (!isNaN(level) && this.sendAction) {
            this.sendAction('set_power', { target: gid, level });
          }
        });
        const incrBtn = el.querySelector('.mini-btn[data-action="incr"]');
        const decrBtn = el.querySelector('.mini-btn[data-action="decr"]');
        incrBtn.addEventListener('click', () => {
          if (auto) return;
          const cur = this.#currentLevel(gid);
          const max = group.max_level != null ? group.max_level : 4;
          if (cur < max && this.sendAction) {
            this.sendAction('set_power', { target: gid, level: cur + 1 });
          }
        });
        decrBtn.addEventListener('click', () => {
          if (auto) return;
          const cur = this.#currentLevel(gid);
          const min = group.min_level != null ? group.min_level : 0;
          if (cur > min && this.sendAction) {
            this.sendAction('set_power', { target: gid, level: cur - 1 });
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

      const livePips = new Set();
      for (let i = minLevel; i <= maxLevel; i++) {
        const key = gid + ':' + i;
        livePips.add(key);
        let pip = this.#pipCache.get(key);
        if (!pip) {
          pip = document.createElement('div');
          pip.className = 'pip';
          pip.dataset.level = i;
          this.#pipCache.set(key, pip);
          pipRow.appendChild(pip);
        }
        pip.className = 'pip' + (i <= level ? ' active' : ' inactive') + (auto ? ' disabled' : '');
      }
      for (const [key, pip] of this.#pipCache) {
        if (key.startsWith(gid + ':') && !livePips.has(key)) { pip.remove(); this.#pipCache.delete(key); }
      }

      const incrBtn = el.querySelector('.mini-btn[data-action="incr"]');
      const decrBtn = el.querySelector('.mini-btn[data-action="decr"]');
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
