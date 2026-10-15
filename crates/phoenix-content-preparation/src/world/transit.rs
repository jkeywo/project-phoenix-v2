//! Accepted static placement recipes, shared by activation and physical commit.
//! These are private physical inputs, not an observation publication.
use crate::entities::config::EntityConfig;
use crate::world::config::WorldEntity;
use phoenix_sim_gameplay::ship::transit::TransitGravityBody;

pub fn body_for_row(
    row: &WorldEntity,
    resolved: &EntityConfig,
) -> Result<Option<TransitGravityBody>, String> {
    let Some(policy) = &resolved.arrival_gravity else {
        return Ok(None);
    };
    phoenix_sim_gameplay::ship::transit_profile::validate_fixed_body(resolved)?;
    let name = row
        .name
        .as_ref()
        .filter(|name| {
            !name.trim().is_empty() && name.len() <= 512 && !name.chars().any(char::is_control)
        })
        .ok_or("arrival_gravity requires an explicit named static World row")?;
    let transform = row
        .transform
        .as_ref()
        .ok_or("arrival_gravity requires explicit absolute transform.position")?;
    let position = transform
        .position
        .filter(|position| position.iter().all(|value| value.is_finite()))
        .ok_or("arrival_gravity requires a finite explicit absolute position")?;
    if transform.anchor.is_some()
        || transform.relative_to.is_some()
        || transform.offset.is_some()
        || row.when.is_some()
        || row.when_predicate.is_some()
    {
        return Err("arrival_gravity does not support relative, anchored, random or conditional placement in this first static-body slice".into());
    }
    Ok(Some(TransitGravityBody {
        body_id: name.clone(),
        position,
        policy: policy.clone(),
    }))
}

#[cfg(test)]
#[path = "transit_tests.rs"]
mod tests;

/// Ephemeral declared-content traversal, shared by boot, browser discovery and
/// Workshop validation. It never activates layers or owns any simulation state.
/// Legacy extra_worlds remain one-level; selected destination graphs include
/// their declared static supporting worlds. A previously read child can be
/// promoted to transit scope without rereading, recompiling or duplicating it.
pub struct SupportingWorldWalk {
    root: String,
    pending: std::collections::VecDeque<String>,
    queued: std::collections::BTreeSet<String>,
    transit: std::collections::BTreeSet<String>,
    known: std::collections::BTreeMap<String, Vec<String>>,
    parents: std::collections::BTreeMap<String, Option<String>>,
}
impl SupportingWorldWalk {
    pub fn new(root: &str, config: &crate::world::config::WorldConfig) -> Result<Self, String> {
        let mut walk = Self {
            root: root.into(),
            pending: Default::default(),
            queued: Default::default(),
            transit: Default::default(),
            known: Default::default(),
            parents: Default::default(),
        };
        for path in &config.extra_worlds {
            walk.enqueue(path, None);
        }
        for destination in &config.transit_destinations {
            if let Some(path) = destination.target.layer_path() {
                walk.promote(path, None)?;
            }
        }
        Ok(walk)
    }
    fn enqueue(&mut self, path: &str, parent: Option<&str>) {
        if self.queued.insert(path.into()) {
            self.parents.insert(path.into(), parent.map(str::to_owned));
            self.pending.push_back(path.into());
        }
    }
    /// First declaration owns the ordinary loader ancestry. Later promotion
    /// only widens content discovery; it cannot reparent an existing layer.
    pub fn parent_path(&self, path: &str) -> Option<&str> {
        self.parents.get(path).and_then(|parent| parent.as_deref())
    }
    fn promote(&mut self, path: &str, parent: Option<&str>) -> Result<(), String> {
        let mut expand =
            std::collections::VecDeque::from([(path.to_owned(), parent.map(str::to_owned))]);
        while let Some((path, parent)) = expand.pop_front() {
            crate::world::transit_destinations::validate_layer_path(&path)?;
            if path == self.root {
                return Err(
                    "World transit cannot load the root as a layer; use a Base target".into(),
                );
            }
            if !self.transit.insert(path.clone()) {
                continue;
            }
            if self.transit.len() > 128 {
                return Err(
                    "Transit destination support exceeds the technical 128-World composition bound"
                        .into(),
                );
            }
            if let Some(children) = self.known.get(&path) {
                expand.extend(
                    children
                        .iter()
                        .map(|child| (child.clone(), Some(path.clone()))),
                );
            }
            self.enqueue(&path, parent.as_deref());
        }
        Ok(())
    }
    pub fn next_path(&mut self) -> Option<String> {
        self.pending.pop_front()
    }
    pub fn accept(
        &mut self,
        path: &str,
        config: &crate::world::config::WorldConfig,
    ) -> Result<(), String> {
        self.known.insert(path.into(), config.extra_worlds.clone());
        if self.transit.contains(path) {
            for child in &config.extra_worlds {
                self.promote(child, Some(path))?;
            }
        }
        Ok(())
    }
}

/// Resolve a selected route against the exact accepted static composition.
/// This is content preparation, not activation or a runtime location registry.
pub fn destination_findings(
    root: &super::validate::WorldSource<'_>,
    children: &[super::validate::WorldSource<'_>],
) -> Vec<super::validate::WorldFinding> {
    use super::validate::WorldFinding;
    use crate::world::transit_destinations::WorldTransitTarget;
    let mut findings = Vec::new();
    for child in children {
        if !child.config.transit_destinations.is_empty() {
            findings.push(WorldFinding::error(
                "unsupported-transit-destination", &child.path, child.toml,
                "transit_destination", "Transit destinations belong to the root World; supporting worlds cannot redefine the route catalogue".into(),
            ));
        }
    }
    for destination in &root.config.transit_destinations {
        let target = match &destination.target {
            WorldTransitTarget::Base { .. } => Some(root),
            WorldTransitTarget::Layer { path, .. } => {
                children.iter().find(|child| &child.path == path)
            }
        };
        let result = target.ok_or_else(|| "declared destination World is absent from the accepted content closure".to_owned()).and_then(|target| {
            target.config.anchors.get(destination.target.anchor())
                .filter(|point| point.iter().all(|value| value.is_finite()))
                .ok_or_else(|| format!("Transit destination '{}' requires its finite arrival anchor '{}' in its selected World", destination.id, destination.target.anchor()))
                .map(|_| ())
        });
        if let Err(message) = result {
            findings.push(WorldFinding::error(
                "invalid-transit-destination",
                &root.path,
                root.toml,
                &destination.id,
                message,
            ));
        }
    }
    // These Worlds remain resident in the first transit slice. The actual
    // existing layer owner uses one named-actor mapping; require unambiguous
    // references across this accepted route composition before any mint.
    if !root.config.transit_destinations.is_empty() {
        let mut names = std::collections::BTreeMap::<&str, &str>::new();
        for source in std::iter::once(root).chain(children) {
            for name in source
                .config
                .entities
                .iter()
                .filter_map(|row| row.name.as_deref())
            {
                if let Some(previous) = names.insert(name, &source.path) {
                    if previous != source.path {
                        findings.push(WorldFinding::error("ambiguous-transit-actor", &source.path, source.toml, name,
                            format!("resident Transit Worlds share actor reference '{name}' with '{previous}'; use distinct authored names")));
                    }
                }
            }
        }
    }
    findings
}
