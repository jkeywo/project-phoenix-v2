// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
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
    expect(queryText(el, '#container')).toBe(t('component.comms_message.no_active_hail'));
  });

  it('renders NO ACTIVE HAIL placeholder when thread is undefined', () => {
    const { el } = setup();
    el.state = {};
    expect(queryText(el, '#container')).toBe(t('component.comms_message.no_active_hail'));
  });

  it('renders NO ACTIVE HAIL placeholder when state is null', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '#container')).toBe(t('component.comms_message.no_active_hail'));
  });

  it('renders thread with sender label and message body', () => {
    const { el } = setup();
    el.state = {
      thread: {
        id: 'm1',
        sender_name: 'Starbase Alpha',
        body: 'Welcome to the sector.',
        responses: [],
      },
    };
    const senderLabel = el.shadowRoot.querySelector('.sender-label');
    expect(senderLabel.textContent.trim()).toBe('Starbase Alpha');
    const msgs = el.shadowRoot.querySelectorAll('.msg');
    expect(msgs.length).toBe(1);
    expect(msgs[0].textContent.trim()).toContain('Welcome to the sector.');
  });

  it('renders empty body placeholder when body is empty', () => {
    const { el } = setup();
    el.state = {
      thread: {
        id: 'm2',
        sender_name: 'Test',
        body: '',
        responses: [],
      },
    };
    const container = el.shadowRoot.getElementById('container');
    expect(container.textContent).toContain('(empty)');
  });

  it('clicking a response button calls sendAction with respond', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      thread: {
        id: 'm1',
        sender_name: 'Starbase Alpha',
        body: 'Welcome.',
        responses: ['Acknowledge', 'Ignore'],
      },
    };
    const btns = el.shadowRoot.querySelectorAll('.resp-btn');
    expect(btns.length).toBe(2);
    btns[0].click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('respond_to_message', { message_id: 'm1', response_index: 0 });
  });

  it('highlights selected response with checkmark and disables it', () => {
    const { el } = setup();
    el.state = {
      thread: {
        id: 'm1',
        sender_name: 'Starbase Alpha',
        body: 'Welcome.',
        responses: ['Acknowledge', 'Ignore'],
        selected_response: 0,
      },
    };
    const btns = el.shadowRoot.querySelectorAll('.resp-btn');
    expect(btns[0].disabled).toBe(true);
    expect(btns[0].textContent.trim()).toContain('\u2713');
    expect(btns[1].disabled).toBe(false);
    expect(btns[1].textContent.trim()).not.toContain('\u2713');
  });

  it('does not render responses section when responses array is empty', () => {
    const { el } = setup();
    el.state = {
      thread: {
        id: 'm1',
        sender_name: 'Test',
        body: 'Hello.',
        responses: [],
      },
    };
    const responsesDiv = el.shadowRoot.querySelector('.responses');
    expect(responsesDiv).toBeNull();
  });

  // ── #761: per-response object shape (text/important/available) ──────────────

  it('renders object-shaped responses with their text', () => {
    const { el } = setup();
    el.state = {
      thread: {
        id: 'm1',
        sender_name: 'Starbase',
        body: 'Welcome.',
        responses: [
          { text: 'Acknowledge', important: false, available: true },
          { text: 'Ignore', important: false, available: true },
        ],
      },
    };
    const btns = el.shadowRoot.querySelectorAll('.resp-btn');
    expect(btns.length).toBe(2);
    expect(btns[0].textContent.trim()).toBe('Acknowledge');
    expect(btns[1].textContent.trim()).toBe('Ignore');
  });

  // ── AC1: important responses require a two-step confirm ─────────────────────

  it('important response arms on first click and submits on second', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      thread: {
        id: 'm1',
        sender_name: 'Command',
        body: 'Arm the warhead?',
        responses: [{ text: 'Arm it', important: true, available: true }],
      },
    };
    const btn = el.shadowRoot.querySelector('.resp-btn');
    // First click: arms, does NOT submit, shows the confirm prompt.
    btn.click();
    expect(sendAction).not.toHaveBeenCalled();
    expect(btn.textContent.trim()).toBe(t('component.comms_message.confirm_important'));
    expect(btn.classList.contains('important')).toBe(true);
    // Second click: submits.
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('respond_to_message', { message_id: 'm1', response_index: 0 });
  });

  it('non-important response still submits immediately (single click)', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      thread: {
        id: 'm1',
        sender_name: 'Command',
        body: 'Proceed?',
        responses: [{ text: 'Yes', important: false, available: true }],
      },
    };
    el.shadowRoot.querySelector('.resp-btn').click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('respond_to_message', { message_id: 'm1', response_index: 0 });
  });

  // ── AC2: unavailable responses are greyed + disabled ────────────────────────

  it('unavailable response is greyed, disabled, and does not submit', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      thread: {
        id: 'm1',
        sender_name: 'Station',
        body: 'Respond.',
        responses: [{ text: 'Reply', important: false, available: false }],
      },
    };
    const btn = el.shadowRoot.querySelector('.resp-btn');
    expect(btn.disabled).toBe(true);
    expect(btn.classList.contains('unavailable')).toBe(true);
    btn.click();
    expect(sendAction).not.toHaveBeenCalled();
  });

  // ── AC3: red flash on host rejection ────────────────────────────────────────

  it('flashes the attempted response red when a rejection for it arrives', () => {
    const { el } = setup();
    const thread = {
      id: 'm1',
      sender_name: 'Station',
      body: 'Respond.',
      responses: [
        { text: 'A', important: false, available: true },
        { text: 'B', important: false, available: true },
      ],
    };
    el.state = { thread };
    let btns = el.shadowRoot.querySelectorAll('.resp-btn');
    expect(btns[1].classList.contains('rejected')).toBe(false);
    // A rejection for response index 1 of this message flashes that button.
    el.state = { thread, rejection: { message_id: 'm1', response_index: 1, ts: 111 } };
    btns = el.shadowRoot.querySelectorAll('.resp-btn');
    expect(btns[1].classList.contains('rejected')).toBe(true);
    expect(btns[0].classList.contains('rejected')).toBe(false);
    expect(btns[1].title).toBe(t('component.comms_message.rejected'));
  });

  it('ignores a rejection targeting a different message', () => {
    const { el } = setup();
    const thread = {
      id: 'm1',
      sender_name: 'Station',
      body: 'Respond.',
      responses: [{ text: 'A', important: false, available: true }],
    };
    el.state = { thread, rejection: { message_id: 'other', response_index: 0, ts: 222 } };
    const btn = el.shadowRoot.querySelector('.resp-btn');
    expect(btn.classList.contains('rejected')).toBe(false);
  });
});
