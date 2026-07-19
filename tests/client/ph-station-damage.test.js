// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-station-damage.js';

function setup() {
  document.body.innerHTML = '<ph-station-damage id="test-el"></ph-station-damage>';
  return { el: document.getElementById('test-el') };
}

describe('PhStationDamage', () => {
  beforeEach(() => { document.body.innerHTML = ''; });
  afterEach(() => { document.body.innerHTML = ''; });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-station-damage')).toBeDefined();
  });

  it('hides itself when the station has no damageable systems', () => {
    const { el } = setup();
    el.state = { entries: [], pct: 1 };
    expect(el.hidden).toBe(true);
  });

  it('shows the integrity bar and percentage when there are systems', () => {
    const { el } = setup();
    el.state = {
      entries: [{ display_name: 'Helm', current: 5, max_hp: 10 }],
      totalCurrent: 5, totalMax: 10, pct: 0.5,
    };
    expect(el.hidden).toBe(false);
    expect(el.shadowRoot.getElementById('pct').textContent).toBe('50%');
    expect(el.shadowRoot.getElementById('fill').style.width).toBe('50%');
    expect(el.shadowRoot.getElementById('fill').className).toContain('warn');
  });

  it('marks the bar critical below 40%', () => {
    const { el } = setup();
    el.state = { entries: [{ display_name: 'Helm', current: 2, max_hp: 10 }], pct: 0.2 };
    expect(el.shadowRoot.getElementById('fill').className).toContain('crit');
  });

  it('toggles a read-only system popup when the bar is clicked', () => {
    const { el } = setup();
    el.state = {
      entries: [
        { display_name: 'Helm', current: 5, max_hp: 10 },
        { display_name: 'Tactical', current: 10, max_hp: 10 },
      ],
      pct: 0.75,
    };
    const popup = el.shadowRoot.getElementById('popup');
    expect(popup.classList.contains('open')).toBe(false);

    el.shadowRoot.getElementById('bar').click();
    expect(popup.classList.contains('open')).toBe(true);

    // The popup renders the per-system detail and contains no dispatch controls.
    const detail = el.shadowRoot.getElementById('detail');
    expect(detail.shadowRoot.textContent).toContain('Helm');
    expect(el.shadowRoot.querySelector('button.dispatch-btn')).toBeNull();

    el.shadowRoot.getElementById('bar').click();
    expect(popup.classList.contains('open')).toBe(false);
  });
});
