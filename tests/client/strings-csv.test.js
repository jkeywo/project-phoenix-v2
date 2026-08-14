import { describe, it, expect } from 'vitest';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { csvField, rowLineNumbers } from '../../scripts/strings-csv.mjs';
import { parseCsv } from '../../gui/strings.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

describe('csvField', () => {
  it('leaves a plain value alone', () => {
    expect(csvField('[Ready]')).toBe('[Ready]');
  });

  it('quotes a value containing a comma', () => {
    // The issue #966 shape: unquoted, this splits the row and truncates the text.
    expect(csvField('[Hold the line, Captain]')).toBe('"[Hold the line, Captain]"');
  });

  it('quotes and doubles an embedded quote', () => {
    expect(csvField('He said "go"')).toBe('"He said ""go"""');
  });

  it('quotes a multi-line value', () => {
    expect(csvField('one\ntwo')).toBe('"one\ntwo"');
  });

  it('round-trips through parseCsv', () => {
    const row = ['some.id', 'Lobby, pre-connection', '[Ready, "Captain"]\nnow'];
    expect(parseCsv(`${row.map(csvField).join(',')}\n`)[0]).toEqual(row);
  });
});

describe('rowLineNumbers', () => {
  const linesOf = (text) => rowLineNumbers(text, parseCsv(text));

  it('numbers a plain file one row per line', () => {
    expect(linesOf('id,en\na,b\nc,d\n')).toEqual([1, 2, 3]);
  });

  it('skips the lines a multi-line quoted value swallows', () => {
    // The whole point: `rows[2]` lives on line 4, not line 3.
    const text = 'id,en\nhail,"Line one\nLine two"\ntail,z\n';
    expect(linesOf(text)).toEqual([1, 2, 4]);
  });

  it('steps over a blank line that parseCsv drops', () => {
    const text = 'id,en\na,b\n\nc,d\n';
    expect(parseCsv(text)).toHaveLength(3);
    expect(linesOf(text)).toEqual([1, 2, 4]);
  });

  it('handles CRLF line endings', () => {
    expect(linesOf('id,en\r\na,b\r\nc,d\r\n')).toEqual([1, 2, 3]);
  });

  it('handles a leading BOM', () => {
    expect(linesOf('﻿id,en\na,b\n')).toEqual([1, 2]);
  });

  it('reports null rather than guess when a row does not line up', () => {
    // A hand-built rows array that does not describe the text: the check has to
    // notice, because a confidently wrong line number points at an innocent row.
    expect(rowLineNumbers('id,en\na,b\n', [['id', 'en'], ['zz', 'b']])).toEqual([1, null]);
  });

  it('resolves every row of the real strings.csv', async () => {
    const text = await readFile(path.join(root, 'assets', 'strings', 'strings.csv'), 'utf8');
    const rows = parseCsv(text);
    const lines = rowLineNumbers(text, rows);

    expect(lines).not.toContain(null);

    // Each row's id really is the first thing on the line we named.
    const physical = text.split('\n');
    rows.forEach((row, i) => {
      expect(physical[lines[i] - 1].startsWith(row[0])).toBe(true);
    });

    // The drift USED to be real (embedded-newline rows made the file longer
    // than its row count) until the strings consolidation (9b89a37b) removed
    // the last multi-line row; equality is legitimate now, and the synthetic
    // cases above own the multi-line resolution coverage.
    expect(lines[lines.length - 1]).toBeGreaterThanOrEqual(rows.length);
  });
});
