import { describe, it, expect } from 'vitest';
import { canonicalStationRatings, MAX_FLEET_CREW_SEATS, MAX_FLEET_CREW_FIELD_BYTES } from '../../gui/fleet-crew.js';
import { openFleet, setCrewReadiness, rosterOf, simulationRosterOf, freezeFleet } from '../../gui/host-mesh.js';

describe('frozen fleet crew ratings', () => {
  it('keeps exact chosen ratings in a stable, independently owned UTF-8 ordering', () => {
    const input = [['\u{10000}', 'Manual'], ['helm', 'Assisted'], ['\ue000', 'Std']];
    const result = canonicalStationRatings(input);
    expect(result).toEqual([['helm', 'Assisted'], ['\ue000', 'Std'], ['\u{10000}', 'Manual']]);
    input[1][1] = 'Changed';
    expect(result[0][1]).toBe('Assisted');
  });

  it('refuses malformed, duplicate and oversized authored identities without dropping a seat', () => {
    for (const value of [null, {}, [['helm']], [['helm', 'Std', 'token']], [['', 'Std']],
      [['helm', '']], [['helm', 'Std'], ['helm', 'Assisted']], [['helm', 'Bad\nRating']],
      [['\ud800', 'Std']], [['helm', 'é'.repeat(MAX_FLEET_CREW_FIELD_BYTES)]],
      Array.from({ length: MAX_FLEET_CREW_SEATS + 1 }, (_, i) => [`s${i}`, 'Std'])]) {
      expect(canonicalStationRatings(value), JSON.stringify(value)).toBeNull();
    }
    expect(canonicalStationRatings()).toEqual([]);
  });

  it('freezes selected ratings into the simulation roster and refuses later changes', () => {
    let fleet = openFleet({ maxSlots: 4, ship: { template_path: 'cruiser.toml' } });
    const selected = setCrewReadiness(fleet, fleet.owner, { connected: 1, ready: 1 }, [['helm', 'Assisted']]);
    expect(selected.ok).toBe(true);
    fleet = selected.fleet;
    const frozen = freezeFleet(fleet);
    expect(simulationRosterOf(rosterOf(frozen), frozen.owner).ships[0].crew).toEqual([['helm', 'Assisted']]);
    expect(setCrewReadiness(frozen, frozen.owner, { connected: 1, ready: 1 }, [['helm', 'Manual']]).ok).toBe(false);
    const malformed = rosterOf(fleet);
    malformed.slots[0].station_ratings = [['helm', 'Std'], ['helm', 'Assisted']];
    expect(simulationRosterOf(malformed, fleet.owner)).toBeNull();
  });
});
