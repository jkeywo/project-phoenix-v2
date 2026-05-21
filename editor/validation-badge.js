/**
 * validation-badge.js — Pure DOM helpers for rendering validation badges
 * next to fields keyed by `data-validation-path`.
 *
 * The validator (validation.js::validateFile) emits records of shape
 *   { path: string, severity: 'error'|'warning', message: string }
 *
 * Callers wrap their rendered subtree, decorate the fields whose
 * validation path matches by setting `element.dataset.validationPath`,
 * then call `applyValidationResults(rootElement, results)` to attach
 * coloured badges. The helper is idempotent: each call clears any
 * pre-existing badges before re-attaching.
 *
 * The badge layer is purely cosmetic — it never changes the validator's
 * severity. Callers may compute their own severity override (e.g. to
 * render dangling-station errors as yellow warnings) and call
 * `renderValidationBadge` directly.
 */

const BADGE_CLASS = 'validation-badge';

/**
 * Build a single badge element for `record`. Severity defaults to
 * 'error' (red). Returns the appended span.
 */
export function renderValidationBadge(host, record) {
  if (!host || !record) return null;
  const severity = record.severity === 'warning' ? 'warning' : 'error';
  const message = record.message != null ? String(record.message) : '';
  const badge = document.createElement('span');
  badge.className = `${BADGE_CLASS} ${BADGE_CLASS}-${severity}`;
  badge.title = message;
  badge.textContent = '⚠';
  host.appendChild(badge);
  return badge;
}

/**
 * Walk `rootElement` for every `[data-validation-path]` node and attach
 * one badge per matching record. A record matches a node if its `path`
 * equals the node's `data-validation-path` OR is a prefix of it (used
 * so a `trigger[3].action` record decorates a child `entity` field).
 *
 * Duplicate (path, message) pairs against the same host are
 * de-duplicated.
 */
export function applyValidationResults(rootElement, results) {
  if (!rootElement) return;
  clearValidationBadges(rootElement);
  if (!Array.isArray(results) || results.length === 0) return;

  const nodes = collectValidationPathNodes(rootElement);
  for (const node of nodes) {
    const path = node.dataset?.validationPath;
    if (!path) continue;
    const seen = new Set();
    for (const rec of results) {
      if (!rec || typeof rec.path !== 'string') continue;
      if (!matches(rec.path, path)) continue;
      const key = `${rec.severity || 'error'}::${rec.message || ''}`;
      if (seen.has(key)) continue;
      seen.add(key);
      renderValidationBadge(node, rec);
    }
  }
}

/**
 * Remove every `.validation-badge` descendant of `rootElement`.
 */
export function clearValidationBadges(rootElement) {
  if (!rootElement) return;
  const badges = collectByClass(rootElement, BADGE_CLASS);
  for (const b of badges) {
    if (b.parentElement) {
      const idx = b.parentElement.children.indexOf(b);
      if (idx !== -1) b.parentElement.children.splice(idx, 1);
      b.parentElement = null;
    } else if (b.parentNode && typeof b.parentNode.removeChild === 'function') {
      b.parentNode.removeChild(b);
    }
  }
}

// Walk the subtree using `children` (works with both real DOM and the
// test FakeElement shim) since `querySelectorAll('[data-validation-path]')`
// is not supported by the shim.
function collectValidationPathNodes(root) {
  const out = [];
  walk(root, (el) => {
    if (el.dataset && el.dataset.validationPath) out.push(el);
  });
  return out;
}

function collectByClass(root, cls) {
  const out = [];
  walk(root, (el) => {
    if (el.classList && typeof el.classList.contains === 'function' && el.classList.contains(cls)) {
      out.push(el);
    }
  });
  return out;
}

function walk(node, visit) {
  if (!node) return;
  visit(node);
  const kids = node.children;
  if (!kids) return;
  // Iterate over a snapshot — visit might mutate children.
  const snap = Array.from(kids);
  for (const c of snap) walk(c, visit);
}

function matches(recordPath, nodePath) {
  if (recordPath === nodePath) return true;
  // Allow a record on `trigger[3].action` to match a node decorated
  // with `trigger[3].action[2].entity`. The prefix must be followed by
  // a delimiter so `trigger[1]` doesn't match `trigger[10]`.
  if (nodePath.startsWith(recordPath)) {
    const tail = nodePath.slice(recordPath.length);
    if (tail.startsWith('.') || tail.startsWith('[')) return true;
  }
  return false;
}
