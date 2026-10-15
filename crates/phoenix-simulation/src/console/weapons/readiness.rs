//! Borrowed read projection shared by ordinary publishers and reconnect.
//! No world query or second target/mount owner is introduced here.
use super::*;
use crate::core::messages::{SystemId, WeaponReadiness, WeaponTargetGeometry};

pub(super) struct Context<'a> {
    pub config: Option<&'a crate::ship_plugin::ShipConfigComponent>,
    pub boards: &'a crate::server_app::ShipSystemBlackboards,
    pub tracks: Option<&'a crate::science::broad::BroadTracks>,
    pub controls: Option<&'a crate::science::broad::SensingControls>,
    pub targets: Option<&'a super::super::assignments::ShipWeaponTargets>,
    pub profiles: Option<&'a crate::weapons::mounts::MountProfiles>,
    pub mounts: Option<&'a crate::weapons::mounts::ShipMounts>,
    pub observer: Option<&'a crate::entities::spawner::EntityUuid>,
    pub runtime: Option<&'a crate::world::server::WorldContentRuntime>,
}
impl Context<'_> {
    pub fn measured(&self) -> bool {
        crate::science::contact_routing::measured_machinery(self.tracks, self.controls)
    }
    pub fn geometry(
        &self,
        owner: &SystemId,
        mount: &crate::weapons::mount::MountConfig,
        physics: ShipPhysics,
        legacy: &impl Fn(&str) -> Option<[f32; 2]>,
    ) -> Option<WeaponTargetGeometry> {
        let point = if let Some(config) = self.config {
            let contact = super::super::assignments::effective_weapon_contact(
                &config.0,
                owner,
                self.boards,
                self.tracks,
                self.controls,
                self.targets,
            )?;
            let route = crate::science::contact_routing::resolve_machinery(
                Some(&config.0),
                Some(self.boards),
                self.tracks,
                self.controls,
                owner,
                &contact,
            )?;
            if self.runtime.is_some_and(|runtime| {
                self.observer.is_none_or(|observer| {
                    crate::gm::operations::contact::mode(
                        &runtime.contact_overrides,
                        &observer.0,
                        &route,
                    ) == crate::gm::operations::contact::ContactMode::Conceal
                })
            }) {
                return None;
            }
            if self.measured() {
                crate::weapons::mounts::measured_point(
                    &config.0,
                    self.boards,
                    self.tracks?,
                    owner,
                    &contact,
                )?
            } else {
                legacy(&route)?
            }
        } else {
            if self.measured() {
                return None;
            }
            legacy(&viewscreen_combat_lock(self.boards)?)?
        };
        if !point.iter().all(|value| value.is_finite()) {
            return None;
        }
        Some(crate::weapons::mounts::geometry(
            self.profiles,
            self.mounts,
            owner,
            mount,
            point,
            [physics.x, physics.z],
            physics.yaw,
        ))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &self,
        physics: ShipPhysics,
        sources: Option<&ShipSystemControlSources>,
        phasers: Option<&PhaserCombatConfigResource>,
        torpedoes: Option<&TorpedoSystemResource>,
        blasters: Option<&BlasterSystemResource>,
        banks: &mut [PhaserBankState],
        tubes: &mut [TorpedoTubeState],
        bolts: &mut [BlasterBankState],
        legacy: impl Fn(&str) -> Option<[f32; 2]>,
    ) {
        let online = |owner: &SystemId| sources.is_none_or(|sources| !sources.0.is_offline(owner));
        for bank in banks {
            let Some(owner) = crate::ship::system_registry::phaser_bank_system_id(&bank.id) else {
                continue;
            };
            let configured = phasers.and_then(|config| config.0.bank_by_id(&bank.id));
            let mount = configured.map_or(
                crate::weapons::mount::MountConfig {
                    facing_deg: 0.0,
                    fire_arc_deg: 180.0,
                    range: crate::entities::config::PhaserCombatConfig::DEFAULT_PHASER_RANGE,
                },
                |bank| crate::weapons::mount::MountConfig {
                    facing_deg: bank.facing_deg,
                    fire_arc_deg: bank.fire_arc_deg,
                    range: if bank.beam_range > 0.0 {
                        bank.beam_range
                    } else {
                        crate::entities::config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
                    },
                },
            );
            let geometry = self.geometry(&owner, &mount, physics, &legacy);
            bank.fire_ready = geometry.is_some_and(|geometry| geometry.in_arc && geometry.in_range);
            bank.readiness =
                WeaponReadiness::evaluate(online(&owner), bank.on_cooldown, false, false, geometry);
        }
        for tube in tubes {
            let Some(owner) = crate::ship::system_registry::torpedo_tube_system_id(&tube.id) else {
                continue;
            };
            let Some(system) = torpedoes else { continue };
            let Some(config) = system.0.tube(&tube.id) else {
                continue;
            };
            let geometry = self.geometry(
                &owner,
                &crate::weapons::mount::MountConfig {
                    facing_deg: config.facing_deg,
                    fire_arc_deg: config.fire_arc_deg,
                    range: system.0.config.speed * system.0.config.lifespan,
                },
                physics,
                &legacy,
            );
            let loading = tube.loaded_count == 0 && tube.state == "loading";
            tube.readiness = WeaponReadiness::evaluate(
                online(&owner),
                false,
                loading,
                tube.loaded_count == 0 && !loading,
                geometry,
            );
        }
        for bolt in bolts {
            let Some(owner) = crate::ship::system_registry::blaster_bank_system_id(&bolt.id) else {
                continue;
            };
            let Some(bank) =
                blasters.and_then(|banks| banks.0.iter().find(|bank| bank.config.id == bolt.id))
            else {
                continue;
            };
            let geometry = self.geometry(&owner, &bank.config.mount(), physics, &legacy);
            bolt.fire_ready = bank.is_fire_ready()
                && geometry.is_some_and(|geometry| geometry.in_arc && geometry.in_range);
            bolt.readiness = WeaponReadiness::evaluate(
                online(&owner),
                bolt.on_cooldown,
                bank.volley.charging || bank.volley.pending_volley > 0,
                false,
                geometry,
            );
        }
    }
    pub fn bearing(&self, owner: &SystemId, fixed: f32) -> Option<f32> {
        let Some(profile) = self.profiles.and_then(|profiles| profiles.0.get(owner)) else {
            return Some(fixed);
        };
        Some(self.mounts?.0.get(owner)?.bearing_deg(profile))
    }
}

#[cfg(test)]
#[path = "readiness_tests.rs"]
mod tests;
