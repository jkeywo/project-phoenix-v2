// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-tutorial-overlay.js';

// Wire-shaped active overlays — content fields are strings.csv ids exactly as
// authored in assets/entities/alliance_destroyer.toml (issue #916).
const WELCOME = {
  id: 'helm-welcome',
  title: 'entity.alliance_destroyer.station.helm.tutorial.welcome.title',
  text: 'entity.alliance_destroyer.station.helm.tutorial.welcome.text',
  anchor: 'helm-radar',
};
const JOYSTICK = {
  id: 'helm-joystick',
  title: 'entity.alliance_destroyer.station.helm.tutorial.joystick.title',
  text: 'entity.alliance_destroyer.station.helm.tutorial.joystick.text',
  anchor: 'helm-joystick',
};

function setup(opts) {
  if (opts && opts.sendAction) window.sendAction = opts.sendAction;
  document.body.innerHTML =
    '<div id="helm-radar"></div><div id="helm-joystick"></div>' +
    '<ph-tutorial-overlay id="overlay"></ph-tutorial-overlay>';
  return { el: document.getElementById('overlay') };
}

function queryText(host, sel) {
  const node = host.shadowRoot.querySelector(sel);
  return node ? node.textContent.trim() : null;
}

describe('PhTutorialOverlay', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-tutorial-overlay')).toBeDefined();
  });

  it('starts hidden and stays hidden for a null/empty tutorial block', () => {
    const { el } = setup();
    expect(el.hidden).toBe(true);
    el.state = null;
    expect(el.hidden).toBe(true);
    el.state = { active: null, remaining: 0 };
    expect(el.hidden).toBe(true);
  });

  it('renders the active overlay title and text via t(id)', () => {
    const { el } = setup();
    el.state = { active: WELCOME, remaining: 1 };
    expect(el.hidden).toBe(false);
    expect(queryText(el, '.eyebrow')).toBe(t('component.tutorial.heading'));
    expect(queryText(el, '.title')).toBe(t(WELCOME.title));
    expect(queryText(el, '.text')).toBe(t(WELCOME.text));
    expect(queryText(el, '.dismiss')).toBe(t('component.tutorial.dismiss'));
  });

  it('shows the queued-tips hint only when more overlays are eligible', () => {
    const { el } = setup();
    el.state = { active: WELCOME, remaining: 3 };
    const more = el.shadowRoot.getElementById('more');
    expect(more.hidden).toBe(false);
    expect(more.textContent).toBe(t('component.tutorial.more', { n: 2 }));
    el.state = { active: WELCOME, remaining: 1 };
    expect(el.shadowRoot.getElementById('more').hidden).toBe(true);
  });

  it('dismiss sends tutorial_dismiss with the active overlay id', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { active: WELCOME, remaining: 1 };
    el.shadowRoot.getElementById('dismiss').click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('tutorial_dismiss', { overlay_id: 'helm-welcome' });
  });

  it('does not throw when dismissed with no sendAction wired', () => {
    const { el } = setup();
    el.state = { active: WELCOME, remaining: 1 };
    expect(() => el.shadowRoot.getElementById('dismiss').click()).not.toThrow();
  });

  it('re-parenting does not stack dismiss listeners (issue #916 review)', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { active: WELCOME, remaining: 1 };
    // Two re-parents = two extra connectedCallback runs; the click listener
    // is wired once in the constructor, so one click stays one action.
    const other = document.createElement('div');
    document.body.appendChild(other);
    other.appendChild(el);
    document.body.appendChild(el);
    el.shadowRoot.getElementById('dismiss').click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('tutorial_dismiss', { overlay_id: 'helm-welcome' });
  });

  it('highlights the anchored light-DOM control and moves the highlight with the state', () => {
    const { el } = setup();
    const radar = document.getElementById('helm-radar');
    const stick = document.getElementById('helm-joystick');

    el.state = { active: WELCOME, remaining: 2 };
    expect(radar.classList.contains('tutorial-highlight')).toBe(true);
    expect(stick.classList.contains('tutorial-highlight')).toBe(false);

    el.state = { active: JOYSTICK, remaining: 1 };
    expect(radar.classList.contains('tutorial-highlight')).toBe(false);
    expect(stick.classList.contains('tutorial-highlight')).toBe(true);

    el.state = null;
    expect(stick.classList.contains('tutorial-highlight')).toBe(false);
    expect(el.hidden).toBe(true);
  });

  it('tolerates an anchor that names no element', () => {
    const { el } = setup();
    expect(() => {
      el.state = { active: { ...WELCOME, anchor: 'not-a-real-id' }, remaining: 1 };
    }).not.toThrow();
    expect(el.hidden).toBe(false);
  });
});
