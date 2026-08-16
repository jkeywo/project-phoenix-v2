/**
 * gui/snapshot-transfer.js — the browser half of portable saves (issue #866).
 *
 * A save that can leave this browser and arrive in another one needs three
 * things the host page cannot express as markup: a file to download, a file to
 * read, and — the part worth testing — a REFUSAL that says which of the two bad
 * outcomes happened.
 *
 * # Two refusals, not one
 *
 * `wasm_prepare_import` answers `"<class>\t<message>"`, and the class is Rust's
 * own reading of `snapshot::LoadRefusal` rather than something inferred here:
 *
 *   - `damaged` — the file is not a save this build can parse. Truncated,
 *     hand-edited, or never a save at all. Pick another file.
 *   - `incompatible` — the file is intact and this build cannot honour it. The
 *     message is `vellum_save::Moved`'s own sentence, which names WHICH of the
 *     three version dimensions moved and to what.
 *
 * They are different instructions to a human, so they are different sentences,
 * and the sentence Rust composed is always shown verbatim inside the localised
 * frame rather than paraphrased — for the reason the host page's snapshot
 * status has always given: the composed half is the only part worth reading.
 *
 * Kept as a module (rather than inline in server.html) so both halves are
 * reachable from vitest: the classification is pure, and the download and the
 * file read take their document and their file as arguments.
 */

import { t } from './strings.js';

/** The class a `"<class>\t<message>"` answer carries when it names none. */
const UNKNOWN_CLASS = 'damaged';

/**
 * Split a tab-separated bridge answer into its class and its message.
 *
 * Only the first tab separates; a message may contain more (Rust's own
 * `wasm_snapshot_status` makes the same promise), so everything past it is
 * rejoined rather than dropped.
 *
 * @param {string} raw
 * @returns {{ ok: boolean, kind: string, message: string }}
 */
export function parseTransferAnswer(raw) {
  const text = typeof raw === 'string' ? raw : '';
  if (text === '') return { ok: true, kind: '', message: '' };
  const at = text.indexOf('\t');
  if (at < 0) return { ok: false, kind: UNKNOWN_CLASS, message: text };
  return {
    ok: false,
    kind: text.slice(0, at) || UNKNOWN_CLASS,
    message: text.slice(at + 1),
  };
}

/**
 * The sentence to show a host for a refused import.
 *
 * `incompatible` and `damaged` resolve to different string ids — that is AC5,
 * and it is the whole reason the class crosses the boundary at all. An
 * unrecognised class is treated as damaged: a refusal this page does not
 * understand is not one it should present as a version answer.
 *
 * @param {{ kind: string, message: string }} answer
 * @returns {string}
 */
export function importRefusalText(answer) {
  const id =
    answer.kind === 'incompatible'
      ? 'server.import_snapshot_incompatible'
      : 'server.import_snapshot_damaged';
  return t(id, { detail: answer.message });
}

/**
 * Split `wasm_peek_import`'s answer, which is the same shape with `ok` in the
 * class position and the scenario path as the message.
 *
 * @param {string} raw
 * @returns {{ ok: boolean, scenario: string, kind: string, message: string }}
 */
export function parsePeekAnswer(raw) {
  const answer = parseTransferAnswer(raw);
  if (answer.kind === 'ok') {
    return { ok: true, scenario: answer.message, kind: 'ok', message: '' };
  }
  return { ok: false, scenario: '', kind: answer.kind, message: answer.message };
}

/**
 * Hand `text` to the browser as a download named `name`.
 *
 * An object URL and a synthetic click, which is the only way a page with no
 * server round-trip can produce a file. Revoked on the next task rather than
 * immediately: Chrome reads the URL asynchronously after the click, and
 * revoking in the same tick cancels the download it was about to start.
 *
 * @param {Document} doc
 * @param {string} name
 * @param {string} text
 * @returns {boolean} whether the download was started
 */
export function downloadArtifact(doc, name, text) {
  if (!doc || !text) return false;
  const view = doc.defaultView;
  const url = view && view.URL ? view.URL : null;
  if (!url || typeof url.createObjectURL !== 'function') return false;
  // `text/plain`, because that is what it is: `Store` moves `String` and the
  // record is RON. A save a human can open is the feature, not a leak.
  const blob = new view.Blob([text], { type: 'text/plain;charset=utf-8' });
  const href = url.createObjectURL(blob);
  const anchor = doc.createElement('a');
  anchor.href = href;
  anchor.download = name;
  anchor.style.display = 'none';
  doc.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  if (typeof view.setTimeout === 'function') {
    view.setTimeout(function () {
      try {
        url.revokeObjectURL(href);
      } catch (_) {
        /* the download already took it */
      }
    }, 0);
  }
  return true;
}

/**
 * Read a picked file as text.
 *
 * `File.text()` where it exists and a `FileReader` where it does not, because
 * the smoke suite's Chromium has the first and jsdom has only the second.
 *
 * @param {File} file
 * @returns {Promise<string>}
 */
export function readFileText(file) {
  if (!file) return Promise.reject(new Error('no file'));
  if (typeof file.text === 'function') return file.text();
  return new Promise(function (resolve, reject) {
    const reader = new FileReader();
    reader.onload = function () {
      resolve(String(reader.result || ''));
    };
    reader.onerror = function () {
      reject(reader.error || new Error('the file could not be read'));
    };
    reader.readAsText(file);
  });
}
