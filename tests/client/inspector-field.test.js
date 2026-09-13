// @vitest-environment jsdom
import { expect, it } from 'vitest';
import { renderInspectorMetadata, renderInspectorReadOnly } from '../../gui/inspector-field.js';
import { npcDescriptor } from '../fixtures/npc-live-inspector.js';
it('renders actual source locations in the shared field presentation and never invents one for runtime-only readings', () => {
  const node = document.createElement('p');
  const t = (id, args) => `${id}${args ? JSON.stringify(args) : ''}`;
  const field = { ...npcDescriptor('float', 'recreate-required', 'global.ai_tick_hz'), default_source: '4.0',
    origin: { schema_path: 'global.ai_tick_hz', document: 'assets/worlds/example.toml', line: 7, layer: null } };
  renderInspectorMetadata(node, field, { t });
  expect(node.textContent).toContain('assets/worlds/example.toml'); expect(node.textContent).toContain('"line":"7"');
  expect(node.textContent).toContain('4.0');
  field.origin.document = null; field.origin.line = null;
  renderInspectorMetadata(node, field, { t });
  expect(node.textContent).toContain('inspector.location_unavailable'); expect(node.textContent).not.toContain('example.toml');
  const row = document.createElement('div');
  renderInspectorReadOnly(row, '<script>unsafe</script>', field, { t, label: 'Definition' });
  expect(row.querySelector('script')).toBeNull(); expect(row.querySelector('input').value).toBe('<script>unsafe</script>');
  expect(row.querySelector('input').disabled).toBe(true);
});
