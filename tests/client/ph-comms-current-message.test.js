// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-comms-current-message.js';

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-comms-current-message id="test-el"></ph-comms-current-message>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhCommsCurrentMessage', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-comms-current-message')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders NO ACTIVE HAIL placeholder when thread is null', () => {
    const { el } = setup();
    el.state = { thread: null };
    expect(queryText(el, '#container')).toBe('NO ACTIVE HAIL');
  });

  it('renders NO ACTIVE HAIL placeholder when thread is undefined', () => {
    const { el } = setup();
    el.state = {};
    expect(queryText(el, '#container')).toBe('NO ACTIVE HAIL');
  });

  it('renders NO ACTIVE HAIL placeholder when state is null', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '#container')).toBe('NO ACTIVE HAIL');
  });

  it('renders thread with sender label and messages', () => {
    const { el } = setup();
    el.state = {
      thread: {
        sender: 'Starbase Alpha',
        messages: [
          { speaker: 'Station', text: 'Welcome to the sector.' },
          { speaker: 'Ship', text: 'Thank you for the welcome.' },
        ],
        responses: [],
      },
    };
    const senderLabel = el.shadowRoot.querySelector('.sender-label');
    expect(senderLabel.textContent.trim()).toBe('Starbase Alpha');
    const msgs = el.shadowRoot.querySelectorAll('.msg');
    expect(msgs.length).toBe(2);
    expect(msgs[0].textContent.trim()).toContain('Welcome to the sector.');
    expect(msgs[1].textContent.trim()).toContain('Thank you for the welcome.');
  });

  it('renders speaker labels for each message', () => {
    const { el } = setup();
    el.state = {
      thread: {
        sender: 'Test',
        messages: [
          { speaker: 'Station', text: 'Hello.' },
          { speaker: 'Commander', text: 'Ready.' },
        ],
        responses: [],
      },
    };
    const speakers = el.shadowRoot.querySelectorAll('.speaker');
    expect(speakers.length).toBe(2);
    expect(speakers[0].textContent.trim()).toBe('Station:');
    expect(speakers[1].textContent.trim()).toBe('Commander:');
  });

  it('clicking a response button calls sendAction with respond', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      thread: {
        sender: 'Starbase Alpha',
        messages: [{ speaker: 'Station', text: 'Welcome.' }],
        responses: [
          { id: 'resp-1', text: 'Acknowledge', available: true },
        ],
      },
    };
    const btn = el.shadowRoot.querySelector('.resp-btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('respond', { response_id: 'resp-1' });
  });

  it('disables response button when available is false', () => {
    const { el } = setup();
    el.state = {
      thread: {
        sender: 'Starbase Alpha',
        messages: [{ speaker: 'Station', text: 'Welcome.' }],
        responses: [
          { id: 'resp-1', text: 'Acknowledge', available: false },
        ],
      },
    };
    const btn = el.shadowRoot.querySelector('.resp-btn');
    expect(btn.disabled).toBe(true);
  });

  it('does not call sendAction when clicking a disabled response button', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      thread: {
        sender: 'Starbase Alpha',
        messages: [{ speaker: 'Station', text: 'Welcome.' }],
        responses: [
          { id: 'resp-1', text: 'Acknowledge', available: false },
        ],
      },
    };
    const btn = el.shadowRoot.querySelector('.resp-btn');
    btn.click();
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('defaults available to true when field is missing', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      thread: {
        sender: 'Test',
        messages: [{ speaker: 'Station', text: 'Hello.' }],
        responses: [
          { id: 'resp-1', text: 'Reply', available: undefined },
        ],
      },
    };
    const btn = el.shadowRoot.querySelector('.resp-btn');
    expect(btn.disabled).toBe(false);
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
  });
});
