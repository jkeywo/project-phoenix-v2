// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { CAPTAIN_VIEW_ACTION_ID } from '../../gui/stations/captain-actions.js';
import '../../gui/components/ph-camera-select.js';

function setup(opts) {
  const activateSemanticAction = opts && opts.activateSemanticAction;
  if (activateSemanticAction) {
    window.activateSemanticAction = activateSemanticAction;
  }
  document.body.innerHTML = '<ph-camera-select id="test-el"></ph-camera-select>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhCameraSelect', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.activateSemanticAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.activateSemanticAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-camera-select')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders NO CAMERA placeholder with empty/null state', () => {
    const { el } = setup();
    el.state = null;
    expect(el.shadowRoot.textContent).toContain(t('component.camera_select.empty'));

    el.state = {};
    expect(el.shadowRoot.textContent).toContain(t('component.camera_select.empty'));
  });

  it('renders 4 buttons when views array provided', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: false,
    };
    const buttons = el.shadowRoot.querySelectorAll('.cam-btn');
    expect(buttons.length).toBe(4);
    expect(buttons[0].textContent.trim()).toBe('Fore');
    expect(buttons[1].textContent.trim()).toBe('Aft');
    expect(buttons[2].textContent.trim()).toBe('Port');
    expect(buttons[3].textContent.trim()).toBe('Starboard');
  });

  it('active view button has distinct styling (active class)', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Port',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: false,
    };
    const buttons = el.shadowRoot.querySelectorAll('.cam-btn');
    expect(buttons[0].classList.contains('active')).toBe(false);
    expect(buttons[1].classList.contains('active')).toBe(false);
    expect(buttons[2].classList.contains('active')).toBe(true);
    expect(buttons[3].classList.contains('active')).toBe(false);
  });

  it('classifies the host-appended cinematic view to its own full-width row', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard', 'Cinematic'],
      auto: false,
    };
    const cross = ['Fore', 'Aft', 'Port', 'Starboard'].map(
      (v) => el.shadowRoot.querySelector(`[data-view="${v}"]`),
    );
    const cinematicBtn = el.shadowRoot.querySelector('[data-view="Cinematic"]');
    expect(cinematicBtn.style.gridArea).toBe('4 / 1 / 5 / 4');
    // Untouched: the cross still lands where it always has.
    expect(cross[0].style.gridArea).toBe('1 / 2');
    expect(cross[1].style.gridArea).toBe('3 / 2');
    expect(cross[2].style.gridArea).toBe('2 / 1');
    expect(cross[3].style.gridArea).toBe('2 / 3');
  });

  it('any other extra camera view still flows into a free cross corner, unaffected by cinematic', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      // Cinematic classifies before the free-cell fallback runs, so it must
      // not consume a slot from `freeCells` — the extra "Drone" view here
      // still lands on the fallback's first cell.
      views: ['Fore', 'Aft', 'Port', 'Starboard', 'Cinematic', 'Drone'],
      auto: false,
    };
    const droneBtn = el.shadowRoot.querySelector('[data-view="Drone"]');
    expect(droneBtn.style.gridArea).toBe('1 / 1');
  });

  it('AUTO badge shown and buttons disabled when auto=true', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: true,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
    expect(badge.textContent.trim()).toBe(t('console.common.auto'));

    const buttons = el.shadowRoot.querySelectorAll('.cam-btn');
    buttons.forEach(btn => {
      expect(btn.disabled).toBe(true);
    });
  });

  it('AUTO badge hidden when auto=false', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: false,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).toBe('none');
  });

  it('routes a visible view through the parameterised Captain semantic identity', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: false,
    };

    const btn = el.shadowRoot.querySelector('[data-view="Port"]');
    btn.click();
    expect(activateSemanticAction).toHaveBeenCalledTimes(1);
    expect(activateSemanticAction).toHaveBeenCalledWith(
      CAPTAIN_VIEW_ACTION_ID,
      expect.objectContaining({
        context: 'captain', source: 'control', detail: { direction: 'Port' },
      }),
    );
  });

  it('buttons disabled when auto=true', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: true,
    };
    const buttons = el.shadowRoot.querySelectorAll('.cam-btn');
    buttons.forEach(btn => {
      expect(btn.hasAttribute('disabled')).toBe(true);
    });
  });

  it('clicking a button does nothing when auto=true', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: true,
    };

    const btn = el.shadowRoot.querySelector('[data-view="Aft"]');
    btn.click();
    expect(activateSemanticAction).not.toHaveBeenCalled();
  });
});
