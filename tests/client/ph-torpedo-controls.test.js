// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-torpedo-controls.js';

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-torpedo-controls id="test-el"></ph-torpedo-controls>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

function tubeRow(host, tubeId) {
  return host.shadowRoot.querySelector(`.tube-row[data-id="${tubeId}"]`);
}

function slots(host, tubeId) {
  return Array.from(tubeRow(host, tubeId).querySelectorAll('.torp-slot'));
}

function minusBtn(host, tubeId) {
  return tubeRow(host, tubeId).querySelectorAll('.mini-btn')[0];
}

function plusBtn(host, tubeId) {
  return tubeRow(host, tubeId).querySelectorAll('.mini-btn')[1];
}

function fireBtn(host, tubeId) {
  return tubeRow(host, tubeId).querySelector('.btn');
}

describe('PhTorpedoControls', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-torpedo-controls')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders NO TORPEDO TUBES placeholder with empty tubes', () => {
    const { el } = setup();
    el.state = { tubes: [], magazine: { current: 0, max: 0 } };
    expect(queryText(el, '#tubes')).toBe(t('component.torpedoes.empty'));
  });

  it('renders NO TORPEDO TUBES placeholder with null state', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '#tubes')).toBe(t('component.torpedoes.empty'));
  });

  it('displays magazine count', () => {
    const { el } = setup();
    el.state = { tubes: [], magazine: { current: 6, max: 20 } };
    expect(queryText(el, '#magazine')).toBe('6 / 20');
  });

  // ── Shared weapon readiness contract (issue #764) ──────────────────────
  // Torpedo tubes render the same status label + row class as the phaser and
  // blaster panels for the shared blocking cases.

  function tubeWith(reason, extra) {
    return {
      id: 'fore_port',
      label: 'Fore Port',
      volley_max: 2,
      loaded_count: reason === 'NoAmmo' || reason === 'Loading' ? 0 : 1,
      target_count: 2,
      state: reason === 'Loading' ? 'loading' : 'loaded',
      readiness: {
        ready: reason === 'Ready',
        blocking_reason: reason,
        target_range: 100,
        target_arc: 5,
        ...extra,
      },
    };
  }

  it('renders READY state and marks the tube ready', () => {
    const { el } = setup();
    el.state = { tubes: [tubeWith('Ready')], magazine: { current: 6, max: 20 }, target_uuid: 'x' };
    expect(tubeRow(el, 'fore_port').classList.contains('ready')).toBe(true);
    expect(fireBtn(el, 'fore_port').disabled).toBe(false);
  });

  it('renders OUT OF RANGE block with the shared label', () => {
    const { el } = setup();
    el.state = { tubes: [tubeWith('OutOfRange')], magazine: { current: 6, max: 20 } };
    expect(tubeRow(el, 'fore_port').classList.contains('blocked')).toBe(true);
    expect(queryText(el, '.status')).toBe(t('console.common.out_of_range'));
  });

  it('renders OUT OF ARC block with the shared label', () => {
    const { el } = setup();
    el.state = { tubes: [tubeWith('OutOfArc')], magazine: { current: 6, max: 20 } };
    expect(queryText(el, '.status')).toBe(t('console.common.out_of_arc'));
  });

  it('renders LOADING block with the shared label', () => {
    const { el } = setup();
    el.state = { tubes: [tubeWith('Loading')], magazine: { current: 6, max: 20 } };
    expect(queryText(el, '.status')).toBe(t('console.common.loading'));
  });

  it('renders NO AMMO block with the shared label', () => {
    const { el } = setup();
    el.state = { tubes: [tubeWith('NoAmmo', { target_range: null, target_arc: null })], magazine: { current: 0, max: 20 } };
    expect(queryText(el, '.status')).toBe(t('console.common.no_ammo'));
  });

  it('renders OFFLINE as an unavailable state and disables fire', () => {
    const { el } = setup();
    el.state = { tubes: [tubeWith('Offline')], magazine: { current: 6, max: 20 }, target_uuid: 'x' };
    const row = tubeRow(el, 'fore_port');
    expect(row.classList.contains('unavailable')).toBe(true);
    expect(queryText(el, '.status')).toBe(t('console.common.offline'));
    expect(fireBtn(el, 'fore_port').disabled).toBe(true);
  });

  it('renders a tube with its label and one slot per volley_max', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', volley_max: 4, loaded_count: 0, target_count: 0 }],
      magazine: { current: 6, max: 20 },
    };
    expect(queryText(el, '.lbl')).toBe('FORE PORT');
    expect(slots(el, 'fore_port').length).toBe(4);
  });

  it('marks loaded slots filled and leaves the rest empty', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 4, loaded_count: 2, target_count: 2, state: 'unloaded' }],
      magazine: { current: 6, max: 20 },
    };
    const s = slots(el, 'fore_port');
    expect(s[0].dataset.state).toBe('filled');
    expect(s[1].dataset.state).toBe('filled');
    expect(s[2].dataset.state).toBe('empty');
    expect(s[3].dataset.state).toBe('empty');
  });

  it('shows the loading slot filling green toward the target', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 2, loaded_count: 0, target_count: 1, state: 'loading', load_progress: 0.4 }],
      magazine: { current: 6, max: 20 },
    };
    const s = slots(el, 'fore_port');
    expect(s[0].dataset.state).toBe('loading');
    expect(s[0].querySelector('.fill').style.height).toBe('40%');
  });

  it('shows the unloading slot draining back to grey', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 2, loaded_count: 1, target_count: 0, state: 'unloading', load_progress: 0.25 }],
      magazine: { current: 6, max: 20 },
    };
    const s = slots(el, 'fore_port');
    expect(s[0].dataset.state).toBe('unloading');
    // fill = 1 - load_progress: drains from full toward empty as unload completes.
    expect(s[0].querySelector('.fill').style.height).toBe('75%');
  });

  it('enables the fire button once at least one round is loaded', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 2, loaded_count: 1, target_count: 1 }],
      magazine: { current: 6, max: 20 },
    };
    expect(fireBtn(el, 'fore_port').disabled).toBe(false);
  });

  it('disables the fire button when nothing is loaded', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 2, loaded_count: 0, target_count: 0 }],
      magazine: { current: 6, max: 20 },
    };
    expect(fireBtn(el, 'fore_port').disabled).toBe(true);
  });

  it('disables minus at target_count 0 and plus at volley_max', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 2, loaded_count: 0, target_count: 0 }],
      magazine: { current: 6, max: 20 },
    };
    expect(minusBtn(el, 'fore_port').disabled).toBe(true);
    expect(plusBtn(el, 'fore_port').disabled).toBe(false);

    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 2, loaded_count: 2, target_count: 2 }],
      magazine: { current: 6, max: 20 },
    };
    expect(minusBtn(el, 'fore_port').disabled).toBe(false);
    expect(plusBtn(el, 'fore_port').disabled).toBe(true);
  });

  it('dispatches set_torpedo_volley_target with count+1 on plus click', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 4, loaded_count: 1, target_count: 1 }],
      magazine: { current: 6, max: 20 },
    };
    plusBtn(el, 'fore_port').click();
    expect(sendAction).toHaveBeenCalledWith('set_torpedo_volley_target', { tube: 'fore_port', count: 2 });
  });

  it('dispatches set_torpedo_volley_target with count-1 on minus click', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 4, loaded_count: 2, target_count: 2 }],
      magazine: { current: 6, max: 20 },
    };
    minusBtn(el, 'fore_port').click();
    expect(sendAction).toHaveBeenCalledWith('set_torpedo_volley_target', { tube: 'fore_port', count: 1 });
  });

  it('dispatches fire_torpedo with the current target_uuid on fire click', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 2, loaded_count: 1, target_count: 1 }],
      magazine: { current: 6, max: 20 },
      target_uuid: 'enemy-1',
    };
    fireBtn(el, 'fore_port').click();
    expect(sendAction).toHaveBeenCalledWith('fire_torpedo', { tube: 'fore_port', target_uuid: 'enemy-1' });
  });

  it('rebuilds slots when volley_max changes', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 2, loaded_count: 0, target_count: 0 }],
      magazine: { current: 6, max: 20 },
    };
    expect(slots(el, 'fore_port').length).toBe(2);

    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 4, loaded_count: 0, target_count: 0 }],
      magazine: { current: 6, max: 20 },
    };
    expect(slots(el, 'fore_port').length).toBe(4);
  });

  it('reconciles tube rows by id', () => {
    const { el } = setup();
    el.state = {
      tubes: [
        { id: 'fore_port', volley_max: 1, loaded_count: 1, target_count: 1 },
        { id: 'fore_starboard', volley_max: 1, loaded_count: 0, target_count: 0 },
      ],
      magazine: { current: 6, max: 20 },
    };
    expect(el.shadowRoot.querySelectorAll('.tube-row').length).toBe(2);

    el.state = {
      tubes: [{ id: 'fore_port', volley_max: 1, loaded_count: 1, target_count: 1 }],
      magazine: { current: 6, max: 20 },
    };
    expect(el.shadowRoot.querySelectorAll('.tube-row').length).toBe(1);
    expect(el.shadowRoot.querySelector('.tube-row').dataset.id).toBe('fore_port');
  });
});
