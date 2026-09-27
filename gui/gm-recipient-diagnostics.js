/** Read-only, absolute authoring feedback. Never submits a GM action. */
export function createGmRecipientDiagnostics({ doc = document, t }) {
  const host = doc.getElementById('gm-mission-panel');
  const root = doc.createElement('details');
  root.id = 'gm-recipient-diagnostics';
  const heading = doc.createElement('summary');
  heading.textContent = t('gm.recipients.diagnostics');
  const list = doc.createElement('ol');
  list.style.maxHeight = '20rem';
  list.style.overflow = 'auto';
  root.append(heading, list);
  root.hidden = true;
  host?.append(root);
  let signature = '';
  function update(value) {
    const rows = Array.isArray(value?.recipient_diagnostics)
      ? value.recipient_diagnostics.slice(-64) : [];
    const next = JSON.stringify(rows);
    if (next === signature) return;
    signature = next;
    root.hidden = rows.length === 0;
    list.replaceChildren(...rows.map(row => {
      const item = doc.createElement('li');
      const label = doc.createElement('strong');
      label.textContent = t('gm.recipients.diagnostic_at', { tick: row.tick, action: row.action });
      const message = doc.createElement('p');
      message.textContent = String(row.message || '');
      const source = doc.createElement('code');
      source.textContent = `${row.source || ''}${row.line ? `:${row.line}` : ''}`;
      item.append(label, message, source);
      return item;
    }));
  }
  return { update, reset: () => update({}), root };
}
