/** Keep a partly entered command and its place when language repaints a surface. */
export function preserveLocaleEditContext(doc, repaint) {
  const descendants = (root) => {
    const nodes = [...root.querySelectorAll('*')];
    return nodes.flatMap((node) => node.shadowRoot
      ? [node, ...descendants(node.shadowRoot)] : [node]);
  };
  const before = descendants(doc);
  const fields = before.filter((node) => node.matches('input, textarea, select'));
  let active = doc.activeElement;
  while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
  const focusedIndex = fields.indexOf(active);
  const values = fields.map((field) => ({
    id: field.id,
    name: field.name,
    value: field.value,
    checked: field.checked,
    start: typeof field.selectionStart === 'number' ? field.selectionStart : null,
    end: typeof field.selectionEnd === 'number' ? field.selectionEnd : null,
  }));
  const scrollers = before.filter((element) => element.id)
    .filter((element) => element.scrollTop || element.scrollLeft)
    .map((element) => ({ id: element.id, top: element.scrollTop, left: element.scrollLeft }));
  repaint();
  const after = descendants(doc);
  const current = after.filter((node) => node.matches('input, textarea, select'));
  values.forEach((saved, index) => {
    const field = (saved.id && current.find((candidate) => candidate.id === saved.id))
      || current.find((candidate) => saved.name && candidate.name === saved.name)
      || current[index];
    if (!field) return;
    field.value = saved.value;
    if (typeof saved.checked === 'boolean') field.checked = saved.checked;
    if (index === focusedIndex) {
      field.focus();
      if (saved.start !== null && typeof field.setSelectionRange === 'function') {
        field.setSelectionRange(saved.start, saved.end);
      }
    }
  });
  for (const saved of scrollers) {
    const element = after.find((candidate) => candidate.id === saved.id);
    if (element) { element.scrollTop = saved.top; element.scrollLeft = saved.left; }
  }
}
