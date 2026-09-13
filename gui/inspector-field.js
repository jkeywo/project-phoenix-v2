/** The same field metadata presentation in Authoring and Live. Authority stays
 * with document patching or the one named runtime action, never this renderer. */
const text = value => typeof value === 'string';
export function validInspectorDescriptor(field) {
  return !!field && text(field.kind)
    && ['named-action', 'derived', 'recreate-required'].includes(field.live_mutability)
    && (field.default_source == null || text(field.default_source))
    && !!field.origin && text(field.origin.schema_path)
    && (field.origin.document == null || text(field.origin.document))
    && (field.origin.layer == null || text(field.origin.layer))
    && (field.origin.line == null || (Number.isSafeInteger(field.origin.line) && field.origin.line > 0))
    && Array.isArray(field.validation) && field.validation.every(text);
}

export function renderInspectorMetadata(node, field, { t = id => id } = {}) {
  if (!node) return;
  if (!validInspectorDescriptor(field)) { node.textContent = ''; return; }
  node.dataset.mutability = field.live_mutability;
  const lines = [t('inspector.type', { type: field.kind }), t(`inspector.mutability.${field.live_mutability}`)];
  if (field.default_source != null) lines.push(t('inspector.default', { value: field.default_source }));
  lines.push(t('inspector.schema', { path: field.origin.schema_path }));
  if (field.origin.document) lines.push(field.origin.line == null
    ? t('inspector.document', { path: field.origin.document })
    : t('inspector.location', { path: field.origin.document, line: String(field.origin.line) }));
  else lines.push(t('inspector.location_unavailable'));
  if (field.origin.layer) lines.push(t('inspector.layer', { layer: field.origin.layer }));
  for (const rule of field.validation) lines.push(t(rule));
  node.textContent = lines.join(' · ');
}

export function renderInspectorReadOnly(node, value, descriptor, { t = id => id, label } = {}) {
  if (!node) return;
  let input = node.querySelector('input');
  let metadata = node.querySelector('[data-inspector-metadata]');
  if (!input) {
    input = node.ownerDocument.createElement('input');
    input.type = 'text'; input.disabled = true; input.readOnly = true;
    metadata = node.ownerDocument.createElement('p');
    metadata.dataset.inspectorMetadata = '';
    node.replaceChildren(input, metadata);
  }
  input.value = value || '';
  input.setAttribute('aria-label', t(label));
  node.dataset.mutability = descriptor?.live_mutability || '';
  renderInspectorMetadata(metadata, descriptor, { t });
}
