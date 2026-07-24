import { describe, it, expect } from 'vitest';
import {
  ClientCommsState, commsState, effectiveThreadId,
  hailMessage, selectCommsMessage, respondToMessage, clearCommsMessage,
} from '../../gui/comms-state.js';

function msg(id, overrides = {}) {
  return {
    id,
    sender_uuid: 's-uuid',
    sender_name: 'Starbase',
    subject: 'Hello',
    body: 'Body text',
    responses: ['Ack'],
    selected_response: null,
    is_read: false,
    is_orphaned: false,
    sender_in_range: true,
    thread_id: id,
    is_urgent: false,
    ...overrides,
  };
}

function msgInThread(id, thread, overrides = {}) {
  return msg(id, { thread_id: thread, ...overrides });
}

function contact(uuid, name, inRange = true) {
  return { uuid, name, in_range: inRange, is_urgent: false };
}

function commsStateMsg(messages, contacts = [], objectives = []) {
  return { type: 'CommsState', data: { messages, objectives, contacts } };
}

describe('defaults', () => {
  it('starts empty', () => {
    const s = new ClientCommsState();
    expect(s.messages).toEqual([]);
    expect(s.objectives).toEqual([]);
    expect(s.contacts).toEqual([]);
    expect(s.selectedThreadId).toBeNull();
    expect(s.version).toBe(0);
    expect(s.isDirty()).toBe(false);
    expect(commsState).toBeInstanceOf(ClientCommsState);
  });
});

describe('apply CommsState', () => {
  it('replaces messages, objectives and contacts and bumps the version', () => {
    const s = new ClientCommsState();
    const obj = { id: 'obj1', text: 'Make contact', mandatory: true, status: 'Active', targets: [] };
    s.apply(commsStateMsg([msg('m1')], [contact('c1', 'Alpha')], [obj]));
    expect(s.messages.map(m => m.id)).toEqual(['m1']);
    expect(s.contacts[0].name).toBe('Alpha');
    expect(s.objectives).toEqual([obj]);
    expect(s.version).toBe(1);
    expect(s.isDirty()).toBe(true);
  });

  it('replaces previous data wholesale', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([msg('m1')], [contact('c1', 'Alpha')]));
    s.apply(commsStateMsg([msg('m2'), msg('m3')]));
    expect(s.messages.map(m => m.id)).toEqual(['m2', 'm3']);
    expect(s.contacts).toEqual([]);
    expect(s.version).toBe(2);
  });

  it('preserves the selected thread when its messages remain', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([msg('m1'), msg('m2')]));
    s.selectThread('m1');
    s.apply(commsStateMsg([msg('m1')]));
    expect(s.selectedThreadId).toBe('m1');
  });

  it('clears the selected thread when its messages disappear', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([msg('m1')]));
    s.selectThread('m1');
    s.apply(commsStateMsg([msg('m2')]));
    expect(s.selectedThreadId).toBeNull();
  });

  it('ignores non-CommsState messages', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([msg('m1')]));
    const before = s.version;
    s.apply({ type: 'GameStarted' });
    s.apply({ type: 'SimState', data: { snapshot: {} } });
    expect(s.version).toBe(before);
    expect(s.messages).toHaveLength(1);
  });
});

describe('effectiveThreadId', () => {
  it('falls back to the message id when thread_id is empty (legacy payloads)', () => {
    expect(effectiveThreadId(msg('m1', { thread_id: '' }))).toBe('m1');
    expect(effectiveThreadId(msgInThread('m1', 't9'))).toBe('t9');
  });

  it('legacy empty thread_id messages form their own thread', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([msg('m1', { thread_id: '' })]));
    s.selectThread('m1');
    expect(s.selectedThreadId).toBe('m1');
    expect(s.threadMessages('m1')).toHaveLength(1);
  });
});

describe('selectThread / clearSelection', () => {
  it('selects only existing threads and bumps the version', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([msg('m1')]));
    s.selectThread('ghost');
    expect(s.selectedThreadId).toBeNull();
    s.selectThread('m1');
    expect(s.selectedThreadId).toBe('m1');
    expect(s.version).toBe(2);
  });

  it('clearSelection only bumps version when something was selected', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([msg('m1')]));
    s.clearSelection(); // nothing selected — no bump
    expect(s.version).toBe(1);
    s.selectThread('m1');
    s.clearSelection();
    expect(s.selectedThreadId).toBeNull();
    expect(s.version).toBe(3);
  });
});

describe('threadMessages / activeMessageForThread', () => {
  it('returns messages of one thread in inbox order', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([
      msgInThread('m1', 't1'), msgInThread('m2', 't2'), msgInThread('m3', 't1'),
    ]));
    expect(s.threadMessages('t1').map(m => m.id)).toEqual(['m1', 'm3']);
  });

  it('active message is the LAST unresponded, in-range, non-orphaned message', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([
      msgInThread('m1', 't1', { selected_response: 0 }),
      msgInThread('m2', 't1'),
    ]));
    expect(s.activeMessageForThread('t1').id).toBe('m2');
  });

  it('orphaned, out-of-range, and responseless messages are not active', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([
      msgInThread('m1', 't1', { is_orphaned: true }),
      msgInThread('m2', 't2', { sender_in_range: false }),
      msgInThread('m3', 't3', { responses: [] }),
    ]));
    expect(s.activeMessageForThread('t1')).toBeNull();
    expect(s.activeMessageForThread('t2')).toBeNull();
    expect(s.activeMessageForThread('t3')).toBeNull();
  });
});

