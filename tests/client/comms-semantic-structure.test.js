import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const HULLS = ['battleship', 'cruiser'];
const COMPONENTS = [
  ['gui/components/ph-comms-contact-list.js', 'COMMS_HAIL_ACTION_ID', "sendAction('hail'"],
  [
    'gui/components/ph-comms-hail-list.js',
    'COMMS_SELECT_MESSAGE_ACTION_ID',
    "sendAction('select_comms_message'",
  ],
  [
    'gui/components/ph-comms-current-message.js',
    'COMMS_RESPOND_ACTION_ID',
    "sendAction('respond_to_message'",
  ],
];

function source(path) {
  return readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');
}

describe('Comms semantic-action structural coverage', () => {
  for (const hull of HULLS) {
    it(`${hull} mounts the complete shared Comms surface and context`, () => {
      const html = source(`gui/${hull}/comms.html`);
      expect(html).toContain('<ph-comms-contact-list');
      expect(html).toContain('<ph-comms-hail-list');
      expect(html).toContain('<ph-comms-current-message');
      expect(html).toMatch(/initConsole\s*\(\s*\{[\s\S]*?name:\s*'comms'/);
      expect(source(`gui/${hull}/comms.console.js`)).toContain('makeCommsRender');
    });

    // Issue #1380: HAILS | CONTACTS over one list area, and the open thread in
    // a local overlay panel with a Back button and NO tab code — the shell's
    // Station Bar only advertises `.overlay-panel[data-tab-code]`, so this
    // panel stays out of the bar by construction rather than by a filter.
    it(`${hull} puts hails and contacts behind one tab pair, with the thread as a local panel`, () => {
      const html = source(`gui/${hull}/comms.html`);
      expect(html).toContain('id="comms-seg"');
      expect(html).toContain('data-i18n="console.comms.seg.hails"');
      expect(html).toContain('data-i18n="console.comms.seg.contacts"');
      expect(html).toContain('id="comms-hails-unread"');
      expect(html).toContain('initConsoleOverlays');
      const panel = html.match(/<div[^>]*id="comms-thread-panel"[^>]*>/);
      expect(panel).not.toBeNull();
      expect(panel[0]).toContain('overlay-panel');
      expect(panel[0]).not.toContain('data-tab-code');
      expect(html).toContain('data-overlay-back');
      expect(html).toContain('data-i18n="console.common.back"');
    });
  }

  for (const [path, semanticId, forbiddenDirectCall] of COMPONENTS) {
    it(`${path} activates its semantic identity and cannot bypass it`, () => {
      const text = source(path);
      expect(text).toContain('activateSemanticAction');
      expect(text).toContain(semanticId);
      expect(text).not.toContain(forbiddenDirectCall);
    });
  }

  it('the shared adapter owns every Comms action-map command and exact response detail', () => {
    const text = source('gui/stations/comms-actions.js');
    for (const action of [
      "sendAction('hail'",
      "sendAction('respond_to_message'",
      "sendAction('clear_comms'",
      "sendAction('show_on_screen'",
    ]) expect(text).toContain(action);
    expect(text).toContain('selectMessage(selected)');
    // Issue #1380: the inbox lists threads, so the one selection identity
    // routes a `thread_id` detail to the renderer's thread seam and a
    // `message_id` detail to the message seam — still no host command either
    // way, and still the same local Applied lifecycle.
    expect(text).toContain('selectThread(thread)');
    expect(text).toContain('detail.thread_id');
    expect(text).not.toContain("sendAction('select_comms_message'");
    expect(text).toContain('response_index');
    expect(text).toContain('confirmed === true');
  });

  it('no shipped client route can produce the retired SelectCommsMessage command', () => {
    const actionMap = source('gui/action-map.js');
    const commsState = source('gui/comms-state.js');
    expect(actionMap).not.toContain('select_comms_message');
    expect(actionMap).not.toContain("type: 'SelectCommsMessage'");
    expect(commsState).not.toContain('function selectCommsMessage');
    expect(commsState).not.toContain("type: 'SelectCommsMessage'");
  });
});
