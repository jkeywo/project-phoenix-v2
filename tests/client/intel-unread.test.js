/**
 * tests/client/intel-unread.test.js — the Intel tab's unread count
 * (gui/intel-unread.js, issue #1373).
 *
 * The badge answers one question: how many dossier SUBJECTS have gained
 * something since the seat last opened the panel. These drive the two pure
 * functions the way the console does — compute a count against a baseline,
 * take a new baseline when the panel is read, compute again.
 */
import { describe, it, expect } from 'vitest';
import {
  dossierEntryCount, intelUnreadCount, markIntelSeen,
} from '../../gui/intel-unread.js';

/** A dossier shaped like `DossierSnapshot` on the wire. */
function subject(uuid, facts, evidence) {
  return {
    uuid,
    name: `entity.${uuid}.name`,
    summary: '',
    facts: Array.from({ length: facts }, (_, i) => ({ text: `f${i}` })),
    evidence: Array.from({ length: evidence }, (_, i) => ({ text: `e${i}` })),
  };
}

describe('what is on file for one subject', () => {
  it('sums facts and evidence, the same total the dossier list row shows', () => {
    expect(dossierEntryCount(subject('a', 2, 3))).toBe(5);
  });

  it('reads a subject with neither array as empty rather than throwing', () => {
    expect(dossierEntryCount({ uuid: 'a' })).toBe(0);
    expect(dossierEntryCount(null)).toBe(0);
    expect(dossierEntryCount(undefined)).toBe(0);
  });
});

describe('how much has arrived since the seat last looked', () => {
  it('counts every subject with something on file when nothing has been read', () => {
    const subjects = [subject('a', 1, 0), subject('b', 0, 2), subject('c', 0, 0)];
    // 'c' is empty — the panel itself says "nothing on file" for it, so there
    // is nothing to announce.
    expect(intelUnreadCount(subjects, null)).toBe(2);
  });

  it('is zero straight after the seat reads the panel', () => {
    const subjects = [subject('a', 1, 1), subject('b', 3, 0)];
    const seen = markIntelSeen(subjects, null);
    expect(intelUnreadCount(subjects, seen)).toBe(0);
  });

  it('grows by one subject when a dossier gains a fact', () => {
    const before = [subject('a', 1, 0), subject('b', 2, 0)];
    const seen = markIntelSeen(before, null);
    const after = [subject('a', 2, 0), subject('b', 2, 0)];
    expect(intelUnreadCount(after, seen)).toBe(1);
  });

  it('grows by one subject when a dossier gains evidence', () => {
    const before = [subject('a', 1, 0)];
    const seen = markIntelSeen(before, null);
    expect(intelUnreadCount([subject('a', 1, 1)], seen)).toBe(1);
  });

  it('counts a subject once however many entries it gained', () => {
    const seen = markIntelSeen([subject('a', 1, 0)], null);
    expect(intelUnreadCount([subject('a', 5, 4)], seen)).toBe(1);
  });

  it('counts a brand-new subject that arrives with something on file', () => {
    const seen = markIntelSeen([subject('a', 1, 0)], null);
    expect(intelUnreadCount([subject('a', 1, 0), subject('b', 1, 0)], seen)).toBe(1);
  });

  it('does not count a subject that shrank — it answers "unseen", not "changed"', () => {
    const seen = markIntelSeen([subject('a', 4, 0)], null);
    expect(intelUnreadCount([subject('a', 2, 0)], seen)).toBe(0);
  });

  it('ignores entries with no uuid to key a baseline on', () => {
    expect(intelUnreadCount([{ facts: [{}], evidence: [] }, null], null)).toBe(0);
  });

  it('treats an absent subject list as nothing unread', () => {
    expect(intelUnreadCount(null, null)).toBe(0);
    expect(intelUnreadCount(undefined, {})).toBe(0);
  });
});

describe('taking the baseline', () => {
  it('returns a new object and leaves the old baseline alone', () => {
    const seen = markIntelSeen([subject('a', 1, 0)], null);
    const next = markIntelSeen([subject('a', 3, 0)], seen);
    expect(seen).toEqual({ a: 1 });
    expect(next).toEqual({ a: 3 });
  });

  it('keeps a subject the payload no longer carries, so it cannot re-announce', () => {
    const seen = markIntelSeen([subject('a', 2, 0), subject('b', 1, 0)], null);
    const afterProjectionDrop = markIntelSeen([subject('a', 2, 0)], seen);
    expect(afterProjectionDrop.b).toBe(1);
    // …and when 'b' comes back unchanged it is still read.
    expect(intelUnreadCount([subject('a', 2, 0), subject('b', 1, 0)], afterProjectionDrop))
      .toBe(0);
  });
});
