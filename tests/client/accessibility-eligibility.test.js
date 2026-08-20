/**
 * tests/client/accessibility-eligibility.test.js — anonymous Station/rating
 * accessibility eligibility on the client (issue #1103).
 *
 * Pure-Node tests over the client half of the eligibility seam in
 * gui/accessibility-profile.js and gui/lobby-view.js. They cover AC5:
 *   - eligible and ineligible direct claims per (station, rating),
 *   - fallback skipping via the anonymous ineligible-station set,
 *   - the NEUTRAL shared shape (only station ids leave the device — no reason,
 *     no setting), versus the PRIVATE functional explanation kept locally,
 *   - private-setting non-disclosure: nothing derived carries the raw profile.
 *
 * The client mirrors the RUST rule (src/ship/eligibility.rs) from the projected
 * `station_assist_gaps` table; these tests pin that mirror.
 */
import { describe, it, expect } from 'vitest';
import {
  ASSIST_REQUEST,
  emptyAccessibilityProfile,
  profileWithAssistance,
  deriveStationEligibility,
  computeIneligibleStations,
  requestedAssistFunctions,
} from '../../gui/accessibility-profile.js';
import { lobbyViewModel } from '../../gui/lobby-view.js';

// A projection like the host sends on Welcome: per station → per rating → the
// assist-functions that station forces its holder to operate manually.
// Helm forces course-keeping at Std, automates it at Simplified; Comms forces
// dialogue-timing at Std; Captain covers everything (absent ⇒ no gaps).
const PROJECTION = {
  helm: { Std: ['helm.course-keeping'] },
  comms: { Std: ['comms.dialogue-timing'] },
};

function requestCourseKeeping() {
  return profileWithAssistance(emptyAccessibilityProfile(), 'helm.course-keeping', ASSIST_REQUEST);
}

describe('requestedAssistFunctions', () => {
  it('lists only functions set to ASSIST_REQUEST', () => {
    expect(requestedAssistFunctions(emptyAccessibilityProfile())).toEqual([]);
    expect(requestedAssistFunctions(requestCourseKeeping())).toEqual(['helm.course-keeping']);
  });
});

describe('deriveStationEligibility — per (station, rating)', () => {
  it('a profile requesting no assistance is eligible everywhere', () => {
    const r = deriveStationEligibility(emptyAccessibilityProfile(), PROJECTION.helm, 'Std');
    expect(r.eligible).toBe(true);
    expect(r.reason).toBeNull();
  });

  it('INELIGIBLE for a station/rating that forces the requested function manual', () => {
    const r = deriveStationEligibility(requestCourseKeeping(), PROJECTION.helm, 'Std');
    expect(r.eligible).toBe(false);
    // Private explanation names the functional consequence.
    expect(r.reason).toEqual({ functions: ['helm.course-keeping'] });
  });

  it('ELIGIBLE at a rating that automates the requested function', () => {
    // Simplified is not a gap for helm ⇒ no entry ⇒ eligible.
    const r = deriveStationEligibility(requestCourseKeeping(), PROJECTION.helm, 'Simplified');
    expect(r.eligible).toBe(true);
    expect(r.reason).toBeNull();
  });

  it('ELIGIBLE at a station that does not host the requested function', () => {
    // Requesting course-keeping at Comms (no helm-steering there) is fine.
    const r = deriveStationEligibility(requestCourseKeeping(), PROJECTION.comms, 'Std');
    expect(r.eligible).toBe(true);
  });

  it('missing station/rating projection is permissively eligible (DEFAULT TRUE)', () => {
    expect(deriveStationEligibility(requestCourseKeeping(), undefined, 'Std').eligible).toBe(true);
    expect(deriveStationEligibility(requestCourseKeeping(), PROJECTION.helm, 'Backfill').eligible).toBe(true);
  });
});

describe('computeIneligibleStations — the anonymous shared shape (AC2/AC5)', () => {
  const ratingFor = () => 'Std';
  const stations = ['captain', 'helm', 'comms'];

  it('returns ONLY the ineligible station ids — no reasons, no settings', () => {
    const profile = requestCourseKeeping();
    const out = computeIneligibleStations(profile, PROJECTION, ratingFor, stations);
    expect(out).toEqual(['helm']); // only helm forces course-keeping manual at Std
    // The shared result is a flat list of ids: no object, no reason, no setting.
    out.forEach((id) => expect(typeof id).toBe('string'));
    const serialized = JSON.stringify(out);
    expect(serialized).not.toMatch(/reason|assist|request|profile|course-keeping/);
  });

  it('reports multiple ineligible stations when multiple functions are requested', () => {
    let profile = profileWithAssistance(emptyAccessibilityProfile(), 'helm.course-keeping', ASSIST_REQUEST);
    profile = profileWithAssistance(profile, 'comms.dialogue-timing', ASSIST_REQUEST);
    const out = computeIneligibleStations(profile, PROJECTION, ratingFor, stations);
    expect(out).toEqual(['comms', 'helm']); // sorted
  });

  it('is empty when the profile requests no assistance', () => {
    const out = computeIneligibleStations(emptyAccessibilityProfile(), PROJECTION, ratingFor, stations);
    expect(out).toEqual([]);
  });
});

describe('private-setting non-disclosure (AC5)', () => {
  it('the private explanation carries functional ids ONLY, never raw settings', () => {
    const profile = requestCourseKeeping();
    const r = deriveStationEligibility(profile, PROJECTION.helm, 'Std');
    // The reason is a functional consequence, not the profile itself.
    expect(r.reason.functions).toEqual(['helm.course-keeping']);
    expect(r.reason).not.toHaveProperty('assistance');
    expect(r.reason).not.toHaveProperty('presentation');
    // No presentation setting (text scale, contrast, motion) ever appears.
    expect(JSON.stringify(r.reason)).not.toMatch(/textScale|contrast|reducedMotion|presentation/);
  });
});

describe('lobbyViewModel eligibility annotation (AC1)', () => {
  const s = {
    phase: 'Lobby',
    players: [{ token: 'me', ready: false }],
    stations: [
      { id: 'helm', name: 'Helm', holder_name: null, holder_token: null, ratings: ['Std'] },
      { id: 'captain', name: 'Captain', holder_name: null, holder_token: null, ratings: ['Std'] },
    ],
    maxPlayers: 2,
  };

  it('marks a free-but-ineligible seat as an ineligible button with a private reason', () => {
    const profile = requestCourseKeeping();
    const eligibilityFor = (st) =>
      deriveStationEligibility(profile, PROJECTION[st.id], 'Std');
    const vm = lobbyViewModel(s, 'me', null, { eligibilityFor });

    const helm = vm.rows.find((r) => r.id === 'helm');
    expect(helm.button).toBe('ineligible');
    expect(helm.eligible).toBe(false);
    expect(helm.ineligibleReason).toEqual({ functions: ['helm.course-keeping'] });

    const captain = vm.rows.find((r) => r.id === 'captain');
    expect(captain.button).toBe('claim'); // captain covers everything ⇒ claimable
    expect(captain.eligible).toBe(true);
    expect(captain.ineligibleReason).toBeNull();
  });

  it('leaves every seat claimable when no assistance is requested', () => {
    const eligibilityFor = (st) =>
      deriveStationEligibility(emptyAccessibilityProfile(), PROJECTION[st.id], 'Std');
    const vm = lobbyViewModel(s, 'me', null, { eligibilityFor });
    expect(vm.rows.every((r) => r.button === 'claim')).toBe(true);
  });
});
