// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-comms-current-message.js';

function setup(opts) {
  const activateSemanticAction = opts && opts.activateSemanticAction;
  if (activateSemanticAction) {
    window.activateSemanticAction = activateSemanticAction;
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
    delete window.activateSemanticAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.activateSemanticAction;
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

  it('shows a read live Critical hail as non-modal text, shape, and colour', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    const thread = {
      id: 'm-critical', thread_id: 'lark', sender_name: 'Lark',
      body: 'Is the corridor safe?', priority: 'Critical', is_read: true,
      selected_response: null, is_orphaned: false, responses: ['Unsafe'],
    };
    el.state = { thread, messages: [thread] };
    const cue = el.shadowRoot.querySelector('.priority-cue');
    expect(cue.hidden).toBe(false);
    expect(cue.textContent).toContain('◆');
    expect(cue.textContent).toContain(t('component.comms.priority.critical'));
    expect(el.shadowRoot.querySelector('style').textContent).toContain('var(--fire-bright)');
    expect(el.shadowRoot.querySelector('[role="dialog"]')).toBeNull();

    el.shadowRoot.querySelector('.resp-btn').click();
    expect(activateSemanticAction).toHaveBeenCalledWith('comms.respond', {
      source: 'control',
      detail: { message_id: 'm-critical', response_index: 0, confirmed: false },
    });
  });

  it('clears the Critical cue after response, invalidation, or supersession', () => {
    const { el } = setup();
    const critical = {
      id: 'm-critical', thread_id: 'lark', sender_name: 'Lark', body: 'Safety check.',
      priority: 'Critical', selected_response: null, is_orphaned: false, responses: ['Unsafe'],
    };
    el.state = { thread: critical, messages: [critical] };
    expect(el.shadowRoot.querySelector('.priority-cue').hidden).toBe(false);

    el.state = { thread: { ...critical, selected_response: 0 }, messages: [{ ...critical, selected_response: 0 }] };
    expect(el.shadowRoot.querySelector('.priority-cue').hidden).toBe(true);

    el.state = { thread: { ...critical, is_orphaned: true }, messages: [{ ...critical, is_orphaned: true }] };
    expect(el.shadowRoot.querySelector('.priority-cue').hidden).toBe(true);

    el.state = { thread: critical, messages: [critical, {
      id: 'm-routine', thread_id: 'lark', priority: 'Routine', selected_response: null,
    }] };
    expect(el.shadowRoot.querySelector('.priority-cue').hidden).toBe(true);
  });

  // ── Issue #1380: the whole conversation, responses pinned beneath ────────

  it('renders every message of the thread in order, with the responses outside the scroll box', () => {
    const { el } = setup();
    const conversation = ['a', 'b', 'c', 'd', 'e'].map((suffix, i) => ({
      id: `m-${suffix}`, thread_id: 'relay', sender_name: i % 2 ? 'Ardent' : 'Relay Seven',
      body: `Line ${i + 1}`, responses: [],
    }));
    const active = { ...conversation[4], responses: ['Acknowledge'] };
    el.state = {
      thread: active,
      messages: [...conversation.slice(0, 4), active],
      sender_name: 'Relay Seven',
    };
    const msgs = [...el.shadowRoot.querySelectorAll('.msg')];
    expect(msgs).toHaveLength(5);
    expect(msgs.map((m) => m.querySelector('.text').textContent))
      .toEqual(['Line 1', 'Line 2', 'Line 3', 'Line 4', 'Line 5']);
    // A multi-line thread names who spoke each line; the header names the
    // channel the projection supplied.
    expect(msgs[1].querySelector('.speaker').textContent).toBe('Ardent');
    expect(el.shadowRoot.getElementById('sender-label').textContent).toBe('Relay Seven');
    // The history scrolls in its own box and the responses sit OUTSIDE it, so
    // a long exchange never pushes them off the console.
    const scroller = el.shadowRoot.getElementById('messages');
    expect(scroller.querySelector('.responses')).toBeNull();
    const responses = el.shadowRoot.querySelector('.responses');
    expect(responses.parentElement).toBe(el.shadowRoot.getElementById('thread'));
    const css = [...el.shadowRoot.querySelectorAll('style')]
      .map((style) => style.textContent).join('\n');
    expect(css).toMatch(/\.messages\s*\{[^}]*overflow-y:\s*auto/);
  });

  it('leaves the speaker blank on a one-message thread, where the header already says it', () => {
    const { el } = setup();
    el.state = { thread: { id: 'm1', sender_name: 'Solo', body: 'Once', responses: [] } };
    expect(el.shadowRoot.querySelector('.msg .speaker').textContent).toBe('');
    expect(el.shadowRoot.getElementById('sender-label').textContent).toBe('Solo');
  });

  it('grows a thread by one node rather than rebuilding the conversation', () => {
    const { el } = setup();
    const first = { id: 'm1', thread_id: 't', sender_name: 'Relay', body: 'One', responses: [] };
    const second = { id: 'm2', thread_id: 't', sender_name: 'Relay', body: 'Two', responses: ['Ack'] };
    el.state = { thread: first, messages: [first] };
    const original = el.shadowRoot.querySelector('.msg');
    el.state = { thread: second, messages: [first, second] };
    const grown = [...el.shadowRoot.querySelectorAll('.msg')];
    expect(grown).toHaveLength(2);
    expect(grown[0]).toBe(original);
    // A message that left the thread takes its node with it.
    el.state = { thread: second, messages: [second] };
    expect(el.shadowRoot.querySelectorAll('.msg')).toHaveLength(1);
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

  it('clicking a response button activates the shared response identity', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
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
    expect(activateSemanticAction).toHaveBeenCalledTimes(1);
    expect(activateSemanticAction).toHaveBeenCalledWith('comms.respond', {
      source: 'control', detail: { message_id: 'm1', response_index: 0, confirmed: false },
    });
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
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
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
    expect(activateSemanticAction).not.toHaveBeenCalled();
    expect(btn.textContent.trim()).toBe(t('component.comms_message.confirm_important'));
    expect(btn.classList.contains('important')).toBe(true);
    // Second click: submits.
    btn.click();
    expect(activateSemanticAction).toHaveBeenCalledTimes(1);
    expect(activateSemanticAction).toHaveBeenCalledWith('comms.respond', {
      source: 'control', detail: { message_id: 'm1', response_index: 0, confirmed: true },
    });
  });

  it('non-important response still submits immediately (single click)', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = {
      thread: {
        id: 'm1',
        sender_name: 'Command',
        body: 'Proceed?',
        responses: [{ text: 'Yes', important: false, available: true }],
      },
    };
    el.shadowRoot.querySelector('.resp-btn').click();
    expect(activateSemanticAction).toHaveBeenCalledTimes(1);
    expect(activateSemanticAction).toHaveBeenCalledWith('comms.respond', {
      source: 'control', detail: { message_id: 'm1', response_index: 0, confirmed: false },
    });
  });

  // ── AC2: unavailable responses are greyed + disabled ────────────────────────

  it('unavailable response is greyed, disabled, and does not submit', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
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
    expect(activateSemanticAction).not.toHaveBeenCalled();
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

  // ── #1380: the open conversation is a live, stable list ─────────────────────

  // jsdom has no layout, so the three measurements the component makes are
  // stubbed. `clientHeight` stands for "the panel is on screen" — 0 while the
  // phone's thread overlay is still display:none. Reading `scrollHeight`
  // records whether the pinned responses existed at that moment: they take
  // height out of the scroller, so a measurement taken before they are created
  // clamps scrollTop short of the newest line.
  function stubScrollBox(el, initialClientHeight) {
    const messages = el.shadowRoot.getElementById('messages');
    const probe = { clientHeight: initialClientHeight, scrollTop: 0, responsesAtMeasure: null };
    Object.defineProperty(messages, 'clientHeight', {
      configurable: true, get: () => probe.clientHeight,
    });
    Object.defineProperty(messages, 'scrollHeight', {
      configurable: true,
      get: () => {
        probe.responsesAtMeasure = !!el.shadowRoot.querySelector('.responses');
        return 1200;
      },
    });
    Object.defineProperty(messages, 'scrollTop', {
      configurable: true,
      get: () => probe.scrollTop,
      set: (v) => { probe.scrollTop = v; },
    });
    return probe;
  }

  const THETA_THREAD = {
    thread: {
      id: 'm2', thread_id: 'theta', sender_name: 'Outpost Theta', body: 'Second',
      responses: [{ text: 'Acknowledged', important: false, available: true }],
    },
    messages: [
      { id: 'm1', thread_id: 'theta', sender_name: 'Outpost Theta', body: 'First' },
      { id: 'm2', thread_id: 'theta', sender_name: 'Outpost Theta', body: 'Second' },
    ],
  };

  it('moves no node when an identical thread is pushed again', () => {
    const { el } = setup();
    el.state = THETA_THREAD;
    expect(el.shadowRoot.querySelectorAll('.msg').length).toBe(2);

    // The repaint runs ten times a second. `appendChild` on a node that is
    // already a child is a remove + re-insert, so appending unconditionally
    // tears the whole conversation out and back every 100ms — which destroys
    // any text the operator has selected inside a hail. An unchanged
    // projection must move NOTHING.
    const messages = el.shadowRoot.getElementById('messages');
    const observer = new MutationObserver(() => {});
    observer.observe(messages, { childList: true });
    el.state = THETA_THREAD;
    const churn = observer.takeRecords();
    observer.disconnect();
    expect(churn).toEqual([]);
  });

  it('opens on the newest line only once the box has layout', () => {
    const { el } = setup();
    const probe = stubScrollBox(el, 0);

    // Rendered while the phone's thread overlay is still hidden: there is
    // nothing to scroll, and the conversation must NOT be recorded as already
    // opened — that would spend its one shot on a box with no layout and leave
    // the operator staring at the OLDEST line when the overlay appears.
    el.state = THETA_THREAD;
    expect(probe.scrollTop).toBe(0);

    // The overlay is on screen now and the SAME conversation repaints: this is
    // the render that owes the operator the newest line.
    probe.clientHeight = 400;
    el.state = THETA_THREAD;
    expect(probe.scrollTop).toBe(1200);

    // And it is one shot per conversation: a later repaint leaves whatever the
    // operator has scrolled to alone.
    probe.scrollTop = 200;
    el.state = THETA_THREAD;
    expect(probe.scrollTop).toBe(200);
  });

  it('pins the responses before measuring the scroll box', () => {
    const { el } = setup();
    const probe = stubScrollBox(el, 400);

    // A desktop/landscape open: the panel is already on screen, so the render
    // that creates the pinned responses is the one that scrolls. Measuring
    // first would clamp scrollTop short by the responses' height — cutting off
    // the very message those responses answer.
    el.state = THETA_THREAD;
    expect(probe.responsesAtMeasure).toBe(true);
    expect(probe.scrollTop).toBe(1200);
  });

});
