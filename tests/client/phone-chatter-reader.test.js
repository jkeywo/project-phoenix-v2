// @vitest-environment jsdom

/**
 * tests/client/phone-chatter-reader.test.js — gui/phone-chatter-reader.js
 * (issue #1429, PRD #1418 stories 20-21).
 *
 * The phone Viewscreen's level-3 Coordination reading surface: open/close,
 * the shared modal focus contract (focus moves in, Escape and a backdrop
 * click dismiss, focus returns to whatever opened it), and the "pin" property
 * — content shown is a snapshot, never re-read from anywhere else, so a
 * message that has since been evicted from the chatter stream (or a flood of
 * new arrivals) cannot move or replace what is on screen while it is open.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { mountPhoneChatterReader, CHATTER_READER_IDS } from '../../gui/phone-chatter-reader.js';

beforeEach(() => {
  document.body.innerHTML = '';
});

function content(overrides = {}) {
  return {
    sender: 'Sensors → Tactical',
    from: 'Sensors',
    title: 'Contact bearing shift',
    body: 'Bearing has drifted three degrees since last report.',
    ...overrides,
  };
}

describe('mountPhoneChatterReader', () => {
  it('builds the overlay hidden and closed', () => {
    const reader = mountPhoneChatterReader(document);
    const overlay = document.getElementById(CHATTER_READER_IDS.overlay);
    expect(overlay).toBeTruthy();
    expect(overlay.hidden).toBe(true);
    expect(overlay.getAttribute('aria-hidden')).toBe('true');
    expect(reader.isOpen()).toBe(false);
  });

  it('is idempotent: mounting twice adopts the same nodes rather than duplicating them', () => {
    mountPhoneChatterReader(document);
    mountPhoneChatterReader(document);
    expect(document.querySelectorAll(`#${CHATTER_READER_IDS.overlay}`).length).toBe(1);
  });

  it('open() shows the overlay and paints the sender, title and body', () => {
    const reader = mountPhoneChatterReader(document);
    reader.open(content());
    expect(reader.isOpen()).toBe(true);
    expect(document.getElementById(CHATTER_READER_IDS.sender).textContent).toBe('Sensors → Tactical');
    expect(document.getElementById(CHATTER_READER_IDS.title).textContent).toBe('Contact bearing shift');
    expect(document.getElementById(CHATTER_READER_IDS.body).textContent)
      .toBe('Bearing has drifted three degrees since last report.');
  });

  it('falls back to `from` when no combined `sender` is supplied, and never throws on an empty snapshot', () => {
    const reader = mountPhoneChatterReader(document);
    expect(() => reader.open({})).not.toThrow();
    expect(document.getElementById(CHATTER_READER_IDS.sender).textContent).toBe('');
    reader.open({ from: 'AI' });
    expect(document.getElementById(CHATTER_READER_IDS.sender).textContent).toBe('AI');
  });

  it('moves focus into the surface on open (the close button, its one control)', () => {
    const reader = mountPhoneChatterReader(document);
    reader.open(content());
    expect(document.activeElement.id).toBe(CHATTER_READER_IDS.close);
  });

  it('returns focus to whatever opened it, on close', () => {
    const reader = mountPhoneChatterReader(document);
    const bubble = document.createElement('div');
    bubble.tabIndex = 0;
    document.body.appendChild(bubble);
    bubble.focus();
    expect(document.activeElement).toBe(bubble);

    reader.open(content());
    expect(document.activeElement).toBe(document.getElementById(CHATTER_READER_IDS.close));

    reader.close();
    expect(reader.isOpen()).toBe(false);
    expect(document.activeElement).toBe(bubble);
  });

  it('Escape dismisses it', () => {
    const reader = mountPhoneChatterReader(document);
    reader.open(content());
    expect(reader.isOpen()).toBe(true);
    document.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(reader.isOpen()).toBe(false);
  });

  it('the close button dismisses it', () => {
    const reader = mountPhoneChatterReader(document);
    reader.open(content());
    document.getElementById(CHATTER_READER_IDS.close).click();
    expect(reader.isOpen()).toBe(false);
  });

  it('a backdrop click dismisses it; a click inside the panel does not', () => {
    const reader = mountPhoneChatterReader(document);
    reader.open(content());
    const overlay = document.getElementById(CHATTER_READER_IDS.overlay);
    const panel = document.getElementById(CHATTER_READER_IDS.panel);

    panel.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
    expect(reader.isOpen()).toBe(true);

    overlay.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
    expect(reader.isOpen()).toBe(false);
  });

  it('pins its content: nothing else arriving in the document changes what is shown', () => {
    const reader = mountPhoneChatterReader(document);
    reader.open(content({ body: 'First message body.' }));

    // A new bubble "arrives" elsewhere in the document — the reader has no
    // subscription to react to it, by construction.
    const container = document.createElement('div');
    container.id = 'chatter-container';
    document.body.appendChild(container);
    const newBubble = document.createElement('div');
    newBubble.className = 'chatter-bubble';
    newBubble.textContent = 'A second, newer message';
    container.appendChild(newBubble);

    expect(document.getElementById(CHATTER_READER_IDS.body).textContent).toBe('First message body.');

    // Only a fresh open() call — a new tap — replaces the pinned content.
    reader.open(content({ body: 'Second message body.' }));
    expect(document.getElementById(CHATTER_READER_IDS.body).textContent).toBe('Second message body.');
  });
});
