/**
 * Browser-host authored-slot agreement gate (issue #1518).
 *
 * A fleet owner applies its own patch synchronously. A member has only sent a
 * proposal until the owner's roster echoes the exact slot+hull. Keeping that
 * distinction pure makes a simultaneous claim race testable without a DOM or
 * transport and prevents the losing page from starting its world optimistically.
 */

export function rosterCarriesSelection(roster, meshSlot, desired) {
  if (!desired || !meshSlot || !roster || !Array.isArray(roster.slots)) return !desired;
  const row = roster.slots.find(slot => slot.id === meshSlot);
  return !!(row && row.ship
    && row.ship.template_path === desired.template_path
    && (row.ship.slot_id || null) === (desired.slot_id || null));
}

export function selectionDisposition({ isOwner, updateAccepted, roster, meshSlot, desired }) {
  if (updateAccepted === false) return 'refused';
  if (isOwner || !desired || !desired.slot_id
      || rosterCarriesSelection(roster, meshSlot, desired)) return 'accepted';
  return 'pending';
}

export function rolledBackSelection(selection) {
  return {
    scenario_id: selection && selection.scenario_id || null,
    slot_id: null,
    template_path: null,
  };
}

if (typeof window !== 'undefined') {
  window.fleetSelectionGate = {
    rosterCarriesSelection,
    selectionDisposition,
    rolledBackSelection,
  };
}
