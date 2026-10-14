//! Immutable layout validation and deterministic physical team routing.
use super::config::ShipConfig;
pub use phoenix_model::interior::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct InteriorGraph {
    pub config: InteriorConfig,
    adjacency: BTreeMap<String, Vec<usize>>,
}
fn id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
impl InteriorGraph {
    pub fn compile(
        config: &InteriorConfig,
        ship: &ShipConfig,
        teams: usize,
    ) -> Result<Self, String> {
        let mut decks = BTreeSet::new();
        for deck in &config.decks {
            if !id(&deck.id)
                || !decks.insert(&deck.id)
                || deck.label.is_empty()
                || !deck.damage_weight.is_finite()
                || deck.damage_weight <= 0.0
            {
                return Err("interior has an invalid or duplicate deck".into());
            }
        }
        let mut adjacency = BTreeMap::new();
        for room in &config.compartments {
            if !id(&room.id)
                || adjacency.insert(room.id.clone(), Vec::new()).is_some()
                || !decks.contains(&room.deck)
                || room.label.is_empty()
                || !id(&room.material)
                || ![room.x, room.z, room.width, room.depth]
                    .iter()
                    .all(|v| v.is_finite())
                || room.width <= 0.0
                || room.depth <= 0.0
            {
                return Err("interior has an invalid or duplicate compartment".into());
            }
        }
        if adjacency.is_empty() {
            return Err("interior requires compartments".into());
        }
        let systems: BTreeSet<_> = ship.systems.iter().map(|s| s.id.clone()).collect();
        if config.placements.keys().cloned().collect::<BTreeSet<_>>() != systems
            || config
                .placements
                .values()
                .any(|room| !adjacency.contains_key(room))
        {
            return Err("interior placements must name every installed System exactly once, including generated equipment".into());
        }
        if config
            .repair_placements
            .iter()
            .any(|(id, room)| systems.contains(id) || !adjacency.contains_key(room))
        {
            return Err("legacy repair placements require distinct hull bucket identities and valid compartments".into());
        }
        if config.team_homes.len() != teams
            || config
                .team_homes
                .iter()
                .any(|room| !adjacency.contains_key(room))
        {
            return Err("interior requires one valid home compartment per repair team".into());
        }
        let mut links = BTreeSet::new();
        for (index, link) in config.connections.iter().enumerate() {
            if !id(&link.id)
                || !links.insert(&link.id)
                || link.from == link.to
                || !adjacency.contains_key(&link.from)
                || !adjacency.contains_key(&link.to)
                || !link.travel_secs.is_finite()
                || link.travel_secs <= 0.0
                || link.boundary.as_ref().is_some_and(|system| {
                    ship.system(system).is_none_or(|system| {
                        system.kind != super::system_registry::ACCESS_GATE_KIND
                    })
                })
            {
                return Err("interior has an invalid connection or boundary reference".into());
            }
            adjacency
                .get_mut(&link.from)
                .expect("checked endpoint")
                .push(index);
            adjacency
                .get_mut(&link.to)
                .expect("checked endpoint")
                .push(index);
        }
        for links in adjacency.values_mut() {
            links.sort_by_key(|i| &config.connections[*i].id);
        }
        let graph = Self {
            config: config.clone(),
            adjacency,
        };
        let first = graph.config.compartments[0].id.as_str();
        if graph
            .adjacency
            .keys()
            .any(|room| graph.route(first, room, &BTreeSet::new()).is_none())
        {
            return Err("interior contains unreachable compartments".into());
        }
        Ok(graph)
    }
    /// Stable minimum-cost route; full path IDs break equal-cost ties.
    pub fn route(&self, from: &str, to: &str, blocked: &BTreeSet<String>) -> Option<Vec<String>> {
        if !self.adjacency.contains_key(from) || !self.adjacency.contains_key(to) {
            return None;
        }
        let mut best: BTreeMap<String, (f64, Vec<String>)> = BTreeMap::new();
        best.insert(from.into(), (0.0, Vec::new()));
        let mut visited = BTreeSet::new();
        loop {
            let (room, (cost, path)) = best
                .iter()
                .filter(|(room, _)| !visited.contains(*room))
                .min_by(|(a, (ac, ap)), (b, (bc, bp))| {
                    ac.total_cmp(bc)
                        .then_with(|| ap.cmp(bp))
                        .then_with(|| a.cmp(b))
                })
                .map(|(room, value)| (room.clone(), value.clone()))?;
            if room == to {
                return Some(path);
            }
            visited.insert(room.clone());
            for index in &self.adjacency[&room] {
                let link = &self.config.connections[*index];
                if blocked.contains(&link.id) {
                    continue;
                }
                let next = if link.from == room {
                    &link.to
                } else {
                    &link.from
                };
                if visited.contains(next) {
                    continue;
                }
                let mut next_path = path.clone();
                next_path.push(link.id.clone());
                let candidate = (cost + f64::from(link.travel_secs), next_path);
                if best.get(next).is_none_or(|old| {
                    candidate
                        .0
                        .total_cmp(&old.0)
                        .then_with(|| candidate.1.cmp(&old.1))
                        .is_lt()
                }) {
                    best.insert(next.clone(), candidate);
                }
            }
        }
    }
    pub fn initial_teams(&self) -> Vec<TeamNavigation> {
        self.config
            .team_homes
            .iter()
            .map(|home| TeamNavigation {
                location: TeamLocation::Compartment {
                    compartment: home.clone(),
                },
                destination: None,
                route: Vec::new(),
                blocked: false,
            })
            .collect()
    }
    pub fn compartment_for_system(&self, id: &crate::core::messages::SystemId) -> Option<&String> {
        self.config
            .placements
            .get(id)
            .or_else(|| self.config.repair_placements.get(id))
    }
    pub fn validate_navigation(&self, teams: &[TeamNavigation]) -> Result<(), String> {
        if teams.len() != self.config.team_homes.len() {
            return Err("interior team count mismatch".into());
        }
        for team in teams {
            if team
                .destination
                .as_ref()
                .is_some_and(|room| !self.adjacency.contains_key(room))
            {
                return Err("unknown team destination".into());
            }
            match &team.location {
                TeamLocation::Compartment { compartment }
                    if self.adjacency.contains_key(compartment) => {}
                TeamLocation::External { target } if !target.is_empty() => {}
                TeamLocation::Crossing {
                    connection,
                    from,
                    to,
                    elapsed,
                } if self.config.connections.iter().any(|link| {
                    &link.id == connection
                        && ((&link.from == from && &link.to == to)
                            || (&link.to == from && &link.from == to))
                        && elapsed.is_finite()
                        && *elapsed >= 0.0
                        && *elapsed < link.travel_secs
                }) => {}
                _ => return Err("invalid physical team location".into()),
            }
            if team
                .route
                .iter()
                .any(|key| !self.config.connections.iter().any(|link| &link.id == key))
            {
                return Err("unknown team route connection".into());
            }
            if !team.route.is_empty() {
                let mut room = match &team.location {
                    TeamLocation::Compartment { compartment } => compartment.clone(),
                    TeamLocation::Crossing {
                        connection, from, ..
                    } if team.route.first() == Some(connection) => from.clone(),
                    _ => return Err("team route is detached from its physical location".into()),
                };
                let mut used = BTreeSet::new();
                for id in &team.route {
                    let link = self
                        .config
                        .connections
                        .iter()
                        .find(|link| &link.id == id)
                        .expect("checked route id");
                    if !used.insert(id) {
                        return Err("team route repeats a connection".into());
                    }
                    if room == link.from {
                        room = link.to.clone();
                    } else if room == link.to {
                        room = link.from.clone();
                    } else {
                        return Err("team route has disconnected steps".into());
                    }
                }
                if team.destination.as_ref() != Some(&room) {
                    return Err("team route does not reach its destination".into());
                }
            }
        }
        Ok(())
    }
    /// Advance at most the supplied time, preserving an in-flight crossing.
    /// New entry observes blockers; already committed crossings may clear them.
    pub fn advance(
        &self,
        team: &mut TeamNavigation,
        destination: &str,
        dt: f32,
        blocked: &BTreeSet<String>,
    ) -> bool {
        if team.destination.as_deref() != Some(destination) {
            team.route.clear();
        }
        team.destination = Some(destination.into());
        team.blocked = false;
        let mut remaining = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        loop {
            match team.location.clone() {
                TeamLocation::External { .. } => return false,
                TeamLocation::Crossing {
                    connection,
                    from,
                    to,
                    elapsed,
                } => {
                    let Some(link) = self
                        .config
                        .connections
                        .iter()
                        .find(|link| link.id == connection)
                    else {
                        team.blocked = true;
                        return false;
                    };
                    let step = remaining.min((link.travel_secs - elapsed).max(0.0));
                    remaining -= step;
                    if elapsed + step < link.travel_secs {
                        team.location = TeamLocation::Crossing {
                            connection,
                            from,
                            to,
                            elapsed: elapsed + step,
                        };
                        return false;
                    }
                    team.location = TeamLocation::Compartment { compartment: to };
                    if team.route.first() == Some(&connection) {
                        team.route.remove(0);
                    }
                }
                TeamLocation::Compartment { compartment } => {
                    if compartment == destination {
                        team.route.clear();
                        return true;
                    }
                    if team.route.is_empty() || team.route.iter().any(|link| blocked.contains(link))
                    {
                        let Some(route) = self.route(&compartment, destination, blocked) else {
                            team.route.clear();
                            team.blocked = true;
                            return false;
                        };
                        team.route = route;
                    }
                    if remaining <= 0.0 {
                        return false;
                    }
                    let link = self
                        .config
                        .connections
                        .iter()
                        .find(|link| link.id == team.route[0])
                        .expect("compiled route");
                    let to = if link.from == compartment {
                        link.to.clone()
                    } else {
                        link.from.clone()
                    };
                    team.location = TeamLocation::Crossing {
                        connection: link.id.clone(),
                        from: compartment,
                        to,
                        elapsed: 0.0,
                    };
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "interior_tests.rs"]
mod tests;
