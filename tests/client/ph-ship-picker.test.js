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
    // The raw token stays the CSS hook; only the caption is localised.
    expect(badge.classList.contains('battleship')).toBe(true);
    expect(badge.textContent.trim()).toBe(t('component.ship_picker.class.battleship'));
  });

  it('falls back to the raw class token when no caption is authored', () => {
    const { el } = setup();
    el.state = {
      ships: [
        { template_path: 'assets/entities/x.toml', label: 'X', class: 'dreadnought' },
      ]
    };
    const badge = el.shadowRoot.querySelector('.ship-badge');
    expect(badge.classList.contains('dreadnought')).toBe(true);
    expect(badge.textContent.trim()).toBe('dreadnought');
  });

  it('badges a hull with no authored class as unknown', () => {
    const { el } = setup();
    el.state = { ships: [{ template_path: 'assets/entities/x.toml', label: 'X' }] };
    const badge = el.shadowRoot.querySelector('.ship-badge');
    expect(badge.classList.contains('unknown')).toBe(true);
    expect(badge.textContent.trim()).toBe(t('component.ship_picker.class.unknown'));
  });

  // The host reads the catalog straight out of wasm_get_scenario_catalog(), so
  // its labels are still raw strings.csv ids — nothing localised them on the
  // way in the way gui/connection-manager.js does for a phone (issue #949).
  it('resolves a label that is a string id', () => {
    const { el } = setup();
    el.state = {
      ships: [
        {
          template_path: 'assets/entities/alliance_destroyer.toml',
          label: 'world.combat_test.available_ships.0.label',
          class: 'destroyer',
        },
      ]
    };
    expect(queryText(el, '.ship-name')).toBe(t('world.combat_test.available_ships.0.label'));
    expect(queryText(el, '.ship-name')).not.toContain('available_ships');
  });

  it('falls back to the template name id when the world authored no label', () => {
    const { el } = setup();
    el.state = {
      ships: [
        { template_path: 'assets/entities/alliance_destroyer.toml', name: 'entity.alliance_destroyer.name' },
      ]
    };
    expect(queryText(el, '.ship-name')).toBe(t('entity.alliance_destroyer.name'));
  });

  it('passes already-resolved text through untouched', () => {
    // A phone receives the catalog over the peer link, where localiseTree has
    // already resolved every id — resolving twice must not mangle it.
    const { el } = setup();
    el.state = {
      ships: [
        { template_path: 'assets/entities/alliance_destroyer.toml', label: 'Alliance Destroyer' },
      ]
    };
    expect(queryText(el, '.ship-name')).toBe('Alliance Destroyer');
  });

  it('localises the power and station stat labels', () => {
    const { el } = setup();
    el.state = {
      ships: [
        { template_path: 'assets/entities/alliance_destroyer.toml', label: 'X', power_rating: 70, station_count: 4 },
      ]
    };
    const labels = [...el.shadowRoot.querySelectorAll('.ship-stat-label')].map((n) => n.textContent.trim());
    expect(labels).toEqual([
      t('component.ship_picker.power'),
      t('component.ship_picker.stations'),
    ]);
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
    expect(el.shadowRoot.textContent).toContain(t('component.ship_picker.empty'));
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