describe('availableResponses / responseButtonsEnabled', () => {
  it('returns responses until one is selected', () => {
    const s = new ClientCommsState();
    expect(s.availableResponses(msg('m1', { responses: ['A', 'B'] }))).toEqual(['A', 'B']);
    expect(s.availableResponses(msg('m1', { responses: ['A'], selected_response: 0 }))).toEqual([]);
  });

  it('passes through object-shaped responses (#761 text/important/available)', () => {
    // Post-#761 the wire carries per-response objects; availableResponses
    // returns them verbatim so the component can render important/available.
    const s = new ClientCommsState();
    const responses = [
      { text: 'Arm it', important: true, available: true },
      { text: 'Reply', important: false, available: false },
    ];
    expect(s.availableResponses(msg('m1', { responses }))).toEqual(responses);
    expect(s.availableResponses(msg('m1', { responses, selected_response: 0 }))).toEqual([]);
  });

  it('buttons enabled only when the selected thread has an active message', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([msg('m1'), msg('m2', { responses: [] })]));
    expect(s.responseButtonsEnabled()).toBe(false); // nothing selected
    s.selectThread('m1');
    expect(s.responseButtonsEnabled()).toBe(true);
    s.selectThread('m2');
    expect(s.responseButtonsEnabled()).toBe(false);
  });
});

describe('canHail', () => {
  it('true only for known, in-range contacts', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([], [contact('c1', 'Alpha', true), contact('c2', 'Beta', false)]));
    expect(s.canHail('c1')).toBe(true);
    expect(s.canHail('c2')).toBe(false);
    expect(s.canHail('ghost')).toBe(false);
  });
});

describe('isDirty / markClean', () => {
  it('tracks the version counter against the clean watermark', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([msg('m1')]));
    expect(s.isDirty()).toBe(true);
    s.markClean();
    expect(s.isDirty()).toBe(false);
    s.selectThread('m1');
    expect(s.isDirty()).toBe(true);
  });
});

describe('sortedThreads', () => {
  it('summarises each thread once with metadata from the latest message', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([
      msgInThread('m1', 't1', { sender_name: 'Old', subject: 'First', is_read: true }),
      msgInThread('m2', 't1', { sender_name: 'New', subject: 'Latest', is_read: true, sender_in_range: false, is_orphaned: true }),
    ]));
    const threads = s.sortedThreads();
    expect(threads).toHaveLength(1);
    expect(threads[0]).toEqual({
      thread_id: 't1',
      sender_name: 'New',
      subject: 'Latest',
      any_unread: false,
      any_urgent: false,
      latest_out_of_range: true,
      latest_orphaned: true,
    });
  });

  it('orders urgent+unread first, then unread, then read — stable within groups', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([
      msgInThread('m1', 'read1', { is_read: true }),
      msgInThread('m2', 'unread1', { is_read: false }),
      msgInThread('m3', 'read2', { is_read: true }),
      msgInThread('m4', 'urgent1', { is_read: false, is_urgent: true }),
      msgInThread('m5', 'unread2', { is_read: false }),
    ]));
    expect(s.sortedThreads().map(t => t.thread_id))
      .toEqual(['urgent1', 'unread1', 'unread2', 'read1', 'read2']);
  });

  it('an urgent message that is already read does not mark the thread urgent', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([
      msgInThread('m1', 't1', { is_read: true, is_urgent: true }),
      msgInThread('m2', 't2', { is_read: false }),
    ]));
    const threads = s.sortedThreads();
    expect(threads[0].thread_id).toBe('t2'); // unread beats read-urgent
    expect(threads.find(t => t.thread_id === 't1').any_urgent).toBe(false);
  });
});

describe('multi-speaker thread summaries', () => {
  it('uses contact name as the thread sender when a multi-speaker thread shares one channel', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([
      msgInThread('m1', 'research-scholar', {
        sender_uuid: 'research-uuid',
        sender_name: 'Research Outpost',
        subject: 'Stand by',
      }),
      msgInThread('m2', 'research-scholar', {
        sender_uuid: 'research-uuid',
        sender_name: 'Dr. Myst',
        subject: 'Signal analysis',
      }),
    ], [contact('research-uuid', 'Research Outpost')]));

    const [thread] = s.sortedThreads();
    expect(thread.sender_name).toBe('Research Outpost');
    expect(thread.subject).toBe('Signal analysis');
  });

  it('falls back to latest speaker name for synthetic broadcasts without contacts', () => {
    const s = new ClientCommsState();
    s.apply(commsStateMsg([
      msgInThread('m1', 'starcorp-command', {
        sender_uuid: 'Starcorp Command',
        sender_name: 'Starcorp Command',
      }),
      msgInThread('m2', 'starcorp-command', {
        sender_uuid: 'Starcorp Command',
        sender_name: 'Admiral Vale',
      }),
    ]));

    const [thread] = s.sortedThreads();
    expect(thread.sender_name).toBe('Admiral Vale');
  });
});

describe('outbound message builders', () => {
  it('build serde tag/content wire objects', () => {
    // Post-#822: full ControlSystem envelopes targeting the comms system.
    expect(hailMessage('u1')).toEqual({
      type: 'ControlSystem',
      data: { target: 'comms', payload: { type: 'Hail', data: { target_uuid: 'u1' } } },
    });
    expect(selectCommsMessage('m1')).toEqual({
      type: 'ControlSystem',
      data: { target: 'comms', payload: { type: 'SelectCommsMessage', data: { message_id: 'm1' } } },
    });
    expect(respondToMessage('m1', 2)).toEqual({
      type: 'ControlSystem',
      data: {
        target: 'comms',
        payload: { type: 'RespondToMessage', data: { message_id: 'm1', response_index: 2 } },
      },
    });
    expect(clearCommsMessage()).toEqual({
      type: 'ControlSystem',
      data: { target: 'comms', payload: { type: 'ClearComms' } },
    });
  });
});
