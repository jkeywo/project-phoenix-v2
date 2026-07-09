// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-comms-contact-list.js';

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-comms-contact-list id="test-el"></ph-comms-contact-list>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhCommsContactList', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-comms-contact-list')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders NO CONTACTS placeholder with empty array', () => {
    const { el } = setup();
    el.state = { contacts: [] };
    expect(queryText(el, '.list')).toBe('NO CONTACTS');
  });

  it('renders NO CONTACTS placeholder with null state', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '.list')).toBe('NO CONTACTS');
  });

  it('renders in-range contact with correct stance badge color', () => {
    const { el } = setup();
    el.state = {
      contacts: [
        { id: 'ent-1', name: 'Station Alpha', stance: 'friendly', in_range: true },
      ],
    };
    const badge = el.shadowRoot.querySelector('.badge');
    expect(badge.classList.contains('friendly')).toBe(true);
    expect(badge.textContent.trim()).toBe('friendly');
  });

  it('applies different stance colors: hostile, friendly, neutral, allied', () => {
    const { el } = setup();
    el.state = {
      contacts: [
        { id: 'ent-1', name: 'Hostile Ship', stance: 'hostile', in_range: true },
        { id: 'ent-2', name: 'Station Alpha', stance: 'friendly', in_range: true },
        { id: 'ent-3', name: 'Trading Post', stance: 'neutral', in_range: true },
        { id: 'ent-4', name: 'Allied Fleet', stance: 'allied', in_range: true },
      ],
    };
    const badges = el.shadowRoot.querySelectorAll('.badge');
    expect(badges[0].classList.contains('hostile')).toBe(true);
    expect(badges[1].classList.contains('friendly')).toBe(true);
    expect(badges[2].classList.contains('neutral')).toBe(true);
    expect(badges[3].classList.contains('allied')).toBe(true);
  });

  it('renders out-of-range contact with greyed styling', () => {
    const { el } = setup();
    el.state = {
      contacts: [
        { id: 'ent-1', name: 'Distant Ship', stance: 'neutral', in_range: false },
      ],
    };
    const pill = el.shadowRoot.querySelector('.pill');
    expect(pill.classList.contains('out-of-range')).toBe(true);
    const btn = el.shadowRoot.querySelector('.hail-btn');
    expect(btn.disabled).toBe(true);
  });

  it('renders in-range contact without out-of-range class and button enabled', () => {
    const { el } = setup();
    el.state = {
      contacts: [
        { id: 'ent-1', name: 'Station Alpha', stance: 'friendly', in_range: true },
      ],
    };
    const pill = el.shadowRoot.querySelector('.pill');
    expect(pill.classList.contains('out-of-range')).toBe(false);
    const btn = el.shadowRoot.querySelector('.hail-btn');
    expect(btn.disabled).toBe(false);
  });

  it('clicking hail button on in-range contact calls sendAction with hail', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      contacts: [
        { id: 'ent-42', name: 'Station Alpha', stance: 'friendly', in_range: true },
      ],
    };
    const btn = el.shadowRoot.querySelector('.hail-btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('hail', { target_uuid: 'ent-42' });
  });

  it('clicking hail button on out-of-range contact does not call sendAction', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      contacts: [
        { id: 'ent-99', name: 'Distant Ship', stance: 'neutral', in_range: false },
      ],
    };
    const btn = el.shadowRoot.querySelector('.hail-btn');
    btn.click();
    expect(sendAction).not.toHaveBeenCalled();
  });
});
