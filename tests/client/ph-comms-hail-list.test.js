// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-comms-hail-list.js';

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-comms-hail-list id="test-el"></ph-comms-hail-list>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhCommsHailList', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-comms-hail-list')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders NO MESSAGES placeholder with empty array', () => {
    const { el } = setup();
    el.state = { threads: [] };
    expect(queryText(el, '.list')).toBe('NO MESSAGES');
  });

  it('renders NO MESSAGES placeholder with null state', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '.list')).toBe('NO MESSAGES');
  });

  it('renders NO MESSAGES placeholder with threads: null', () => {
    const { el } = setup();
    el.state = { threads: null };
    expect(queryText(el, '.list')).toBe('NO MESSAGES');
  });

  it('renders an unread thread with bold sender and blue dot', () => {
    const { el } = setup();
    el.state = {
      threads: [
        { id: 'msg-1', sender: 'Starbase Alpha', preview: 'Incoming transmission...', unread: true, timestamp: '' },
      ],
    };
    const dot = el.shadowRoot.querySelector('.dot');
    expect(dot.classList.contains('unread')).toBe(true);
    expect(dot.classList.contains('read')).toBe(false);
    const sender = el.shadowRoot.querySelector('.sender');
    expect(sender.classList.contains('unread')).toBe(true);
    expect(sender.textContent.trim()).toBe('Starbase Alpha');
  });

  it('renders a read thread without bold sender and without blue dot', () => {
    const { el } = setup();
    el.state = {
      threads: [
        { id: 'msg-1', sender: 'Starbase Alpha', preview: 'Previous transmission', unread: false, timestamp: '' },
      ],
    };
    const dot = el.shadowRoot.querySelector('.dot');
    expect(dot.classList.contains('read')).toBe(true);
    expect(dot.classList.contains('unread')).toBe(false);
    const sender = el.shadowRoot.querySelector('.sender');
    expect(sender.classList.contains('unread')).toBe(false);
  });

  it('clicking a thread row calls sendAction with open_thread', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      threads: [
        { id: 'msg-42', sender: 'Test', preview: 'Hello', unread: true, timestamp: '' },
      ],
    };
    const row = el.shadowRoot.querySelector('.row');
    row.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('open_thread', { thread_id: 'msg-42' });
  });

  it('does not throw when sendAction is not set and row is clicked', () => {
    const { el } = setup();
    el.state = {
      threads: [
        { id: 'msg-1', sender: 'Test', preview: 'Hello', unread: false, timestamp: '' },
      ],
    };
    const row = el.shadowRoot.querySelector('.row');
    expect(() => row.click()).not.toThrow();
  });
});
