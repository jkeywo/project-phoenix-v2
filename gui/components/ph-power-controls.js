import { phAdoptConsoleStyles } from './ph-console-styles.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';

/**
 * The lowest rung to draw for a group whose entry carries no `min_level`.
 *
 * Since issue #1004 the server publishes the authored floor on every
 * `PowerGroupEntry`, so this only stands in for a pre-#1004 payload — and there
 * the right answer is 1, not 0. The engine has always clamped every group to
 * `GROUP_LEVEL_MIN` (= 1); a 0 here drew a bottom pip no order could ever light,
 * which is what put nine lights on a three-group console instead of twelve. Same
 * number as `ship::config::default_min_power_level`, which is also the Rust
 * decoder's `#[serde(default)]` for the field.
 */
const DEFAULT_MIN_LEVEL = 1;

export class PhPowerControls extends HTMLElement {
  #state = null;
  #pipCache = new Map();
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
    .group { border: 1px solid var(--line-faint); background: var(--bg-card); padding: 0.5rem; display: flex; flex-direction: column; gap: 0.4rem; }
    .group-top { display: flex; justify-content: space-between; align-items: center; }
    .group-label { font-size: 0.65rem; font-weight: 600; letter-spacing: 0.2em; color: var(--ink); }
    .pip-row { display: flex; align-items: center; gap: 0.4rem; justify-content: center; }
    .pip { width: 1.2rem; height: 1.2rem; border-radius: 50%; border: 2px solid var(--line-faint); background: var(--bg-deep); cursor: pointer; transition: all 0.15s ease; }
    .pip:hover:not(.disabled) { border-color: var(--ink-dim); }
    .pip.active { background: var(--loaded); border-color: var(--loaded); box-shadow: 0 0 6px rgba(78,200,112,0.5); }
    .pip.inactive { background: transparent; border-color: var(--line-faint); }
    /* A rung the order asks for that the reactor's battery floor is refusing:
       outlined in the reloading amber, hollow, so the officer can see the gap
       between what was commanded and what is running. */
    .pip.held { background: transparent; border-color: var(--reloading); box-shadow: none; }
    .pip.disabled { cursor: default; opacity: 0.3; }
    .level-text.held { color: var(--reloading); }
    .pip-btn-row { display: flex; align-items: center; gap: 0.5rem; justify-content: center; }
    .level-text { font-size: 0.6rem; color: var(--ink-dim); letter-spacing: 0.1em; min-width: 1.5rem; text-align: center; }
    .empty { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header">
    <span>${t('component.power.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <div id="groups-container"></div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
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
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = t('component.power.empty'); container.appendChild(this.#emptyEl); }
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
          const min = group.min_level != null ? group.min_level : DEFAULT_MIN_LEVEL;
          if (cur > min && this.sendAction) {
            this.sendAction('set_power', { target: gid, level: cur - 1 });
          }
        });
        // NB: both handlers step from `#currentLevel`, which is the COMMANDED
        // level, not the effective one — see that method.
        if (idx < container.children.length) {
          container.insertBefore(el, container.children[idx]);
        } else {
          container.appendChild(el);
        }
      }

      // EFFECTIVE (what the group is running at) vs COMMANDED (the standing
      // order). They differ while the reactor's battery floor is holding the
      // group down. The pips light the effective level, the rungs between the
      // two are shown as held, and every CONTROL works off the commanded one.
      const level = group.level != null ? group.level : 0;
      const commanded = commandedLevel(group);
      const held = commanded > level;
      const minLevel = group.min_level != null ? group.min_level : DEFAULT_MIN_LEVEL;
      const maxLevel = group.max_level != null ? group.max_level : 4;

      el.querySelector('.group-label').textContent = group.label || group.id;
      const levelText = el.querySelector('.level-text');
      levelText.textContent = held
        ? t('component.power.held', { n: level, c: commanded })
        : t('component.power.level', { n: level });
      levelText.classList.toggle('held', held);

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
        let stateClass = ' inactive';
        if (i <= level) stateClass = ' active';
        else if (i <= commanded) stateClass = ' held';
        pip.className = 'pip' + stateClass + (auto ? ' disabled' : '');
      }
      for (const [key, pip] of this.#pipCache) {
        if (key.startsWith(gid + ':') && !livePips.has(key)) { pip.remove(); this.#pipCache.delete(key); }
      }

      const incrBtn = el.querySelector('.mini-btn[data-action="incr"]');
      const decrBtn = el.querySelector('.mini-btn[data-action="decr"]');
      // Gated on the COMMANDED level for the same reason the handlers step
      // from it: a floored group whose order is already at the cap must not
      // offer a `+` that would silently be a no-op, and one whose order is
      // above its minimum must still offer a `−`.
      incrBtn.disabled = auto || commanded >= maxLevel;
      decrBtn.disabled = auto || commanded <= minLevel;
    });
  }

  /**
   * The level the `+`/`−` buttons step from: the group's COMMANDED level.
   *
   * `set_power` carries an ABSOLUTE level and the server measures the delta
   * against the standing order, so stepping from the effective level is a bug
   * whenever a battery floor is holding the group down (issue #952). Helm
   * commanded 4 and floored to 2: `+` would send 3, which the server reads as
   * a DECREASE — the panel would not move, a second press would re-send the
   * same 3, and `−` would send 1 and collapse the order outright.
   *
   * Falls back to `level` when `commanded_level` is absent or 0, which is what
   * a pre-#952 server sends.
   */
  #currentLevel(groupId) {
    const s = this.#state || {};
    const groups = Array.isArray(s.groups) ? s.groups : [];
    const g = groups.find(x => x.id === groupId);
    return g ? commandedLevel(g) : 0;
  }
}

/** @see PhPowerControls#currentLevel — the commanded level, with its fallback. */
function commandedLevel(group) {
  if (!group) return 0;
  if (group.commanded_level != null && group.commanded_level > 0) return group.commanded_level;
  return group.level != null ? group.level : 0;
}

if (typeof window !== 'undefined' && !customElements.get('ph-power-controls')) {
  customElements.define('ph-power-controls', PhPowerControls);
}
