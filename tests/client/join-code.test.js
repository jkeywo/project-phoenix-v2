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
  SURFACE_CLIENT,
  SURFACE_SERVER,
} from '../../gui/join-code.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const DATA = JSON.parse(
  readFileSync(path.join(root, 'assets/join/join-codes.json'), 'utf8'),
);

// Installed at module scope, not in beforeAll: the describe bodies below read
// GUIDs out of the table while vitest is still collecting.
setJoinCodeData(DATA);

describe('authored format data', () => {
  it('mints eight letters from an alphabet that excludes every normalised-away letter', () => {
    // EIGHT, and the number is the scheme's whole defence rather than a taste:
    // the code is the only secret in front of a game (the join stamp is
    // public), and five letters over this 25-letter alphabet is 25^5 ≈ 9.77e6
    // — 23.2 bits — which a host answering ~2,340 wrong guesses a second walks
    // in about 35 minutes. 25^8 ≈ 1.526e11 (37.15 bits) is ~377 days at that
    // same rate, before any attempt-limiting. See the [suffix] note in
    // assets/join/join-codes.toml.
    expect(DATA.suffix.length).toBe(8);
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

  it('authors every denied word short enough to appear inside a minted suffix', () => {
    // A word is matched ANYWHERE inside a canonicalised suffix, so an entry
    // longer than a whole suffix could never match and would be a row that
    // silently does nothing.
    for (const entry of DATA.deny) {
      expect(entry.reason, `deny entry ${entry.word} needs a reason`).toBeTruthy();
      const canonical = canonicaliseSuffix(entry.word);
      expect(canonical.length, entry.word).toBeGreaterThan(0);
      expect(canonical.length, entry.word).toBeLessThanOrEqual(DATA.suffix.length);
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
  it('accepts eight canonical letters', () => {
    expect(validateSuffix('quarking')).toEqual({ ok: true, suffix: 'QUARKING' });
  });

  it('accepts input that only becomes valid after normalisation', () => {
    expect(validateSuffix('b01ldest')).toEqual({ ok: true, suffix: 'BOIIDEST' });
  });

  it('separates empty, wrong-length and out-of-alphabet input', () => {
    expect(validateSuffix('')).toMatchObject({ ok: false, reason: 'empty' });
    expect(validateSuffix('QUARKIN')).toMatchObject({ ok: false, reason: 'length' });
    expect(validateSuffix('QUARKINGS')).toMatchObject({ ok: false, reason: 'length' });
    expect(validateSuffix('QU4RKING')).toMatchObject({ ok: false, reason: 'charset' });
  });

  it('refuses the authored deny-list wherever it appears inside a suffix', () => {
    const denied = [...deniedSuffixes()][0];
    const padded = (word) => (word + 'XYZWVUTS').slice(0, DATA.suffix.length);
    expect(validateSuffix(padded(denied))).toMatchObject({ ok: false, reason: 'denied' });
    expect(validateSuffix(padded(denied).toLowerCase()))
      .toMatchObject({ ok: false, reason: 'denied' });
    // A word buried in the MIDDLE is the case exact matching used to miss, and
    // the reason the rule is containment now the suffix is longer than the
    // list's words: a code is refused for READING as one of them.
    expect(isDenied('XADMINYZ')).toBe(true);
    expect(isDenied('adminxyz')).toBe(true);
    expect(isDenied('xyzwadmin')).toBe(true);
    // Authored with an L, stored canonically with an I: both spellings refuse.
    expect(isDenied('LOGINXYZ')).toBe(true);
    expect(isDenied('IOGINXYZ')).toBe(true);
  });

  it('does not refuse an ordinary code', () => {
    expect(isDenied('QUARKING')).toBe(false);
  });
});

describe('full join identifiers', () => {
  const client = projectGuidFor(NAMESPACE_CLIENT);
  const server = projectGuidFor(NAMESPACE_SERVER);

  it('composes PROJECT_VERSION_SUFFIX from local context when only a suffix is typed', () => {
    const built = joinCodeForSuffix('quarking', NAMESPACE_CLIENT);
    expect(built).toMatchObject({ ok: true, suffix: 'QUARKING', namespace: NAMESPACE_CLIENT });
    expect(built.full).toBe(`${client}_${versionGuid()}_QUARKING`);
  });

  it('round-trips a composed code back to its three parts', () => {
    const full = composeJoinCode({ project: server, version: versionGuid(), suffix: 'QUARKING' });
    const parsed = parseJoinCode(full, NAMESPACE_CLIENT);
    expect(parsed).toMatchObject({
      ok: true,
      typed: 'full',
      project: server,
      suffix: 'QUARKING',
      namespace: NAMESPACE_SERVER,
    });
  });

  it('reads a bare suffix as the page-supplied namespace', () => {
    expect(parseJoinCode('quarking', NAMESPACE_CLIENT)).toMatchObject({
      ok: true,
      typed: 'suffix',
      namespace: NAMESPACE_CLIENT,
      project: client,
    });
  });

  it('reads a QR link by taking the code out of the URL fragment', () => {
    const full = `${client}_${versionGuid()}_QUARKING`;
    const parsed = parseJoinCode(`https://example.test/client/index.html#${full}`, NAMESPACE_CLIENT);
    expect(parsed).toMatchObject({ ok: true, typed: 'full', suffix: 'QUARKING', project: client });
  });

  it('reaches the identical identifier from typed suffix, pasted code and QR link', () => {
    const typed = parseJoinCode('quarking', NAMESPACE_CLIENT);
    const pasted = parseJoinCode(`${client}_${versionGuid()}_quarking`, NAMESPACE_CLIENT);
    const scanned = parseJoinCode(`https://x.test/client/#${client}_${versionGuid()}_QUARKING`, NAMESPACE_CLIENT);
    expect(pasted.full).toBe(typed.full);
    expect(scanned.full).toBe(typed.full);
  });

  it('names an unrecognised project GUID rather than retyping it', () => {
    const parsed = parseJoinCode(`not-a-known-project_${versionGuid()}_QUARKING`, NAMESPACE_CLIENT);
    expect(parsed).toMatchObject({ ok: false, reason: 'unknown-project' });
  });

  it('rejects a code with the wrong number of parts', () => {
    expect(parseJoinCode(`${client}_QUARKING`, NAMESPACE_CLIENT)).toMatchObject({
      ok: false,
      reason: 'malformed',
    });
  });

  it('reads a suffix a player punctuated, even with the part separator in it', () => {
    // `_` is BOTH the separator inside a full code and an authored strip
    // character, because a guest reading a code aloud writes it apart.
    // Splitting before deciding the shape reported this as "not readable",
    // which is a lie about input the scheme accepts.
    for (const typed of [
      'QU_ARKING', 'qu_arking', 'Q_U_A_R_K_I_N_G', ' q-u a_r k-i n_g ', 'QUAR KING',
    ]) {
      expect(parseJoinCode(typed, NAMESPACE_CLIENT), typed).toMatchObject({
        ok: true,
        typed: 'suffix',
        suffix: 'QUARKING',
      });
    }
  });

  it('keeps a punctuated suffix failure specific rather than calling it malformed', () => {
    expect(parseJoinCode('AD_MINXYZ', NAMESPACE_CLIENT)).toMatchObject({ reason: 'denied' });
    expect(parseJoinCode('QU-ARKIN', NAMESPACE_CLIENT)).toMatchObject({ reason: 'length' });
  });

  it('reads a full code that was retyped with spaces around the separators', () => {
    const full = `${client} _ ${versionGuid()} _ QUARKING`;
    expect(parseJoinCode(full, NAMESPACE_CLIENT)).toMatchObject({
      ok: true,
      typed: 'full',
      project: client,
      suffix: 'QUARKING',
    });
  });

  it('refuses a denied suffix even inside a well-formed full code', () => {
    expect(parseJoinCode(`${client}_${versionGuid()}_ADMINXYZ`, NAMESPACE_CLIENT)).toMatchObject({
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
    const script = [...draws('QUARKING'), ...draws('MOISTURE')];
    let i = 0;
    const taken = new Set(['QUARKING']);
    const minted = mintSuffix(DATA, (s) => taken.has(s), () => script[i++]);
    expect(minted).toEqual({ ok: true, suffix: 'MOISTURE' });
    expect(i, 'the collision was never drawn, so the retry never ran')
      .toBe(script.length);
  });

  it('never mints a suffix that reads as a denied word', () => {
    // Force the first whole draw onto a suffix CONTAINING 'ADMIN', then let it
    // fall through to the next one — which is the branch that matters now the
    // rule is containment rather than equality.
    const a = DATA.suffix.alphabet;
    const admin = [...'ADMINXYZ'].map((c) => a.indexOf(c));
    const quark = [...'QUARKING'].map((c) => a.indexOf(c));
    const minted = mintSuffix(DATA, () => false, seq([...admin, ...quark]));
    expect(minted).toEqual({ ok: true, suffix: 'QUARKING' });
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
   * the only way JavaScript can notice a new variant is to be told the list.
   * Below, this list is checked for SET EQUALITY against the string literals
   * extracted straight out of `fn code()`'s match arms — not just "these five
   * are present" — so a sixth `StampMismatch` variant with no row here fails
   * loudly instead of quietly rendering as `error_unknown` on the phone. If
   * you add or rename a variant in StampMismatch, this test is where you find
   * out.
   */
  const STAMP_MISMATCH_CODES = [
    'client-stamp-missing',
    'bundle-content-missing',
    'protocol-mismatch',
    'content-id-mismatch',
    'content-epoch-mismatch',
  ];

  /**
   * Pull the string literals straight out of `StampMismatch::code()`'s match
   * arms in src/delivery/stamp.rs — the arms are a clean
   * `Variant { .. } => "literal",` block, so scanning for the balanced brace
   * that closes the function and regexing the quoted literals inside it is
   * enough, with no Rust parser involved.
   */
  function stampMismatchCodesFromRust() {
    const src = readFileSync(path.join(root, 'src/delivery/stamp.rs'), 'utf8');
    const marker = 'fn code(';
    const fnStart = src.indexOf(marker);
    if (fnStart === -1) return [];
    const braceStart = src.indexOf('{', fnStart);
    if (braceStart === -1) return [];
    let depth = 0;
    let braceEnd = -1;
    for (let i = braceStart; i < src.length; i += 1) {
      if (src[i] === '{') depth += 1;
      else if (src[i] === '}') {
        depth -= 1;
        if (depth === 0) {
          braceEnd = i;
          break;
        }
      }
    }
    if (braceEnd === -1) return [];
    const body = src.slice(braceStart, braceEnd + 1);
    return [...body.matchAll(/"([a-z0-9-]+)"/g)].map((m) => m[1]);
  }

  it('gives unknown, wrong-type and version-mismatch three different strings', () => {
    const ids = ['unknown', 'wrong-type', 'version-mismatch'].map(reasonStringId);
    expect(new Set(ids).size).toBe(3);
  });

  it('falls back to the unknown string for a reason it has no row for', () => {
    expect(reasonStringId('something-new')).toBe(reasonStringId('unknown'));
  });

  it('maps every host refusal code the compatibility handshake can return', () => {
    const rustCodes = stampMismatchCodesFromRust();
    // A silent zero-match extraction would sail through a set-equality check
    // against an empty expectation just as quietly as the old containment
    // check did against a new variant — fail loudly instead, so a Rust
    // reformatting that breaks the regex is caught here rather than by a
    // shipped "no ship is using that code" for every refusal.
    expect(
      rustCodes.length,
      'extracted zero string literals from fn code() in src/delivery/stamp.rs — the regex or the fn code( marker is stale',
    ).toBeGreaterThan(0);
    // Set equality, not mere containment: this catches BOTH a new Rust
    // variant with no row here (the bug this test exists to prevent) AND a
    // stale JS entry for a code Rust no longer emits.
    expect(new Set(rustCodes)).toEqual(new Set(STAMP_MISMATCH_CODES));

    for (const code of STAMP_MISMATCH_CODES) {
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

  it('maps both native identity refusals to authored recovery messages', () => {
    const source = readFileSync(path.join(root, 'src/native_host/relay_transport.rs'), 'utf8');
    const codes = [...source.matchAll(/pub const (?:RESERVED|INVALID)_TOKEN_CODE: &str = "([a-z-]+)"/g)]
      .map((match) => match[1]);
    expect(new Set(codes)).toEqual(new Set(['reserved-token', 'invalid-token']));
    for (const code of codes) {
      expect(reasonStringId(code)).not.toBe(reasonStringId('unknown'));
    }
    expect(reasonStringId('invalid-token')).toBe('client.join.error_invalid_token');
  });

  it('does not call a foreign GUID a fleet code', () => {
    // unknown-project/unknown-namespace mean "this belongs to no Phoenix
    // namespace", which is a different statement from "that is the other typed
    // namespace" — and the wrong-type wording is a false claim about it.
    expect(reasonStringId('unknown-project')).not.toBe(reasonStringId('wrong-type'));
    expect(reasonStringId('unknown-namespace')).toBe(reasonStringId('unknown-project'));
  });

  it('maps every refusal reason the rendezvous service can actually emit', () => {
    // Driven from the SERVICE's source, not from this map's own keys. A
    // coverage test that iterates knownReasons() can only agree with itself,
    // and that is precisely how #1113's four relay refusals (relay-full,
    // relay-too-large, not-relaying, no-peer) shipped rendering as "No ship is
    // using that code" — the least actionable sentence in the game, and a lie
    // about a correct code with a live host behind it.
    const registry = readFileSync(
      path.join(root, 'worker-rendezvous/src/registry.js'),
      'utf8',
    );
    const relay = readFileSync(path.join(root, 'worker-rendezvous/src/relay.js'), 'utf8');
    const emitted = new Set([
      // `fail(connId, request, 'reason')` / `cut(...)` — the error frames.
      ...[...registry.matchAll(/(?:fail|cut)\([^)]*?'([a-z-]+)'\s*\)/g)].map((m) => m[1]),
      // …plus the reasons relay.js hands back for the registry to relay on, and
      // the ones it puts on a `relay-closed` frame.
      ...[...relay.matchAll(/reason:\s*'([a-z-]+)'/g)].map((m) => m[1]),
      ...[...registry.matchAll(/reason:\s*'([a-z-]+)'/g)].map((m) => m[1]),
    ]);
    // A regex that matched nothing would pass this vacuously, which is the
    // exact failure the stamp-code test above already had to close.
    expect(
      emitted.size,
      'extracted no refusal reasons from worker-rendezvous/src — the regexes are stale',
    ).toBeGreaterThan(8);

    const mapped = new Set(knownReasons());
    const unmapped = [...emitted].filter((r) => !mapped.has(r));
    expect(
      unmapped,
      `the service emits these with no row in REASON_STRING_IDS: ${unmapped.join(', ')}`,
    ).toEqual([]);
  });

  it('has an authored strings.csv row for every reason it can display', () => {
    const table = buildTable(readFileSync(path.join(root, 'assets/strings/strings.csv'), 'utf8'));
    for (const reason of knownReasons()) {
      const id = reasonStringId(reason);
      expect(table.get(id), `${reason} maps to ${id}, which has no strings.csv row`).toBeTruthy();
    }
  });

  it('gives a fleet operator every StampMismatch code in a host-page sentence', () => {
    // A ship host refused on its BUILD reads the answer through
    // `server.fleet.error_joining`, so every one of these codes has to have a
    // server-surface row too — otherwise a fleet refusal quietly borrows a
    // sentence written for a phone ("Reload this page from the ship's own
    // address") on a screen where it means nothing.
    const table = buildTable(readFileSync(path.join(root, 'assets/strings/strings.csv'), 'utf8'));
    for (const code of STAMP_MISMATCH_CODES) {
      const id = reasonStringId(code, SURFACE_SERVER);
      expect(id, code).toMatch(/^server\.fleet\./);
      expect(table.get(id), `${code} maps to ${id}, which has no strings.csv row`).toBeTruthy();
    }
  });

  it('gives a full relay its own sentence rather than the code-is-wrong one', () => {
    // The one degraded state a guest actually meets: a correct code, a live
    // host, and a relay at its authored ceiling. The remedy is somebody
    // disconnecting or another network, and neither is discoverable from
    // "check the viewscreen and try again".
    expect(reasonStringId('relay-full')).not.toBe(reasonStringId('unknown'));
    expect(reasonStringId('relay-full')).not.toBe(reasonStringId('admission-closed'));
    expect(reasonStringId('relay-too-large')).not.toBe(reasonStringId('unknown'));
    expect(reasonStringId('not-relaying')).not.toBe(reasonStringId('unknown'));
    expect(reasonStringId('no-peer')).not.toBe(reasonStringId('unknown'));
  });

  it('defaults to the phone\'s wording, so an unqualified lookup is unchanged', () => {
    for (const reason of knownReasons()) {
      expect(reasonStringId(reason), reason).toBe(reasonStringId(reason, SURFACE_CLIENT));
    }
  });
});
