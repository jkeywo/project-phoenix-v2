// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-damage-detail.js';

function setup(html) {
  document.body.innerHTML = html || '<ph-damage-detail id="test-panel"></ph-damage-detail>';
  const el = document.getElementById('test-panel');
  return { el };
}

describe('PhDamageDetail', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-damage-detail')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('goes away when a caller sets the hidden attribute', () => {
    // The Station Bar's damage popup hides this element for a Station that
    // owns no damageable systems (gui/station-damage-popup.js, issue #1374).
    // `:host { display: block }` is an AUTHOR declaration and therefore beats
    // the UA stylesheet's `[hidden] { display: none }`, so without a matching
    // `:host([hidden])` rule the attribute is inert and the popup opens onto a
    // box that only LOOKS empty because the row list happens to render at zero
    // height today.
    //
    // Asserted against the declaration rather than `getComputedStyle`: jsdom
    // does not cascade shadow-root styles at all, so a computed-style check
    // reports `none` whether the rule is there or not and could never fail
    // (nor could the popup suite's own `.hidden === true` assertion, which is
    // about the attribute, not about what the attribute does).
    const { el } = setup();
    el.hidden = true;
    expect(el.hasAttribute('hidden')).toBe(true);
    // `data-ph-shared` is the adopted control family, appended ahead of the
    // component's own sheet (gui/components/ph-console-styles.js).
    const own = el.shadowRoot.querySelector('style:not([data-ph-shared])');
    expect(own.textContent).toMatch(/:host\(\[hidden\]\)\s*\{[^}]*display:\s*none/);
  });

  it('renders correct number of rows for entries', () => {
    const { el } = setup();
    el.state = {
      entries: [
        { display_name: 'Phaser Bank A', current: 200, max_hp: 200, tier: 3, debuff_magnitude: 0.0 },
        { display_name: 'Torpedo Launcher', current: 75, max_hp: 150, tier: 2, debuff_magnitude: 0.0 },
        { display_name: 'Deflector Array', current: 100, max_hp: 100, tier: 1, debuff_magnitude: 0.0 },
      ],
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows.length).toBe(3);
  });

  it('renders display_name and tier badge for each entry', () => {
    const { el } = setup();
    el.state = {
      entries: [
        { display_name: 'Phaser Bank A', current: 200, max_hp: 200, tier: 3, debuff_magnitude: 0.0 },
      ],
    };
    expect(el.shadowRoot.textContent).toContain('Phaser Bank A');
    expect(el.shadowRoot.textContent).toContain('T3');
  });

  it('a destroyed system (current === 0) renders DESTROYED text', () => {
    const { el } = setup();
    el.state = {
      entries: [
        { display_name: 'Torpedo Launcher', current: 0, max_hp: 150, tier: 2, debuff_magnitude: 0.5 },
      ],
    };
    expect(el.shadowRoot.textContent).toContain(t('console.common.destroyed'));
    const row = el.shadowRoot.querySelector('.row');
    expect(row.className).toContain('destroyed');
  });

  it('a non-destroyed system does not render DESTROYED text', () => {
    const { el } = setup();
    el.state = {
      entries: [
        { display_name: 'Phaser Bank A', current: 200, max_hp: 200, tier: 3, debuff_magnitude: 0.0 },
      ],
    };
    expect(el.shadowRoot.textContent).not.toContain(t('console.common.destroyed'));
    const row = el.shadowRoot.querySelector('.row');
    expect(row.className).not.toContain('destroyed');
  });

  it('missing data renders empty list without error', () => {
    const { el } = setup();
    el.state = null;
    const list = el.shadowRoot.getElementById('list');
    expect(list.innerHTML).toBe('');
  });

  it('null data set via property renders without error', () => {
    const { el } = setup();
    expect(() => { el.state = null; }).not.toThrow();
    const list = el.shadowRoot.getElementById('list');
    expect(list.innerHTML).toBe('');
  });

  it('empty entries array renders empty list', () => {
    const { el } = setup();
    el.state = { entries: [] };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows.length).toBe(0);
  });

  it('applies correct colour class for healthy system (pct >= 0.75)', () => {
    const { el } = setup();
    el.state = {
      entries: [
        { display_name: 'Healthy System', current: 200, max_hp: 200, tier: 1, debuff_magnitude: 0.0 },
      ],
    };
    const fill = el.shadowRoot.querySelector('.fill');
    expect(fill.className).toBe('fill');
  });

  it('applies warn class for amber system (0.4 <= pct < 0.75)', () => {
    const { el } = setup();
    el.state = {
      entries: [
        { display_name: 'Damaged System', current: 60, max_hp: 100, tier: 2, debuff_magnitude: 0.0 },
      ],
    };
    const fill = el.shadowRoot.querySelector('.fill');
    expect(fill.className).toBe('fill warn');
  });

  it('applies crit class for critically damaged system (pct < 0.4)', () => {
    const { el } = setup();
    el.state = {
      entries: [
        { display_name: 'Critical System', current: 30, max_hp: 100, tier: 2, debuff_magnitude: 0.0 },
      ],
    };
    const fill = el.shadowRoot.querySelector('.fill');
    expect(fill.className).toBe('fill crit');
  });

  it('updates correctly when data changes', () => {
    const { el } = setup();
    el.state = {
      entries: [
        { display_name: 'System A', current: 100, max_hp: 100, tier: 1, debuff_magnitude: 0.0 },
      ],
    };
    expect(el.shadowRoot.querySelectorAll('.row').length).toBe(1);
    el.state = {
      entries: [
        { display_name: 'System A', current: 100, max_hp: 100, tier: 1, debuff_magnitude: 0.0 },
        { display_name: 'System B', current: 0, max_hp: 80, tier: 2, debuff_magnitude: 0.3 },
      ],
    };
    expect(el.shadowRoot.querySelectorAll('.row').length).toBe(2);
    expect(el.shadowRoot.textContent).toContain(t('console.common.destroyed'));
  });
});
