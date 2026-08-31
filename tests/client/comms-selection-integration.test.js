// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { initConsole } from '../../gui/console-core.js';
import { renderStation as renderComms } from '../../gui/battleship/comms.console.js';
import { COMMS_SELECT_MESSAGE_ACTION_ID } from '../../gui/stations/comms-actions.js';
import '../../gui/components/ph-comms-current-message.js';
import '../../gui/components/ph-comms-hail-list.js';

const STATE = {
  messages: [
    { id: 'm1', sender_name: 'Alpha', body: 'First message', is_read: false },
    { id: 'm2', sender_name: 'Bravo', body: 'Second message', is_read: true },
  ],
  contacts: [],
};

function rows() {
  return [...document.getElementById('comms-hail-list').shadowRoot.querySelectorAll('.row')];
}

function currentSender() {
  return document.getElementById('comms-current-message')
    .shadowRoot.getElementById('sender-label').textContent;
}

function expectSelection(index, sender) {
  const renderedRows = rows();
  expect(renderedRows[index].getAttribute('aria-selected')).toBe('true');
  expect(renderedRows[1 - index].getAttribute('aria-selected')).toBe('false');
  expect(currentSender()).toBe(sender);
  expect(document.querySelector(
    '.semantic-action-feedback__item[data-action-id="comms.select-message"]',
  )?.getAttribute('data-state')).toBe('Applied');
}

describe('Comms local selection semantic action', () => {
  let runtime;

  beforeEach(() => {
    renderComms.resetSelection();
    document.body.innerHTML = `
      <ph-comms-hail-list id="comms-hail-list"></ph-comms-hail-list>
      <ph-comms-current-message id="comms-current-message"></ph-comms-current-message>
    `;
    // Select the native transport branch so the fixture does not open a real
    // BroadcastChannel. A local selection must never reach this transport.
    window.ipc = { postMessage: vi.fn() };
    runtime = initConsole({
      name: 'comms',
      render: renderComms,
    });
    window.__updateConsole('comms', JSON.stringify(STATE));
  });

  afterEach(() => {
    runtime?.disposeSemanticActions();
    renderComms.resetSelection();
    delete window.ipc;
    delete window.activateSemanticAction;
    delete window.__updateConsole;
    delete window.__updateActionFeedback;
    document.body.innerHTML = '';
  });

  it('converges pointer, keyboard and gamepad on one rendered state owner', () => {
    expect(rows().every((row) => row.getAttribute('aria-selected') === 'false')).toBe(true);
    expect(currentSender()).toBe('Alpha');

    rows()[1].click();
    expectSelection(1, 'Bravo');

    document.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'KeyM', key: 'm', bubbles: true, cancelable: true,
    }));
    expectSelection(0, 'Alpha');

    expect(runtime.semanticActions.activate(COMMS_SELECT_MESSAGE_ACTION_ID, {
      context: 'comms', source: 'gamepad',
    })).toMatchObject({ claimed: true, handled: true });
    expectSelection(1, 'Bravo');

    expect(window.ipc.postMessage).not.toHaveBeenCalled();
  });
});
