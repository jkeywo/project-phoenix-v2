/** Closed EntityTag selection policy, shared by weapon and portable beam forms. */
export const WEAPON_TARGET_TAGS = Object.freeze(['asteroid','ship','asteroid_field','star','planet','moon','region','structure','station','player','missile','trigger_volume','objective_marker','gm_removable','charted','cargo']);
export function validateWeaponTargets(value) {
  if (value === undefined || value === false) return;
  if (!Array.isArray(value) || value.some(tag => !WEAPON_TARGET_TAGS.includes(tag)) || new Set(value).size !== value.length)
    throw new Error('Weapon targets require distinct supported EntityTag values; an empty list deliberately holds fire.');
}
