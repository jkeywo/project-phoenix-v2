// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-ship-picker.js';

function setup() {
  document.body.innerHTML = '<ph-ship-picker id="test-picker"></ph-ship-picker>';
  return { el: document.getElementById('test-picker') };
}

function queryText(el, selector) {
  return el.shadowRoot.querySelector(selector)?.textContent?.trim() ?? '';
}

describe('PhShipPicker', () => {
  beforeEach(() => { document.body.innerHTML = ''; });
  afterEach(() => { document.body.innerHTML = ''; });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-ship-picker')).toBeDefined();
  });

  it('renders ship cards from state', () => {
    const { el } = setup();
    el.state = {
      ships: [
        { template_path: 'assets/entities/alliance_battleship.toml', label: 'Battleship', class: 'battleship', hull_id: 'NCC-2001', power_rating: 120 },
        { template_path: 'assets/entities/alliance_cruiser.toml', label: 'Cruiser', class: 'cruiser', hull_id: 'NCC-1864', power_rating: 90 },
      ]
    };
    const cards = el.shadowRoot.querySelectorAll('.ship-card');
    expect(cards.length).toBe(2);
    expect(queryText(el, '.ship-name')).toBe('Battleship');
  });

  it('shows class badge with correct color class', () => {
    const { el } = setup();
    el.state = {
      ships: [
        { template_path: 'assets/entities/alliance_battleship.toml', label: 'Battleship', class: 'battleship', power_rating: 120 },
      ]
    };
    const badge = el.shadowRoot.querySelector('.ship-badge');
    expect(badge.classList.contains('battleship')).toBe(true);
    expect(badge.textContent.trim()).toBe('battleship');
  });

  it('shows hull_id and power_rating', () => {
    const { el } = setup();
    el.state = {
      ships: [
        { template_path: 'assets/entities/alliance_battleship.toml', label: 'Battleship', class: 'battleship', hull_id: 'NCC-2001', power_rating: 120 },
      ]
    };
    expect(el.shadowRoot.textContent).toContain('NCC-2001');
    expect(el.shadowRoot.textContent).toContain('120');
  });

  it('dispatches ship-selected event with template_path on card click', async () => {
    const { el } = setup();
    el.state = {
      ships: [
        { template_path: 'assets/entities/alliance_battleship.toml', label: 'Battleship', class: 'battleship' },
        { template_path: 'assets/entities/alliance_cruiser.toml', label: 'Cruiser', class: 'cruiser' },
      ]
    };

    let received = null;
    el.addEventListener('ship-selected', (e) => { received = e.detail; });

    el.shadowRoot.querySelectorAll('.ship-card')[1].click();

    expect(received).not.toBeNull();
    expect(received.template_path).toBe('assets/entities/alliance_cruiser.toml');
    expect(received.ship.label).toBe('Cruiser');
  });

  it('renders empty state when no ships', () => {
    const { el } = setup();
    el.state = { ships: [] };
    expect(el.shadowRoot.textContent).toContain('No ships available');
  });

  it('falls back to template_path filename as name when no label', () => {
    const { el } = setup();
    el.state = {
      ships: [
        { template_path: 'assets/entities/alliance_battleship.toml' },
      ]
    };
    expect(queryText(el, '.ship-name')).toBe('alliance_battleship');
  });
});