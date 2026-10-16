//! Physical aggregate handling for the existing guided tube owner. No queue or stock.
use super::torpedo::{TorpedoSystem, TubeLoadState};
use crate::entities::config::TorpedoTubeConfig;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoaderGroupConfig {
    pub id: String,
    pub tubes: Vec<String>,
    pub work_rate: f32,
}
pub fn validate_groups(
    groups: &[LoaderGroupConfig],
    tubes: &[TorpedoTubeConfig],
) -> Result<(), String> {
    let mut names = BTreeSet::new();
    let mut members = BTreeSet::new();
    for group in groups {
        if group.id.is_empty()
            || group.id.len() > 115
            || !group
                .id
                .bytes()
                .all(|v| v.is_ascii_lowercase() || v.is_ascii_digit() || v == b'-')
            || !names.insert(&group.id)
            || group.tubes.len() < 2
            || !group.work_rate.is_finite()
            || group.work_rate <= 0.0
            || group.work_rate > group.tubes.len() as f32
        {
            return Err("loader groups need unique bounded ids, at least two guided members and finite positive work_rate no greater than member count".into());
        }
        for member in &group.tubes {
            if !tubes.iter().any(|tube| &tube.id == member) || !members.insert(member) {
                return Err(
                    "loader group member is missing, duplicated or belongs to another group".into(),
                );
            }
        }
    }
    Ok(())
}
impl TorpedoSystem {
    pub fn loader_group(&self, tube: &str) -> Option<&LoaderGroupConfig> {
        self.loader_groups
            .iter()
            .find(|group| group.tubes.iter().any(|member| member == tube))
    }
    pub fn loader_view(&self, tube: &str) -> Option<phoenix_model::shared_loader::LoaderView> {
        let group = self.loader_group(tube)?;
        let member = self.tube(tube)?;
        let waiting = self.loader_wait.get(tube).copied().or_else(|| {
            (member.loaded_count < member.target_count
                && !matches!(member.load_state, TubeLoadState::Loading { .. }))
            .then_some(if self.magazine.available() < 1 {
                phoenix_model::shared_loader::LoaderWait::Stock
            } else {
                phoenix_model::shared_loader::LoaderWait::MagazineUnavailable
            })
        });
        Some(phoenix_model::shared_loader::LoaderView {
            group: group.id.clone(),
            work_rate: group.work_rate,
            priority: member.load_priority,
            waiting,
        })
    }
    pub fn set_load_priority(&mut self, tube: &str, priority: u16) -> bool {
        if self.loader_group(tube).is_none() {
            return false;
        }
        let Some(tube) = self.tube_mut(tube) else {
            return false;
        };
        tube.load_priority = priority;
        true
    }
    pub fn loader_order(&self) -> Vec<usize> {
        let mut order: Vec<_> = (0..self.tubes.len()).collect();
        if self.loader_groups.is_empty() {
            return order;
        }
        // Independent members preserve their relative physical order. Groups do
        // not borrow capacity from one another or from independent equipment.
        order.sort_by_key(|index| {
            let tube = &self.tubes[*index];
            self.loader_group(&tube.id).map_or(
                (String::new(), 0, false, format!("{index:010}")),
                |group| {
                    (
                        group.id.clone(),
                        tube.load_priority,
                        !matches!(
                            tube.load_state,
                            TubeLoadState::Loading { .. } | TubeLoadState::Unloading { .. }
                        ),
                        format!("torpedo-tube-{}", tube.id.replace('_', "-")),
                    )
                },
            )
        });
        order
    }
    pub fn loader_budgets(&self, dt: f32, magazine_operation: f32) -> BTreeMap<String, f32> {
        let factor = if magazine_operation.is_finite() {
            magazine_operation.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.loader_groups
            .iter()
            .map(|group| (group.id.clone(), group.work_rate * dt.max(0.0) * factor))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weapons::torpedo::{TorpedoConfig, TorpedoWork};
    use std::collections::HashMap;
    #[test]
    fn actual_optional_loader_hull_resolves_complete_inherited_equipment_without_changing_base() {
        use crate::{core::messages::SystemId, entities::config::EntityConfig};
        let accepted = |path| {
            let resolved = crate::test_fixtures::resolve_from_disk(path).unwrap();
            EntityConfig::from_toml(&resolved.toml).unwrap()
        };
        let base = accepted("assets/entities/alliance_cruiser.toml");
        let shared = accepted("assets/entities/shared_loader_cruiser.toml");
        let base_torpedoes = base.torpedoes.as_ref().unwrap();
        assert!(base_torpedoes.loader_groups.is_empty());
        let torpedoes = shared.torpedoes.as_ref().unwrap();
        assert_eq!(torpedoes.loader_groups.len(), 1);
        let group = &torpedoes.loader_groups[0];
        assert_eq!(group.id, "forward");
        assert_eq!(group.tubes, ["fore_port", "fore_starboard"]);
        assert_eq!(group.work_rate, 1.0);
        assert!(torpedoes.tubes.iter().any(|tube| tube.id == "aft"));
        let ship = shared.ship_config.as_ref().unwrap();
        assert_eq!(ship.stations.len(), 6);
        let energy = shared.energy.as_ref().unwrap();
        assert!(energy.reactor.max_output_per_sec > 0.0);
        assert!(energy.reactor.reserve.capacity > 0.0);
        let interior = shared.interior.as_ref().unwrap();
        for owner in std::iter::once(SystemId("torpedo-magazine".into())).chain(
            group
                .tubes
                .iter()
                .map(|tube| SystemId(format!("torpedo-tube-{}", tube.replace('_', "-")))),
        ) {
            let system = ship.system(&owner).unwrap();
            assert!(ship.console_for_system(&owner).is_some());
            assert!(interior.placements.contains_key(&owner));
            let battery = energy
                .consumers
                .iter()
                .find(|consumer| consumer.system == owner)
                .unwrap();
            assert!(battery.reserve.capacity > 0.0);
            assert!(!system.damage_components.is_empty());
        }
        // Compare the entire accepted recipe, not a private JSON merge or a
        // handful of scalar fields: this optional hull changes only its public
        // name and shared handling declaration. The independent source stays
        // complete and unchanged, including reactor, Console, layout and AI data.
        let mut equivalent = shared;
        equivalent.name = base.name.clone();
        equivalent.torpedoes.as_mut().unwrap().loader_groups.clear();
        assert_eq!(equivalent, base);
    }

    fn system(grouped: bool) -> TorpedoSystem {
        let mut system = TorpedoSystem::new(TorpedoConfig {
            load_time: 2.0,
            ..Default::default()
        });
        if grouped {
            system.loader_groups = vec![LoaderGroupConfig {
                id: "forward".into(),
                tubes: vec!["fore_port".into(), "fore_starboard".into()],
                work_rate: 1.0,
            }];
        }
        for id in ["fore_port", "fore_starboard"] {
            system.set_volley_target(id, 1);
            assert!(system.start_load(id));
        }
        system
    }
    fn advance(
        system: &mut TorpedoSystem,
        dt: f32,
        factor: f32,
        work: &mut impl FnMut(TorpedoWork<'_>) -> f32,
    ) {
        system.advance_with_loader_work(dt, &HashMap::new(), factor, work, &mut || {
            panic!("no launch")
        });
        system.validate_magazine().unwrap();
    }
    #[test]
    fn independent_loaders_keep_parallel_timing_and_shared_rate_limits_actual_total_work() {
        let mut independent = system(false);
        advance(&mut independent, 2.0, 0.0, &mut |_| 1.0);
        assert_eq!(
            independent
                .tubes
                .iter()
                .map(|tube| tube.loaded_count)
                .sum::<u32>(),
            2
        );
        let mut shared = system(true);
        let mut paid = 0.0;
        for _ in 0..2 {
            advance(&mut shared, 1.0, 1.0, &mut |work| {
                if let TorpedoWork::Progress { dt, .. } = work {
                    paid += dt;
                }
                1.0
            });
        }
        assert_eq!(paid, 2.0);
        assert_eq!(shared.tube("fore_port").unwrap().loaded_count, 1);
        assert_eq!(shared.tube("fore_starboard").unwrap().loaded_count, 0);
        advance(&mut shared, 2.0, 1.0, &mut |_| 1.0);
        assert_eq!(shared.tube("fore_starboard").unwrap().loaded_count, 1);
        assert_eq!(shared.magazine.level, 8);
    }
    #[test]
    fn changed_priority_preserves_paid_partial_work_and_unfunded_member_leaves_capacity() {
        let mut shared = system(true);
        advance(&mut shared, 0.5, 1.0, &mut |_| 1.0);
        let old = shared.tube("fore_port").unwrap().load_state.clone();
        assert!(shared.set_load_priority("fore_starboard", 1));
        advance(&mut shared, 0.5, 1.0, &mut |_| 1.0);
        assert_eq!(shared.tube("fore_port").unwrap().load_state, old);
        let other = shared.tube("fore_starboard").unwrap().load_state.clone();
        advance(&mut shared, 0.5, 1.0, &mut |work| {
            if matches!(work,TorpedoWork::Progress{tube,..} if tube=="fore_starboard") {
                0.0
            } else {
                1.0
            }
        });
        assert_eq!(shared.tube("fore_starboard").unwrap().load_state, other);
        assert_ne!(shared.tube("fore_port").unwrap().load_state, old);
        assert_eq!(shared.magazine.reserved.len(), 2);
    }
    #[test]
    fn actual_paid_fraction_spends_shared_budget_once_and_magazine_pause_retains_claims() {
        let mut shared = system(true);
        let before = shared.magazine.clone();
        advance(&mut shared, 1.0, 0.0, &mut |_| 1.0);
        assert_eq!(shared.magazine, before);
        assert_eq!(
            shared.loader_view("fore_port").unwrap().waiting,
            Some(phoenix_model::shared_loader::LoaderWait::MagazineUnavailable)
        );
        let mut offered = Vec::new();
        advance(&mut shared, 1.0, 1.0, &mut |work| {
            if let TorpedoWork::Progress { tube, dt } = work {
                offered.push((tube.to_owned(), dt));
                return 0.5;
            }
            1.0
        });
        assert_eq!(
            offered,
            vec![("fore_port".into(), 1.0), ("fore_starboard".into(), 0.5)]
        );
        assert_eq!(
            shared.tube("fore_port").unwrap().load_state,
            TubeLoadState::Loading {
                remaining: 1.5,
                total: 2.0
            }
        );
        assert_eq!(
            shared.tube("fore_starboard").unwrap().load_state,
            TubeLoadState::Loading {
                remaining: 1.75,
                total: 2.0
            }
        );
    }
    #[test]
    fn group_order_is_independent_of_authored_member_and_tube_vector_order() {
        let mut normal = system(true);
        let mut reversed = normal.clone();
        reversed.tubes.reverse();
        reversed.loader_groups[0].tubes.reverse();
        advance(&mut normal, 1.0, 1.0, &mut |_| 1.0);
        advance(&mut reversed, 1.0, 1.0, &mut |_| 1.0);
        for id in ["fore_port", "fore_starboard"] {
            assert_eq!(
                normal.tube(id).unwrap().load_state,
                reversed.tube(id).unwrap().load_state
            );
        }
    }
    #[test]
    fn unloading_competes_with_loading_and_returns_only_the_same_paid_round() {
        let mut shared = system(true);
        shared.set_load_priority("fore_starboard", 1);
        advance(&mut shared, 2.0, 1.0, &mut |_| 1.0);
        assert_eq!(shared.tube("fore_starboard").unwrap().loaded_count, 1);
        let held = shared.tube("fore_port").unwrap().load_state.clone();
        shared.set_volley_target("fore_starboard", 0);
        assert!(shared.start_unload("fore_starboard"));
        advance(&mut shared, 1.0, 1.0, &mut |_| 1.0);
        assert_eq!(shared.tube("fore_port").unwrap().load_state, held);
        assert_eq!(shared.magazine.level, 9);
        advance(&mut shared, 1.0, 1.0, &mut |_| 1.0);
        assert_eq!(shared.magazine.level, 10);
        assert_eq!(shared.rounds_aboard(), 10);
        assert_eq!(shared.tube("fore_port").unwrap().load_state, held);
    }
}
