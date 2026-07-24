// Torpedo tube controls: per-tube volley-target UI (issue #632/#637 model).
//
// Each tube shows one slot icon per possible round (up to volley_max). Slots
// are grey when empty, fill green as a round loads, solid green when loaded,
// and drain back to grey as a round unloads. A "-" button lowers the target
// load count, "+" raises it (the tube auto-loads/unloads toward the target
// one round at a time), and "FIRE" launches whatever is currently loaded.
//
// This mirrors gui/weapons-console.html's tube UI but is a standalone custom
// element so it can be embedded in the per-hull tactical consoles
// (destroyer/tactical.html, cruiser/tactical.html).
import { phAdoptConsoleStyles } from './ph-console-styles.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { weaponReadinessView } from '../weapon-readiness.js';

export class PhTorpedoControls extends HTMLElement {
  #state = null;
  #tubeEls = {};

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .magazine { font-size: 0.9rem; color: var(--tactical); font-weight: 600; font-family: 'Chakra Petch', sans-serif; }
    .tube-row { display: flex; align-items: center; gap: 0.5rem; font-size: 0.65rem; padding: 0.3rem 0; }
    .tube-row + .tube-row { border-top: 1px solid var(--line-faint); }
    .tube-row .lbl { min-width: 4rem; color: var(--ink-dim); flex-shrink: 0; }
    .tube-row .status { font-size: 0.5rem; letter-spacing: 0.15em; color: var(--ink-dim); flex-shrink: 0; }
    .tube-row.blocked .status { color: var(--fire); }
    .tube-row.unavailable .status { color: var(--ink-faint); }
    .tube-row.ready .status { color: var(--loaded); }
    .tube-controls { display: flex; align-items: center; gap: 0.4rem; margin-left: auto; }
    .torp-slots { display: flex; gap: 0.2rem; align-items: center; }
    .torp-slot {
      position: relative; width: 0.85rem; height: 1.4rem; border-radius: 2px; overflow: hidden; flex-shrink: 0;
      background: rgba(255,255,255,0.10); border: 1px solid rgba(255,255,255,0.20);
    }
    .torp-slot[data-state="queued-to-fill"], .torp-slot[data-state="loading"] {
      border-color: rgba(78,200,112,0.55); border-style: dashed;
    }
    .torp-slot[data-state="filled"] {
      border-color: var(--loaded); box-shadow: 0 0 4px rgba(78,200,112,0.5);
    }
    .torp-slot[data-state="queued-to-empty"] {
      border-color: rgba(255,255,255,0.45); border-style: dashed;
    }
    .torp-slot .fill {
      position: absolute; bottom: 0; left: 0; right: 0; height: 0%;
      background: linear-gradient(0deg, var(--loaded) 0%, #7ee29a 100%);
      transition: height 0.15s linear;
    }
    .pattern-row { display: flex; gap: 0.5rem; padding-left: 4.5rem; font-size: 0.5rem; letter-spacing: 0.15em; color: var(--reloading); }
    .pattern-row.idle { display: none; }
    .empty { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header">
    <span>${t('component.torpedoes.title')}</span>
    <span class="magazine" id="magazine">0 / 0</span>
  </div>
  <div id="tubes"></div>
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

  #buildTubeRow(tubeId) {
    const row = document.createElement('div');
    row.className = 'tube-row';
    row.dataset.id = tubeId;

    const lbl = document.createElement('span');
    lbl.className = 'lbl';
    row.appendChild(lbl);

    const status = document.createElement('span');
    status.className = 'status';
    row.appendChild(status);

    const slotsEl = document.createElement('div');
    slotsEl.className = 'torp-slots';

    const minusBtn = document.createElement('button');
    minusBtn.type = 'button';
    minusBtn.className = 'mini-btn';
    minusBtn.innerHTML = '<span class="mini-bg"></span><span class="lbl">−</span>';
    minusBtn.addEventListener('click', () => {
      const tube = (this.#state && this.#state.tubes || []).find((x) => x.id === tubeId);
      const cur = tube && typeof tube.target_count === 'number' ? tube.target_count : 0;
      if (cur > 0 && this.sendAction) this.sendAction('set_torpedo_volley_target', { tube: tubeId, count: cur - 1 });
    });

    const plusBtn = document.createElement('button');
    plusBtn.type = 'button';
    plusBtn.className = 'mini-btn';
    plusBtn.innerHTML = '<span class="mini-bg"></span><span class="lbl">+</span>';
    plusBtn.addEventListener('click', () => {
      const tube = (this.#state && this.#state.tubes || []).find((x) => x.id === tubeId);
      const cur = tube && typeof tube.target_count === 'number' ? tube.target_count : 0;
      const max = tube && typeof tube.volley_max === 'number' ? tube.volley_max : 1;
      if (cur < max && this.sendAction) this.sendAction('set_torpedo_volley_target', { tube: tubeId, count: cur + 1 });
    });

    const fireBtn = document.createElement('button');
    fireBtn.type = 'button';
    fireBtn.className = 'btn';
    fireBtn.innerHTML = '<span class="btn-bg"></span><span class="led"></span><span class="label">' + t('console.common.fire') + '</span>';
    fireBtn.addEventListener('click', () => {
      if (fireBtn.disabled || !this.sendAction) return;
      const targetUuid = this.#state && this.#state.target_uuid ? this.#state.target_uuid : null;
      this.sendAction('fire_torpedo', { tube: tubeId, target_uuid: targetUuid });
    });

    const controls = document.createElement('div');
    controls.className = 'tube-controls';
    controls.appendChild(minusBtn);
    controls.appendChild(slotsEl);
    controls.appendChild(plusBtn);
    controls.appendChild(fireBtn);
    row.appendChild(controls);

    // Patterned-attack indicator (issue #766): current pattern step + active
    // barrels. Hidden unless the tube has a multi-barrel pattern.
    const patternRow = document.createElement('div');
    patternRow.className = 'pattern-row idle';
    const stepEl = document.createElement('span');
    stepEl.className = 'pattern-step';
    const barrelsEl = document.createElement('span');
    barrelsEl.className = 'pattern-barrels';
    patternRow.appendChild(stepEl);
    patternRow.appendChild(barrelsEl);
    row.appendChild(patternRow);

    return { row, lbl, status, slotsEl, minusBtn, plusBtn, fireBtn, slotEls: [], patternRow };
  }

  // Rebuild slot <div> elements when volley_max changes.
  #ensureSlotEls(els, vollMax) {
    if (els.slotEls.length === vollMax) return els.slotEls;
    while (els.slotsEl.firstChild) els.slotsEl.removeChild(els.slotsEl.firstChild);
    els.slotEls = [];
    for (let i = 0; i < vollMax; i++) {
      const slotEl = document.createElement('div');
      slotEl.className = 'torp-slot';
      slotEl.dataset.state = 'empty';
      const fillEl = document.createElement('div');
      fillEl.className = 'fill';
      slotEl.appendChild(fillEl);
      els.slotsEl.appendChild(slotEl);
      els.slotEls.push(slotEl);
    }
    return els.slotEls;
  }

  // Per-slot state: 'empty' | 'queued-to-fill' | 'loading' | 'filled' |
  // 'queued-to-empty' | 'unloading', with a 0..1 green-fill fraction.
  #computeSlots(tube) {
    const loadedCount = typeof tube.loaded_count === 'number' ? tube.loaded_count : (tube.loaded ? 1 : 0);
    const targetCount = typeof tube.target_count === 'number' ? tube.target_count : 0;
    const vollMax = typeof tube.volley_max === 'number' ? tube.volley_max : 1;
    const loadProg = typeof tube.load_progress === 'number' ? tube.load_progress : 0;
    const tubeState = tube.state || (tube.loaded ? 'loaded' : 'unloaded');
    const activeIdx = tubeState === 'loading' ? loadedCount : (tubeState === 'unloading' ? loadedCount - 1 : -1);

    const slots = [];
    for (let i = 0; i < vollMax; i++) {
      let state;
      let fill;
      if (i < loadedCount) {
        if (i === activeIdx && tubeState === 'unloading') {
          state = 'unloading';
          fill = 1 - loadProg;
        } else {
          state = i >= targetCount ? 'queued-to-empty' : 'filled';
          fill = 1;
        }
      } else if (i === activeIdx && tubeState === 'loading') {
        state = 'loading';
        fill = loadProg;
      } else {
        state = i < targetCount ? 'queued-to-fill' : 'empty';
        fill = 0;
      }
      slots.push({ state, fill });
    }
    return slots;
  }

  #render() {
    const s = this.#state || {};
    const tubes = Array.isArray(s.tubes) ? s.tubes : [];
    const mag = s.magazine || {};
    const magCurrent = mag.current != null ? mag.current : 0;
    const magMax = mag.max != null ? mag.max : 0;

    this.shadowRoot.getElementById('magazine').textContent = magCurrent + ' / ' + magMax;

    const container = this.shadowRoot.getElementById('tubes');

    if (tubes.length === 0) {
      container.innerHTML = '<div class="empty">' + t('component.torpedoes.empty') + '</div>';
      this.#tubeEls = {};
      return;
    }
    if (container.querySelector('.empty')) container.innerHTML = '';

    const seenIds = {};
    tubes.forEach((t) => { seenIds[t.id] = true; });
    Object.keys(this.#tubeEls).forEach((id) => {
      if (!seenIds[id]) {
        this.#tubeEls[id].row.remove();
        delete this.#tubeEls[id];
      }
    });

