// Pure tests for the typed join-identifier scheme (issue #1111).
//
// Everything here goes through gui/join-code.js's public functions against the
// real authored table in assets/join/join-codes.json — no fixture copy of the
// format, because a fixture would let the shipped data drift from the rules
// these tests claim to pin.

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { buildTable } from '../../gui/strings.js';
import {
  JOIN_CODE_FORMAT_VERSION,
  NAMESPACE_CLIENT,
  NAMESPACE_SERVER,
  checkJoinCodeFormat,
  setJoinCodeData,
  canonicaliseSuffix,
  validateSuffix,
  deniedSuffixes,
  isDenied,
  projectGuidFor,
  namespaceOf,
  versionGuid,
  composeJoinCode,
  joinCodeForSuffix,
  parseJoinCode,
  mintSuffix,
  reasonStringId,
  knownReasons,
} from '../../gui/join-code.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const DATA = JSON.parse(
  readFileSync(path.join(root, 'assets/join/join-codes.json'), 'utf8'),
);

// Installed at module scope, not in beforeAll: the describe bodies below read
// GUIDs out of the table while vitest is still collecting.
setJoinCodeData(DATA);

describe('authored format data', () => {
  it('mints five letters from an alphabet that excludes every normalised-away letter', () => {
    expect(DATA.suffix.length).toBe(5);
    for (const from of Object.keys(DATA.suffix.normalise)) {
      expect(DATA.suffix.alphabet).not.toContain(from);
    }
  });

  it('keeps J distinct — it is not folded into anything', () => {
    expect(DATA.suffix.normalise).not.toHaveProperty('J');
    expect(DATA.suffix.alphabet).toContain('J');
    expect(canonicaliseSuffix('J')).toBe('J');
  });

  it('gives the client and server namespaces different stable project GUIDs', () => {
    expect(projectGuidFor(NAMESPACE_CLIENT)).toBeTruthy();
    expect(projectGuidFor(NAMESPACE_SERVER)).toBeTruthy();
    expect(projectGuidFor(NAMESPACE_CLIENT)).not.toBe(projectGuidFor(NAMESPACE_SERVER));
  });

  it('carries exactly one compatible-release version GUID', () => {
    expect(versionGuid()).toBeTruthy();
    expect(typeof versionGuid()).toBe('string');
  });

  it('authors every denied word so it canonicalises to a mintable-length suffix', () => {
    for (const entry of DATA.deny) {
      expect(entry.reason, `deny entry ${entry.word} needs a reason`).toBeTruthy();
      expect(canonicaliseSuffix(entry.word), entry.word).toHaveLength(DATA.suffix.length);
    }
  });

  it('refuses a table from a format revision this build does not implement', () => {
    // The constant is only worth having if something compares against it: an
    // unchecked version field is a comment wearing a keyword.
    expect(DATA.format_version).toBe(JOIN_CODE_FORMAT_VERSION);
    expect(() => checkJoinCodeFormat({ ...DATA, format_version: 99 })).toThrow(/format_version/);
    expect(() => checkJoinCodeFormat({ suffix: DATA.suffix })).toThrow(/format_version/);
    // …and installing one is the same refusal, so a page cannot half-read it.
    expect(() => setJoinCodeData({ ...DATA, format_version: 2 })).toThrow();
    setJoinCodeData(DATA);
  });
});

describe('canonicalisation', () => {
  it('is case-insensitive', () => {
    expect(canonicaliseSuffix('quark')).toBe('QUARK');
    expect(canonicaliseSuffix('QuArK')).toBe('QUARK');
  });

  it('normalises 0 to O and both 1 and L to I', () => {
    expect(canonicaliseSuffix('0')).toBe('O');
    expect(canonicaliseSuffix('1')).toBe('I');
    expect(canonicaliseSuffix('l')).toBe('I');
    expect(canonicaliseSuffix('L')).toBe('I');
    expect(canonicaliseSuffix('b01ld')).toBe('BOIID');
  });

  it('ignores the spacing a player adds while reading a code aloud', () => {
    expect(canonicaliseSuffix(' q-u a_r k ')).toBe('QUARK');
  });

  it('folds every confusable spelling of one code onto one suffix', () => {
    const spellings = ['MOIST', 'm0ist', 'MO1ST', 'mO-lST'];
    const canonical = spellings.map((s) => canonicaliseSuffix(s));
    expect(new Set(canonical).size).toBe(1);
  });
});

