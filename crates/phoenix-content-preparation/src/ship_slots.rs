use crate::world::config::ShipSlotConfig;
/// Apply one scenario manifest's playable-hull allowlist to authored slots.
///
/// These returned rows are launch authority, not picker decoration. Refusing
/// an empty slot or an excluded default prevents curation from being bypassed
/// by an unclaimed Backfill hull or a forged mesh announcement.
pub fn curate_ship_slots(
    slots: &[ShipSlotConfig],
    curated_ships: &[String],
) -> Result<Vec<ShipSlotConfig>, String> {
    if curated_ships.is_empty() {
        return Ok(slots.to_vec());
    }
    slots
        .iter()
        .map(|slot| {
            let mut curated = slot.clone();
            curated
                .ships
                .retain(|ship| curated_ships.iter().any(|path| path == &ship.template_path));
            if curated.ships.is_empty() {
                return Err(format!(
                    "ship slot {:?} offers no hull allowed by this scenario manifest",
                    slot.id
                ));
            }
            if !curated
                .ships
                .iter()
                .any(|ship| ship.template_path == curated.default_ship)
            {
                return Err(format!(
                    "ship slot {:?} default {:?} is excluded by this scenario manifest",
                    slot.id, slot.default_ship
                ));
            }
            Ok(curated)
        })
        .collect()
}
