import { localiseTree } from './strings.js';

/** One live overlay on browser and native Viewscreen. No retained cue list. */
export function createPresentationCard({ doc = globalThis.document } = {}) {
  const card = doc.createElement('section');
  card.className = 'vs-presentation-card'; card.hidden = true;
  card.tabIndex = 0;
  card.setAttribute('role', 'region'); card.setAttribute('aria-live', 'polite');
  const title = doc.createElement('h1'), body = doc.createElement('p');
  card.append(title, body); doc.body.appendChild(card);
  let previous = null;
  function update(value) {
    const valid = value && ['title', 'incoming'].includes(value.kind)
      && typeof value.title === 'string' && typeof value.body === 'string';
    const key = valid ? JSON.stringify(value) : null;
    if (key === previous) return;
    previous = key;
    card.hidden = !valid;
    const display = valid ? localiseTree(value) : null;
    title.textContent = display?.title || '';
    body.textContent = display?.body || '';
    card.scrollTop = 0;
    card.dataset.kind = valid ? value.kind : '';
    card.setAttribute('aria-label', title.textContent);
  }
  return { update, destroy() { card.remove(); } };
}
