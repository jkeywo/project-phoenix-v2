//! Mission ship-slot reservation and hull confirmation (issue #1518).
//! Transport-independent so browser and native lobby adapters share one law.

use std::collections::BTreeMap;

use crate::world::config::{ShipSlotConfig, UnclaimedSlotPolicy};

/// Stable authored identity of the mission slot a spawned player ship occupies.
///
/// This is deliberately separate from `FleetSlotOf(HostSlot)`: the latter is
/// transport authority, while this value is content vocabulary used by
/// Objective recipient selectors.
#[derive(bevy::prelude::Component, Clone, Debug, PartialEq, Eq)]
pub struct AuthoredShipSlotId(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotReservation {
    pub claimant: String,
    pub hull: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimOutcome {
    Claimed,
    AlreadyHeld,
    Occupied,
    UnknownSlot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HullOutcome {
    Confirmed,
    NotClaimant,
    HullNotAllowed,
    UnknownSlot,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShipSlotReservations {
    reservations: BTreeMap<String, SlotReservation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LaunchSource {
    Claimed,
    Backfill,
}

/// One present ship in the launch-frozen roster. Absent slots deliberately
/// have no row, so peers cannot accidentally treat an empty audience as all.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LaunchedSlot {
    pub slot_id: String,
    pub hull: String,
    pub claimant: Option<String>,
    pub source: LaunchSource,
}

/// Immutable initial roster captured at launch and safe to snapshot/replay.
#[derive(
    bevy::prelude::Resource,
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct FrozenShipSlots(pub Vec<LaunchedSlot>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackfillSlotOutcome {
    Applied,
    AlreadyPresent,
    UnknownSlot,
}

impl ShipSlotReservations {
    pub fn claim(
        &mut self,
        slots: &[ShipSlotConfig],
        slot_id: &str,
        claimant: &str,
    ) -> ClaimOutcome {
        if !slots.iter().any(|slot| slot.id == slot_id) {
            return ClaimOutcome::UnknownSlot;
        }
        match self.reservations.get(slot_id) {
            Some(held) if held.claimant == claimant => ClaimOutcome::AlreadyHeld,
            Some(_) => ClaimOutcome::Occupied,
            None => {
                self.reservations.insert(
                    slot_id.to_string(),
                    SlotReservation {
                        claimant: claimant.to_string(),
                        hull: None,
                    },
                );
                ClaimOutcome::Claimed
            }
        }
    }

    pub fn confirm_hull(
        &mut self,
        slots: &[ShipSlotConfig],
        slot_id: &str,
        claimant: &str,
        hull: &str,
    ) -> HullOutcome {
        let Some(slot) = slots.iter().find(|slot| slot.id == slot_id) else {
            return HullOutcome::UnknownSlot;
        };
        if !slot.ships.iter().any(|ship| ship.template_path == hull) {
            return HullOutcome::HullNotAllowed;
        }
        let Some(held) = self.reservations.get_mut(slot_id) else {
            return HullOutcome::NotClaimant;
        };
        if held.claimant != claimant {
            return HullOutcome::NotClaimant;
        }
        held.hull = Some(hull.to_string());
        HullOutcome::Confirmed
    }

    /// Back and pre-start disconnect intentionally share immediate release.
    pub fn release_claimant(&mut self, claimant: &str) -> Vec<String> {
        let released: Vec<_> = self
            .reservations
            .iter()
            .filter(|(_, held)| held.claimant == claimant)
            .map(|(slot, _)| slot.clone())
            .collect();
        self.reservations
            .retain(|_, held| held.claimant != claimant);
        released
    }

    pub fn get(&self, slot_id: &str) -> Option<&SlotReservation> {
        self.reservations.get(slot_id)
    }

    pub fn claimed_are_confirmed(&self) -> bool {
        self.reservations.values().all(|held| held.hull.is_some())
    }

    /// Freeze claimed hulls plus authored unclaimed policy in slot order.
    /// Returns `None` while any claimed slot still lacks a confirmed hull.
    pub fn freeze(&self, slots: &[ShipSlotConfig]) -> Option<FrozenShipSlots> {
        if !self.claimed_are_confirmed() {
            return None;
        }
        let mut launched = Vec::new();
        for slot in slots {
            if let Some(held) = self.reservations.get(&slot.id) {
                launched.push(LaunchedSlot {
                    slot_id: slot.id.clone(),
                    hull: held.hull.clone()?,
                    claimant: Some(held.claimant.clone()),
                    source: LaunchSource::Claimed,
                });
            } else if slot.unclaimed == UnclaimedSlotPolicy::Backfill {
                launched.push(LaunchedSlot {
                    slot_id: slot.id.clone(),
                    hull: slot.default_ship.clone(),
                    claimant: None,
                    source: LaunchSource::Backfill,
                });
            }
        }
        Some(FrozenShipSlots(launched))
    }
}

impl FrozenShipSlots {
    /// A disposable Workshop Test controls exactly one authored slot. Every
    /// other present slot uses the same Backfill/absent freeze as a live launch.
    pub fn for_workshop_test(
        slots: &[ShipSlotConfig],
        controlled_slot: &str,
        hull: &str,
    ) -> Result<Self, String> {
        let mut reservations = ShipSlotReservations::default();
        if reservations.claim(slots, controlled_slot, "workshop-test") != ClaimOutcome::Claimed {
            return Err(format!("Unknown Test ship slot {controlled_slot:?}"));
        }
        if reservations.confirm_hull(slots, controlled_slot, "workshop-test", hull)
            != HullOutcome::Confirmed
        {
            return Err(format!(
                "Hull {hull:?} is not offered by Test ship slot {controlled_slot:?}"
            ));
        }
        reservations
            .freeze(slots)
            .ok_or_else(|| "Test slot confirmation failed".into())
    }

    /// Add one still-empty authored slot using its default hull. This is the
    /// pre-launch GM operation; sorting back into authored order preserves the
    /// same spawn/mint order an authored `unclaimed = "backfill"` row has.
    pub fn backfill_slot(
        &mut self,
        slots: &[ShipSlotConfig],
        slot_id: &str,
    ) -> BackfillSlotOutcome {
        let Some(slot) = slots.iter().find(|slot| slot.id == slot_id) else {
            return BackfillSlotOutcome::UnknownSlot;
        };
        if self.0.iter().any(|row| row.slot_id == slot_id) {
            return BackfillSlotOutcome::AlreadyPresent;
        }
        self.0.push(LaunchedSlot {
            slot_id: slot.id.clone(),
            hull: slot.default_ship.clone(),
            claimant: None,
            source: LaunchSource::Backfill,
        });
        self.0.sort_by_key(|row| {
            slots
                .iter()
                .position(|slot| slot.id == row.slot_id)
                .unwrap_or(usize::MAX)
        });
        BackfillSlotOutcome::Applied
    }

    /// Freeze a direct launch that has no ship-slot claimant surface.
    ///
    /// Headless and native `--world` boots enter the ordinary mission lobby
    /// without running the browser/native runtime slot arbiter. They therefore
    /// have no claims to preserve: every authored Backfill slot launches its
    /// default hull and every Absent slot stays omitted. Materialising that
    /// decision before `InProgress` prevents the spawn fallback from treating
    /// an Absent row as an ordinary NPC or applying one CLI-selected hull to an
    /// authored multi-ship roster.
    pub fn from_unclaimed_slots(slots: &[ShipSlotConfig]) -> Self {
        ShipSlotReservations::default()
            .freeze(slots)
            .expect("an empty reservation table has no unconfirmed claims")
    }

    /// Freeze an authenticated mesh roster against the world's authored slot
    /// vocabulary. Every peer runs this before adopting the roster, so an
    /// unknown/duplicate slot or a hull outside that slot's allowlist refuses
    /// the fleet instead of spawning different worlds.
    pub fn from_fleet_roster(
        slots: &[ShipSlotConfig],
        roster: &crate::lockstep::FleetRoster,
    ) -> Result<Self, String> {
        let mut claimed = BTreeMap::new();
        for ship in roster.ships() {
            let slot_id = ship
                .authored_slot_id
                .as_deref()
                .ok_or_else(|| format!("fleet host {} names no authored ship slot", ship.host.0))?;
            let slot = slots
                .iter()
                .find(|slot| slot.id == slot_id)
                .ok_or_else(|| {
                    format!(
                        "fleet host {} names unknown ship slot {slot_id:?}",
                        ship.host.0
                    )
                })?;
            let hull = ship
                .ship_path
                .as_deref()
                .ok_or_else(|| format!("fleet host {} has no confirmed hull", ship.host.0))?;
            if !slot
                .ships
                .iter()
                .any(|offered| offered.template_path == hull)
            {
                return Err(format!(
                    "fleet host {} chose hull {hull:?} outside ship slot {slot_id:?}",
                    ship.host.0
                ));
            }
            if claimed
                .insert(
                    slot_id.to_string(),
                    LaunchedSlot {
                        slot_id: slot_id.to_string(),
                        hull: hull.to_string(),
                        claimant: Some(ship.host.slot_id()),
                        source: LaunchSource::Claimed,
                    },
                )
                .is_some()
            {
                return Err(format!(
                    "more than one fleet host claimed ship slot {slot_id:?}"
                ));
            }
        }
        let mut launched = Vec::new();
        for slot in slots {
            if let Some(claim) = claimed.remove(&slot.id) {
                launched.push(claim);
            } else if slot.unclaimed == UnclaimedSlotPolicy::Backfill {
                launched.push(LaunchedSlot {
                    slot_id: slot.id.clone(),
                    hull: slot.default_ship.clone(),
                    claimant: None,
                    source: LaunchSource::Backfill,
                });
            }
        }
        Ok(Self(launched))
    }
}

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

#[cfg(test)]
#[path = "ship_slots_tests.rs"]
mod tests;