describe('suffix validation', () => {
  it('accepts five canonical letters', () => {
    expect(validateSuffix('quark')).toEqual({ ok: true, suffix: 'QUARK' });
  });

  it('accepts input that only becomes valid after normalisation', () => {
    expect(validateSuffix('b01ld')).toEqual({ ok: true, suffix: 'BOIID' });
  });

  it('separates empty, wrong-length and out-of-alphabet input', () => {
    expect(validateSuffix('')).toMatchObject({ ok: false, reason: 'empty' });
    expect(validateSuffix('QUAR')).toMatchObject({ ok: false, reason: 'length' });
    expect(validateSuffix('QUARKS')).toMatchObject({ ok: false, reason: 'length' });
    expect(validateSuffix('QU4RK')).toMatchObject({ ok: false, reason: 'charset' });
  });

  it('refuses the authored deny-list, in every spelling that reaches it', () => {
    const denied = [...deniedSuffixes()][0];
    expect(validateSuffix(denied)).toMatchObject({ ok: false, reason: 'denied' });
    expect(validateSuffix(denied.toLowerCase())).toMatchObject({ ok: false, reason: 'denied' });
    expect(isDenied('admin')).toBe(true);
    // Authored with an L, stored canonically with an I: both spellings refuse.
    expect(isDenied('LOGIN')).toBe(true);
    expect(isDenied('IOGIN')).toBe(true);
  });

  it('does not refuse an ordinary code', () => {
    expect(isDenied('QUARK')).toBe(false);
  });
});

describe('full join identifiers', () => {
  const client = projectGuidFor(NAMESPACE_CLIENT);
  const server = projectGuidFor(NAMESPACE_SERVER);

  it('composes PROJECT_VERSION_SUFFIX from local context when only a suffix is typed', () => {
    const built = joinCodeForSuffix('quark', NAMESPACE_CLIENT);
    expect(built).toMatchObject({ ok: true, suffix: 'QUARK', namespace: NAMESPACE_CLIENT });
    expect(built.full).toBe(`${client}_${versionGuid()}_QUARK`);
  });

  it('round-trips a composed code back to its three parts', () => {
    const full = composeJoinCode({ project: server, version: versionGuid(), suffix: 'QUARK' });
    const parsed = parseJoinCode(full, NAMESPACE_CLIENT);
    expect(parsed).toMatchObject({
      ok: true,
      typed: 'full',
      project: server,
      suffix: 'QUARK',
      namespace: NAMESPACE_SERVER,
    });
  });

  it('reads a bare suffix as the page-supplied namespace', () => {
    expect(parseJoinCode('quark', NAMESPACE_CLIENT)).toMatchObject({
      ok: true,
      typed: 'suffix',
      namespace: NAMESPACE_CLIENT,
      project: client,
    });
  });

  it('reads a QR link by taking the code out of the URL fragment', () => {
    const full = `${client}_${versionGuid()}_QUARK`;
    const parsed = parseJoinCode(`https://example.test/client/index.html#${full}`, NAMESPACE_CLIENT);
    expect(parsed).toMatchObject({ ok: true, typed: 'full', suffix: 'QUARK', project: client });
  });

  it('reaches the identical identifier from typed suffix, pasted code and QR link', () => {
    const typed = parseJoinCode('quark', NAMESPACE_CLIENT);
    const pasted = parseJoinCode(`${client}_${versionGuid()}_quark`, NAMESPACE_CLIENT);
    const scanned = parseJoinCode(`https://x.test/client/#${client}_${versionGuid()}_QUARK`, NAMESPACE_CLIENT);
    expect(pasted.full).toBe(typed.full);
    expect(scanned.full).toBe(typed.full);
  });

  it('names an unrecognised project GUID rather than retyping it', () => {
    const parsed = parseJoinCode(`not-a-known-project_${versionGuid()}_QUARK`, NAMESPACE_CLIENT);
    expect(parsed).toMatchObject({ ok: false, reason: 'unknown-project' });
  });

  it('rejects a code with the wrong number of parts', () => {
    expect(parseJoinCode(`${client}_QUARK`, NAMESPACE_CLIENT)).toMatchObject({
      ok: false,
      reason: 'malformed',
    });
  });

  it('reads five letters a player punctuated, even with the part separator in them', () => {
    // `_` is BOTH the separator inside a full code and an authored strip
    // character, because a guest reading five letters aloud writes them
    // apart. Splitting before deciding the shape reported this as "not
    // readable", which is a lie about input the scheme accepts.
    for (const typed of ['QU_ARK', 'qu_ark', 'Q_U_A_R_K', ' q-u a_r k ', 'QU ARK']) {
      expect(parseJoinCode(typed, NAMESPACE_CLIENT), typed).toMatchObject({
        ok: true,
        typed: 'suffix',
        suffix: 'QUARK',
      });
    }
  });

  it('keeps a punctuated suffix failure specific rather than calling it malformed', () => {
    expect(parseJoinCode('AD_MIN', NAMESPACE_CLIENT)).toMatchObject({ reason: 'denied' });
    expect(parseJoinCode('QU-AR', NAMESPACE_CLIENT)).toMatchObject({ reason: 'length' });
  });

  it('reads a full code that was retyped with spaces around the separators', () => {
    const full = `${client} _ ${versionGuid()} _ QUARK`;
    expect(parseJoinCode(full, NAMESPACE_CLIENT)).toMatchObject({
      ok: true,
      typed: 'full',
      project: client,
      suffix: 'QUARK',
    });
  });

  it('refuses a denied suffix even inside a well-formed full code', () => {
    expect(parseJoinCode(`${client}_${versionGuid()}_ADMIN`, NAMESPACE_CLIENT)).toMatchObject({
      ok: false,
      reason: 'denied',
    });
  });
});

