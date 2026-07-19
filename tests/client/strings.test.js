import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import {
  parseCsv, buildTable, setTable, getTable, t, has, applyToDom, localiseTree,
} from '../../gui/strings.js';

describe('parseCsv', () => {
  it('parses a plain table', () => {
    expect(parseCsv('id,context,en\na,b,c\n')).toEqual([
      ['id', 'context', 'en'],
      ['a', 'b', 'c'],
    ]);
  });

  it('keeps commas inside quoted fields', () => {
    const rows = parseCsv('id,context,en\nx,"Lobby, pre-connection","[Ready, Captain]"\n');
    expect(rows[1]).toEqual(['x', 'Lobby, pre-connection', '[Ready, Captain]']);
  });

  it('keeps newlines inside quoted fields', () => {
    // The comms dialogue in assets/worlds/*.toml is genuinely multi-line.
    const rows = parseCsv('id,en\nhail,"Line one\nLine two"\n');
    expect(rows[1]).toEqual(['hail', 'Line one\nLine two']);
  });

  it('unescapes doubled quotes', () => {
    const rows = parseCsv('id,en\nq,"He said ""go"" twice"\n');
    expect(rows[1][1]).toBe('He said "go" twice');
  });

  it('handles CRLF line endings', () => {
    expect(parseCsv('id,en\r\na,b\r\n')).toEqual([['id', 'en'], ['a', 'b']]);
  });

  it('handles a final row with no trailing newline', () => {
    expect(parseCsv('id,en\na,b')).toEqual([['id', 'en'], ['a', 'b']]);
  });

  it('drops the empty row a trailing newline would produce', () => {
    expect(parseCsv('id,en\na,b\n')).toHaveLength(2);
  });

  it('strips a UTF-8 BOM', () => {
    expect(parseCsv('﻿id,en\na,b\n')[0][0]).toBe('id');
  });

  it('preserves empty fields', () => {
    expect(parseCsv('id,context,en\na,,c\n')[1]).toEqual(['a', '', 'c']);
  });

  it('returns no rows for empty input', () => {
    expect(parseCsv('')).toEqual([]);
  });
});

describe('buildTable', () => {
  it('maps ids to the en column', () => {
    const table = buildTable('id,context,en\na.b,ctx,[Hello]\n');
    expect(table.get('a.b')).toBe('[Hello]');
  });

  it('reads a requested locale column', () => {
    const table = buildTable('id,context,en,fr\na.b,ctx,[Hello],Bonjour\n', 'fr');
    expect(table.get('a.b')).toBe('Bonjour');
  });

  it('falls back to en when the locale column is absent', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const table = buildTable('id,context,en\na.b,ctx,[Hello]\n', 'de');
    expect(table.get('a.b')).toBe('[Hello]');
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it('ignores rows with a blank id', () => {
    const table = buildTable('id,context,en\n,ctx,[Orphan]\na.b,ctx,[Real]\n');
    expect(table.size).toBe(1);
    expect(table.get('a.b')).toBe('[Real]');
  });

  it('warns on a duplicate id and keeps the later row', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const table = buildTable('id,en\na,[First]\na,[Second]\n');
    expect(table.get('a')).toBe('[Second]');
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("duplicate id 'a'"));
    warn.mockRestore();
  });

  it('throws when the id column is missing', () => {
    expect(() => buildTable('key,en\na,b\n')).toThrow(/missing required 'id'/);
  });

  it('throws when there is no en column to fall back to', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(() => buildTable('id,fr\na,b\n')).toThrow(/missing required 'en'/);
    warn.mockRestore();
  });

  it('returns an empty table for empty input', () => {
    expect(buildTable('').size).toBe(0);
  });
});

describe('t', () => {
  beforeEach(() => {
    setTable(new Map([
      ['plain', '[Red Alert]'],
      ['one_param', '[{n} CONTACTS]'],
      ['two_params', '[HEADING {deg} — HULL {pct}%]'],
      ['repeated', '[{x} and {x}]'],
    ]));
  });

  it('returns the text for a known id', () => {
    expect(t('plain')).toBe('[Red Alert]');
  });

  it('substitutes a placeholder', () => {
    expect(t('one_param', { n: 3 })).toBe('[3 CONTACTS]');
  });

  it('substitutes multiple placeholders', () => {
    expect(t('two_params', { deg: '045', pct: 82 })).toBe('[HEADING 045 — HULL 82%]');
  });

  it('substitutes every occurrence of a repeated placeholder', () => {
    expect(t('repeated', { x: 'A' })).toBe('[A and A]');
  });

  it('coerces numbers to strings', () => {
    expect(t('one_param', { n: 0 })).toBe('[0 CONTACTS]');
  });

  it('leaves unsupplied placeholders intact rather than blanking them', () => {
    expect(t('one_param', {})).toBe('[{n} CONTACTS]');
  });

  it('ignores params for a string with no placeholders', () => {
    expect(t('plain', { n: 1 })).toBe('[Red Alert]');
  });

  it('renders a missing id as angle-bracketed and warns', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(t('nope.missing')).toBe('⟨nope.missing⟩');
    expect(warn).toHaveBeenCalledWith("strings: no entry for 'nope.missing'");
    warn.mockRestore();
  });

  it('warns only once per missing id', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    t('nope.missing');
    t('nope.missing');
    t('nope.missing');
    expect(warn).toHaveBeenCalledTimes(1);
    warn.mockRestore();
  });
});

