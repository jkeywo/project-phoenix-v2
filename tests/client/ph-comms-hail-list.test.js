// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
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
    el.state = { messages: [] };
    expect(queryText(el, '.list')).toBe(t('component.comms_hails.empty'));
  });

  it('renders NO MESSAGES placeholder with null state', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '.list')).toBe(t('component.comms_hails.empty'));
  });

  it('renders NO MESSAGES placeholder with messages: null', () => {
    const { el } = setup();
    el.state = { messages: null };
    expect(queryText(el, '.list')).toBe(t('component.comms_hails.empty'));
  });

  it('renders an unread message with bold sender and blue dot', () => {
    const { el } = setup();
    el.state = {
      messages: [
        { id: 'msg-1', sender_name: 'Starbase Alpha', subject: 'Incoming transmission...', is_read: false },
      ],
    };
    const dot = el.shadowRoot.querySelector('.dot');
    expect(dot.classList.contains('unread')).toBe(true);
    expect(dot.classList.contains('read')).toBe(false);
    const sender = el.shadowRoot.querySelector('.sender');
    expect(sender.classList.contains('unread')).toBe(true);
    expect(sender.textContent.trim()).toBe('Starbase Alpha');
  });

  it('renders a read message without bold sender and without blue dot', () => {
    const { el } = setup();
    el.state = {
      messages: [
        { id: 'msg-1', sender_name: 'Starbase Alpha', subject: 'Previous transmission', is_read: true },
      ],
    };
    const dot = el.shadowRoot.querySelector('.dot');
    expect(dot.classList.contains('read')).toBe(true);
    expect(dot.classList.contains('unread')).toBe(false);
    const sender = el.shadowRoot.querySelector('.sender');
    expect(sender.classList.contains('unread')).toBe(false);
  });

  it('marks a live Critical hail with localized text, a diamond, and danger colour even after read', () => {
    const { el } = setup();
    el.state = {
      messages: [{
        id: 'msg-critical',
        thread_id: 'lark',
        sender_name: 'Lark',
        body: 'Confirm corridor safety.',
        priority: 'Critical',
        is_read: true,
        selected_response: null,
        is_orphaned: false,
      }],
    };
    const row = el.shadowRoot.querySelector('.row');
    const cue = row.querySelector('.priority-cue');
    expect(row.classList.contains('critical')).toBe(true);
    expect(row.dataset.priority).toBe('critical');
    expect(cue.classList.contains('critical')).toBe(true);
    expect(cue.querySelector('.priority-shape').textContent).toBe('◆');
    expect(cue.querySelector('.priority-text').textContent)
      .toBe(t('component.comms.priority.critical'));
    expect(el.shadowRoot.querySelector('style').textContent).toContain('var(--fire-bright)');
  });

  it('removes the Critical cue after response, invalidation, or supersession', () => {
    const { el } = setup();
    const critical = {
      id: 'msg-critical', thread_id: 'lark', sender_name: 'Lark',
      priority: 'Critical', is_read: true, selected_response: null, is_orphaned: false,
    };
    el.state = { messages: [critical] };
    expect(el.shadowRoot.querySelector('.row').classList.contains('critical')).toBe(true);

    el.state = { messages: [{ ...critical, selected_response: 0 }] };
    expect(el.shadowRoot.querySelector('.row').classList.contains('critical')).toBe(false);

    el.state = { messages: [{ ...critical, is_orphaned: true }] };
    expect(el.shadowRoot.querySelector('.row').classList.contains('critical')).toBe(false);

    el.state = { messages: [critical, {
      id: 'msg-routine', thread_id: 'lark', sender_name: 'Lark', priority: 'Routine',
    }] };
    expect(el.shadowRoot.querySelectorAll('.row.critical')).toHaveLength(0);
  });

  it('shows a readable preview derived from the resolved body, not a chopped id', () => {
    const { el } = setup();
    el.state = {
      messages: [
        {
          id: 'msg-1',
          sender_name: 'Axiom Station',
          // Body arrives resolved (localiseTree ran at the wire boundary); the
          // old behaviour chopped the body ID to 40 chars and showed that.
          body: 'Phoenix, hold position — we are reading a hull breach on deck four.',
          subject: 'world.default.comms.hull_breach_report.message',
          is_read: false,
        },
      ],
    };
    const preview = el.shadowRoot.querySelector('.preview');
    expect(preview.textContent).toContain('Phoenix, hold position');
    expect(preview.textContent).not.toContain('world.');
  });

  it('clicking a message row calls sendAction with select_comms_message', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      messages: [
        { id: 'msg-42', sender_name: 'Test', subject: 'Hello', is_read: false },
      ],
    };
    const row = el.shadowRoot.querySelector('.row');
    row.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('select_comms_message', { message_id: 'msg-42' });
  });

  it('does not throw when sendAction is not set and row is clicked', () => {
    const { el } = setup();
    el.state = {
      messages: [
        { id: 'msg-1', sender_name: 'Test', subject: 'Hello', is_read: true },
      ],
    };
    const row = el.shadowRoot.querySelector('.row');
    expect(() => row.click()).not.toThrow();
  });
});
