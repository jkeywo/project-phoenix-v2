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
      expect(html).toContain(`initConsole({ name: 'comms'`);
      expect(source(`gui/${hull}/comms.console.js`)).toContain('makeCommsRender');
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