describe('has', () => {
  it('reports presence', () => {
    setTable(new Map([['a', '[A]']]));
    expect(has('a')).toBe(true);
    expect(has('b')).toBe(false);
  });
});

describe('setTable / getTable', () => {
  it('replaces the live table', () => {
    setTable(new Map([['a', '[A]']]));
    expect(getTable().get('a')).toBe('[A]');
    setTable(new Map([['b', '[B]']]));
    expect(getTable().has('a')).toBe(false);
  });

  it('clears the missing-id warning memo so a reload re-warns', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    setTable(new Map());
    t('gone');
    setTable(new Map());
    t('gone');
    expect(warn).toHaveBeenCalledTimes(2);
    warn.mockRestore();
  });
});

describe('localiseTree', () => {
  beforeEach(() => {
    setTable(new Map([
      ['entity.cruiser.name', '[Alliance Cruiser]'],
      ['world.entity.axiom_station.name', '[Axiom Station]'],
      ['world.btf.comms.hail.message', '[We have a situation.]'],
    ]));
  });

  it('resolves a bare string id', () => {
    expect(localiseTree('entity.cruiser.name')).toBe('[Alliance Cruiser]');
  });

  it('leaves strings that are not ids alone', () => {
    // uuids, system ids and tokens travel in the same payloads.
    expect(localiseTree('helm-engine-port')).toBe('helm-engine-port');
    expect(localiseTree('aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa'))
      .toBe('aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa');
  });

  it('resolves ids nested in objects', () => {
    const msg = { CommsState: { messages: [{ from: 'world.entity.axiom_station.name', body: 'world.btf.comms.hail.message' }] } };
    expect(localiseTree(msg)).toEqual({
      CommsState: { messages: [{ from: '[Axiom Station]', body: '[We have a situation.]' }] },
    });
  });

  it('resolves ids inside arrays', () => {
    expect(localiseTree(['entity.cruiser.name', 'other']))
      .toEqual(['[Alliance Cruiser]', 'other']);
  });

  it('preserves non-string scalars', () => {
    const msg = { hp: 42, alive: true, target: null, pct: 0.5 };
    expect(localiseTree(msg)).toEqual(msg);
  });

  it('does not mutate the input', () => {
    const msg = { from: 'entity.cruiser.name' };
    localiseTree(msg);
    expect(msg.from).toBe('entity.cruiser.name');
  });

  it('passes everything through untouched when the table is empty', () => {
    setTable(new Map());
    const msg = { from: 'entity.cruiser.name', hp: 1 };
    expect(localiseTree(msg)).toEqual(msg);
  });
});

describe('applyToDom', () => {
  // Minimal stand-in for the DOM: vitest runs in the `node` environment, and
  // applyToDom only ever touches querySelectorAll/textContent/setAttribute.
  function fakeRoot(elements) {
    return {
      querySelectorAll(selector) {
        const attr = selector.slice(1, -1); // '[data-i18n]' -> 'data-i18n'
        return elements.filter((el) => Object.prototype.hasOwnProperty.call(el.attrs, attr));
      },
    };
  }

  function el(attrs) {
    return {
      attrs,
      textContent: '',
      set: {},
      getAttribute(name) { return this.attrs[name]; },
      setAttribute(name, value) { this.set[name] = value; },
    };
  }

  beforeEach(() => {
    setTable(new Map([
      ['title.id', '[SENSORS]'],
      ['tip.id', '[Toggle the debug panel]'],
      ['label.id', '[Settings]'],
    ]));
  });

  it('sets textContent from data-i18n', () => {
    const node = el({ 'data-i18n': 'title.id' });
    applyToDom(fakeRoot([node]));
    expect(node.textContent).toBe('[SENSORS]');
  });

  it('sets attributes from data-i18n-attr', () => {
    const node = el({ 'data-i18n-attr': 'title:tip.id' });
    applyToDom(fakeRoot([node]));
    expect(node.set.title).toBe('[Toggle the debug panel]');
  });

  it('handles several attribute pairs', () => {
    const node = el({ 'data-i18n-attr': 'title:tip.id,aria-label:label.id' });
    applyToDom(fakeRoot([node]));
    expect(node.set.title).toBe('[Toggle the debug panel]');
    expect(node.set['aria-label']).toBe('[Settings]');
  });

  it('tolerates whitespace around pairs', () => {
    const node = el({ 'data-i18n-attr': ' title : tip.id , aria-label : label.id ' });
    applyToDom(fakeRoot([node]));
    expect(node.set.title).toBe('[Toggle the debug panel]');
    expect(node.set['aria-label']).toBe('[Settings]');
  });

  it('skips malformed pairs without throwing', () => {
    const node = el({ 'data-i18n-attr': 'no-colon-here,title:tip.id' });
    expect(() => applyToDom(fakeRoot([node]))).not.toThrow();
    expect(node.set.title).toBe('[Toggle the debug panel]');
  });

  it('does nothing when given a root without querySelectorAll', () => {
    expect(() => applyToDom({})).not.toThrow();
  });
});