    tubes.forEach((tube) => {
      let els = this.#tubeEls[tube.id];
      if (!els) {
        els = this.#buildTubeRow(tube.id);
        this.#tubeEls[tube.id] = els;
        container.appendChild(els.row);
      }

      els.lbl.textContent = String(tube.label || tube.id).replace(/_/g, ' ').toUpperCase();

      const vollMax = typeof tube.volley_max === 'number' ? tube.volley_max : 1;
      const slotEls = this.#ensureSlotEls(els, vollMax);
      const slots = this.#computeSlots(tube);
      slots.forEach((slot, i) => {
        const slotEl = slotEls[i];
        slotEl.dataset.state = slot.state;
        slotEl.firstChild.style.height = Math.round(slot.fill * 100) + '%';
      });

      const loadedCount = typeof tube.loaded_count === 'number' ? tube.loaded_count : (tube.loaded ? 1 : 0);
      const targetCount = typeof tube.target_count === 'number' ? tube.target_count : 0;

      // Shared blocking-reason path (issue #764). The `readiness` contract, when
      // present, drives the status label + row class (equivalent to the phaser
      // and blaster panels). A torpedo can still be dumb-fired at no lock, so
      // the fire button gates on loaded rounds + the tube not being offline —
      // the offline (unavailable) state is the one that disables firing.
      const rv = weaponReadinessView(tube.readiness);
      if (rv.present) {
        els.status.textContent = rv.label;
        els.row.className = 'tube-row ' + (rv.unavailable ? 'unavailable' : rv.ready ? 'ready' : 'blocked');
      } else {
        els.status.textContent = '';
        els.row.className = 'tube-row';
      }
      const canFire = loadedCount > 0 && !(rv.present && rv.unavailable);

      els.minusBtn.disabled = targetCount <= 0;
      els.plusBtn.disabled = targetCount >= vollMax;
      els.fireBtn.disabled = !canFire;
      els.fireBtn.className = canFire ? 'btn armed' : 'btn disabled';
      els.fireBtn.querySelector('.led').className = 'led' + (canFire ? ' on' : '');

      // Patterned-attack indicator (issue #766). `pattern_len > 0` marks a
      // multi-barrel patterned tube; show which step is active and which
      // barrel(s) most recently fired.
      const patternRow = els.patternRow;
      const patternLen = Number(tube.pattern_len || 0);
      const patternStep = Number(tube.pattern_step || 0);
      const activeBarrels = Array.isArray(tube.active_barrels) ? tube.active_barrels : [];
      if (patternLen > 0 && patternStep > 0) {
        patternRow.className = 'pattern-row';
        patternRow.querySelector('.pattern-step').textContent = t('component.torpedoes.pattern_step', {
          step: patternStep,
          total: patternLen,
        });
        patternRow.querySelector('.pattern-barrels').textContent = activeBarrels.length
          ? t('component.torpedoes.barrels', { barrels: activeBarrels.join(',') })
          : '';
      } else {
        patternRow.className = 'pattern-row idle';
        patternRow.querySelector('.pattern-step').textContent = '';
        patternRow.querySelector('.pattern-barrels').textContent = '';
      }
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-torpedo-controls')) {
  customElements.define('ph-torpedo-controls', PhTorpedoControls);
}
