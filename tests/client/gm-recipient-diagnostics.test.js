// @vitest-environment jsdom
import { expect, it } from 'vitest';
import { createGmRecipientDiagnostics } from '../../gui/gm-recipient-diagnostics.js';
import { t } from '../../gui/strings.js';

it('shows bounded source-located empty-set feedback and clears an absolute projection', () => {
  document.body.innerHTML = '<section id="gm-mission-panel"></section>';
  const panel = createGmRecipientDiagnostics({ doc: document, t });
  expect(panel.root.hidden).toBe(true);
  panel.update({ recipient_diagnostics: [{ tick: 7, action: 'open_comms',
    message: 'No current player ships matched', source: '<script>.rhai', line: 12 }] });
  expect(panel.root.hidden).toBe(false);
  expect(panel.root.textContent).toContain('No current player ships matched');
  expect(panel.root.querySelector('code').textContent).toBe('<script>.rhai:12');
  expect(panel.root.querySelector('script')).toBeNull();
  panel.update({ recipient_diagnostics: Array.from({ length: 90 }, (_, tick) => ({ tick, action: 'addressed', message: 'empty' })) });
  expect(panel.root.querySelectorAll('li')).toHaveLength(64);
  panel.reset();
  expect(panel.root.hidden).toBe(true);
  expect(panel.root.querySelectorAll('li')).toHaveLength(0);
});
