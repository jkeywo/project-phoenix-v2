// @vitest-environment jsdom
//
// Issue #866 — portable saves, the browser half.
//
// The part worth testing without a browser is the CLASSIFICATION: a refused
// import is one of two different pieces of news, and the whole reason the class
// crosses the wasm boundary as its own field is that a host told the wrong one
// goes looking in the wrong place. Everything here drives the real module the
// host page publishes, against the real string table (vitest's setup file loads
// assets/strings/strings.csv), so a renamed id or a collapsed message is a
// failure here rather than a surprise in a bug report.
//
// The download and the file read are DOM mechanics, and they take their document
// and their file as arguments precisely so they can be driven under jsdom.

import { describe, it, expect, vi } from 'vitest';
import { t } from '../../gui/strings.js';
import {
  parseTransferAnswer,
  parsePeekAnswer,
  importRefusalText,
  downloadArtifact,
  readFileText,
} from '../../gui/snapshot-transfer.js';

describe('the bridge answer', () => {
  it('reads an empty answer as success', () => {
    expect(parseTransferAnswer('')).toEqual({ ok: true, kind: '', message: '' });
    expect(parseTransferAnswer(undefined).ok).toBe(true);
  });

  it('splits on the FIRST tab only, so a message may contain one', () => {
    const answer = parseTransferAnswer('incompatible\tformat 9\tbut this build reads 10');
    expect(answer.kind).toBe('incompatible');
    expect(answer.message).toBe('format 9\tbut this build reads 10');
  });

  it('treats a classless refusal as damaged rather than as a version answer', () => {
    // The safe direction: an answer this page does not understand must not be
    // presented as "your build is old", which would send a host to the wrong
    // place with confidence.
    const answer = parseTransferAnswer('something went wrong');
    expect(answer.ok).toBe(false);
    expect(answer.kind).toBe('damaged');
    expect(answer.message).toBe('something went wrong');
  });
});

describe('the two refusals', () => {
  it('renders damaged and incompatible as different sentences', () => {
    const damaged = importRefusalText({ kind: 'damaged', message: 'expected struct Run' });
    const incompatible = importRefusalText({
      kind: 'incompatible',
      message: 'this save was written at payload format 9; this build reads 10',
    });

    expect(damaged).not.toBe(incompatible);
    expect(damaged).toBe(t('server.import_snapshot_damaged', { detail: 'expected struct Run' }));
    expect(incompatible).toBe(
      t('server.import_snapshot_incompatible', {
        detail: 'this save was written at payload format 9; this build reads 10',
      }),
    );
  });

  it('shows the sentence Rust composed verbatim inside the localised frame', () => {
    // The rule the host page has followed since #862: the composed half names
    // WHICH dimension moved, and paraphrasing it throws away the only part
    // worth reading.
    const moved = 'this save was written at payload format 9; this build reads 10';
    expect(importRefusalText({ kind: 'incompatible', message: moved })).toContain(moved);
  });

  it('falls back to the damaged sentence for an unrecognised class', () => {
    expect(importRefusalText({ kind: 'weather', message: 'x' })).toBe(
      t('server.import_snapshot_damaged', { detail: 'x' }),
    );
  });
});

describe('the peek answer', () => {
  it('carries the scenario a good file names', () => {
    expect(parsePeekAnswer('ok\tassets/worlds/combat_test.toml')).toEqual({
      ok: true,
      scenario: 'assets/worlds/combat_test.toml',
      kind: 'ok',
      message: '',
    });
  });

  it('carries the refusal a damaged file earns, and no scenario', () => {
    const answer = parsePeekAnswer('damaged\tthe save could not be parsed: expected struct');
    expect(answer.ok).toBe(false);
    expect(answer.scenario).toBe('');
    expect(importRefusalText(answer)).toContain('expected struct');
  });
});

describe('the download', () => {
  it('offers the text under the given name and cleans up after itself', () => {
    const created = [];
    const revoked = [];
    window.URL.createObjectURL = (blob) => {
      created.push(blob);
      return 'blob:phoenix/1';
    };
    window.URL.revokeObjectURL = (url) => revoked.push(url);
    const clicked = [];
    const realCreate = document.createElement.bind(document);
    vi.spyOn(document, 'createElement').mockImplementation((tag) => {
      const el = realCreate(tag);
      if (tag === 'a') el.click = () => clicked.push({ href: el.href, name: el.download });
      return el;
    });

    const started = downloadArtifact(document, 'phoenix-save.ron', '(scenario: "x")');

    expect(started).toBe(true);
    expect(clicked).toEqual([{ href: 'blob:phoenix/1', name: 'phoenix-save.ron' }]);
    expect(created).toHaveLength(1);
    expect(document.querySelector('a[download]')).toBeNull();
    vi.restoreAllMocks();
  });

  it('does nothing when there is no text to offer', () => {
    // An export whose capture has not happened yet hands back "", and a
    // zero-byte file is a worse answer than no file.
    expect(downloadArtifact(document, 'phoenix-save.ron', '')).toBe(false);
  });
});

describe('reading a picked file', () => {
  it('prefers File.text() where the browser has it', async () => {
    const file = { text: () => Promise.resolve('(scenario: "x")') };
    await expect(readFileText(file)).resolves.toBe('(scenario: "x")');
  });

  it('falls back to a FileReader where it does not', async () => {
    // A REAL `File` with `text()` taken away for the duration, rather than a
    // hand-built stand-in: `FileReader` reads a file through internals a fake
    // object does not have, so a stand-in would prove the fallback works on
    // something no browser will ever hand it.
    const original = Blob.prototype.text;
    delete Blob.prototype.text;
    try {
      const file = new File(['(scenario: "y")'], 'phoenix-save.ron', { type: 'text/plain' });
      expect(typeof file.text).toBe('undefined');
      await expect(readFileText(file)).resolves.toBe('(scenario: "y")');
    } finally {
      Blob.prototype.text = original;
    }
  });

  it('rejects when handed nothing', async () => {
    await expect(readFileText(null)).rejects.toThrow();
  });
});