describe('minting', () => {
  const seq = (values) => {
    let i = 0;
    return () => values[i++ % values.length];
  };

  it('draws only from the authored alphabet', () => {
    const minted = mintSuffix(DATA, () => false, (n) => Math.floor(Math.random() * n));
    expect(minted.ok).toBe(true);
    expect(minted.suffix).toHaveLength(DATA.suffix.length);
    for (const ch of minted.suffix) expect(DATA.suffix.alphabet).toContain(ch);
  });

  it('skips a collision in the namespace and draws again', () => {
    // Scripted so the FIRST draw genuinely lands on a taken suffix and the
    // retry branch executes. Two random draws almost never collide, so a test
    // written that way asserts nothing about the branch it is named for.
    const a = DATA.suffix.alphabet;
    const draws = (word) => [...word].map((c) => a.indexOf(c));
    const script = [...draws('QUARK'), ...draws('MOIST')];
    let i = 0;
    const taken = new Set(['QUARK']);
    const minted = mintSuffix(DATA, (s) => taken.has(s), () => script[i++]);
    expect(minted).toEqual({ ok: true, suffix: 'MOIST' });
    expect(i, 'the collision was never drawn, so the retry never ran')
      .toBe(script.length);
  });

  it('never mints a denied word', () => {
    // Force the draw onto 'ADMIN', then let it fall through to the next draw.
    const a = DATA.suffix.alphabet;
    const admin = [...'ADMIN'].map((c) => a.indexOf(c));
    const quark = [...'QUARK'].map((c) => a.indexOf(c));
    const minted = mintSuffix(DATA, () => false, seq([...admin, ...quark]));
    expect(minted).toEqual({ ok: true, suffix: 'QUARK' });
  });

  it('gives up with a reason rather than looping on a saturated namespace', () => {
    expect(mintSuffix(DATA, () => true, () => 0, 8)).toEqual({ ok: false, reason: 'exhausted' });
  });
});

describe('reason reporting', () => {
  /**
   * Every code `StampMismatch::code()` can emit — src/delivery/stamp.rs.
   *
   * Hardcoded on purpose: these five strings cross a language boundary
   * (Rust → encode_join_verdict → server.html → JoinRefused → this module), so
   * the only way JavaScript can notice a new variant is to be told the list and
   * fail when it stops matching. If you add or rename a variant in
   * StampMismatch, this test is where you find out that the phone would have
   * shown the player "no ship is using that code" for it.
   */
  const STAMP_MISMATCH_CODES = [
    'client-stamp-missing',
    'bundle-content-missing',
    'protocol-mismatch',
    'content-id-mismatch',
    'content-epoch-mismatch',
  ];

  it('gives unknown, wrong-type and version-mismatch three different strings', () => {
    const ids = ['unknown', 'wrong-type', 'version-mismatch'].map(reasonStringId);
    expect(new Set(ids).size).toBe(3);
  });

  it('falls back to the unknown string for a reason it has no row for', () => {
    expect(reasonStringId('something-new')).toBe(reasonStringId('unknown'));
  });

  it('maps every host refusal code the compatibility handshake can return', () => {
    const rustCodes = readFileSync(path.join(root, 'src/delivery/stamp.rs'), 'utf8');
    for (const code of STAMP_MISMATCH_CODES) {
      // The list above really is the list in the Rust file.
      expect(rustCodes, `StampMismatch::code() no longer emits ${code}`).toContain(`"${code}"`);
      // …and none of them renders as "no ship is using that code", which is
      // what an unmapped reason falls back to.
      expect(reasonStringId(code), code).not.toBe(reasonStringId('unknown'));
    }
  });

  it('separates a different build from different content from an unidentifiable phone', () => {
    expect(reasonStringId('protocol-mismatch')).toBe(reasonStringId('version-mismatch'));
    expect(reasonStringId('content-id-mismatch')).toBe(reasonStringId('content-epoch-mismatch'));
    expect(reasonStringId('content-id-mismatch')).not.toBe(reasonStringId('protocol-mismatch'));
    expect(reasonStringId('client-stamp-missing')).not.toBe(reasonStringId('content-id-mismatch'));
  });

  it('does not call a foreign GUID a fleet code', () => {
    // unknown-project/unknown-namespace mean "this belongs to no Phoenix
    // namespace", which is a different statement from "that is the other typed
    // namespace" — and the wrong-type wording is a false claim about it.
    expect(reasonStringId('unknown-project')).not.toBe(reasonStringId('wrong-type'));
    expect(reasonStringId('unknown-namespace')).toBe(reasonStringId('unknown-project'));
  });

  it('has an authored strings.csv row for every reason it can display', () => {
    const table = buildTable(readFileSync(path.join(root, 'assets/strings/strings.csv'), 'utf8'));
    for (const reason of knownReasons()) {
      const id = reasonStringId(reason);
      expect(table.get(id), `${reason} maps to ${id}, which has no strings.csv row`).toBeTruthy();
    }
  });
});
