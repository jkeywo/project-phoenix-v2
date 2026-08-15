// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { shipArtStyle, setShipCards } from '../../gui/components/ph-ship-picker.js';
import { CARD_TILE } from '../../scripts/ship-cards.mjs';
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

/**
 * PRD #1023 module 4, user story 3 — the card shows the ship.
 *
 * The picture is one tile of the captured yaw-ring atlas, positioned by CSS.
 * An off-by-one in the strip maths is a card showing half of two ships, so the
 * ends and the step are pinned here.
 */
describe('shipArtStyle — one tile out of the yaw strip', () => {
  it('sizes the strip so exactly one tile spans the box', () => {
    const style = shipArtStyle({ image: 'a.png', views: 8, tile: 3 });
    expect(style).toContain('background-size:800% auto');
    expect(style).toContain("background-image:url('a.png')");
  });

  it('puts the first tile at 0% and the last at 100%', () => {
    expect(shipArtStyle({ image: 'a.png', views: 8, tile: 0 }))
      .toContain('background-position-x:0.0000%');
    expect(shipArtStyle({ image: 'a.png', views: 8, tile: 7 }))
      .toContain('background-position-x:100.0000%');
  });

  it('steps evenly between them', () => {
    // Tile i of an n-tile strip sits at i/(n-1) of the scroll range.
    const style = shipArtStyle({ image: 'a.png', views: 8, tile: CARD_TILE });
    expect(style).toContain(`background-position-x:${((CARD_TILE / 7) * 100).toFixed(4)}%`);
  });

  it('is empty when there is no art, so the card renders without a frame', () => {
    expect(shipArtStyle(null)).toBe('');
    expect(shipArtStyle({ image: '', views: 8, tile: 0 })).toBe('');
    expect(shipArtStyle({ image: 'a.png', views: 1, tile: 0 })).toBe('');
  });
});

describe('PhShipPicker — card art', () => {
  const DESTROYER = 'assets/entities/alliance_destroyer.toml';
  beforeEach(() => { document.body.innerHTML = ''; setShipCards(null); });
  afterEach(() => { document.body.innerHTML = ''; setShipCards(null); });

  it('draws the hull when the build resolved art for its template', () => {
    setShipCards({ [DESTROYER]: { image: 'assets/ship-cards/alliance_destroyer.png', views: 8, tile: 3 } });
    const { el } = setup();
    el.state = { ships: [{ template_path: DESTROYER, label: 'Destroyer' }] };
    const art = el.shadowRoot.querySelector('.ship-art');
    expect(art).not.toBeNull();
    expect(art.getAttribute('style')).toContain('alliance_destroyer.png');
    // Decoration: the card's own name is the accessible label.
    expect(art.getAttribute('aria-hidden')).toBe('true');
  });

  // A mod pack's hull was never scanned at build time. That is not an error —
  // the card is complete without a portrait, exactly as it was before.
  it('renders a full card with no art for a hull the build never saw', () => {
    setShipCards({});
    const { el } = setup();
    el.state = { ships: [{ template_path: 'assets/entities/mod_hull.toml', label: 'Mod Hull' }] };
    expect(el.shadowRoot.querySelector('.ship-art')).toBeNull();
    expect(queryText(el, '.ship-name')).toBe('Mod Hull');
    expect(el.shadowRoot.querySelectorAll('.ship-card').length).toBe(1);
  });
});

/**
 * PRD #1023 module 4, user story 16 — an unacknowledged pick.
 *
 * The host's arbiter is first-valid-wins and sends the asking phone no
 * acknowledgement, so the picker has to show the request is out on its own.
 */
describe('PhShipPicker — pending pick', () => {
  const A = 'assets/entities/alliance_destroyer.toml';
  const B = 'assets/entities/alliance_cruiser.toml';
  const twoShips = { ships: [{ template_path: A, label: 'D' }, { template_path: B, label: 'C' }] };

  beforeEach(() => { document.body.innerHTML = ''; setShipCards({}); });
  afterEach(() => { document.body.innerHTML = ''; setShipCards(null); });

  it('is not busy with nothing in flight', () => {
    const { el } = setup();
    el.state = twoShips;
    expect(el.shadowRoot.getElementById('grid').dataset.busy).toBe('false');
    expect(el.shadowRoot.querySelectorAll('.ship-card.pending').length).toBe(0);
  });

  it('marks the tapped card and busies the grid', () => {
    const { el } = setup();
    el.state = { ...twoShips, pendingTemplate: A };
    expect(el.shadowRoot.getElementById('grid').dataset.busy).toBe('true');
    const pending = el.shadowRoot.querySelectorAll('.ship-card.pending');
    expect(pending.length).toBe(1);
    expect(pending[0].dataset.template).toBe(A);
    expect(pending[0].textContent).toContain(t('client.pick_pending'));
  });

  it('leaves the other cards unmarked', () => {
    const { el } = setup();
    el.state = { ...twoShips, pendingTemplate: A };
    const other = [...el.shadowRoot.querySelectorAll('.ship-card')]
      .find(c => c.dataset.template === B);
    expect(other.classList.contains('pending')).toBe(false);
    expect(other.textContent).not.toContain(t('client.pick_pending'));
  });
});