//! Immutable accepted route and gravity recipes. Existing layers own activation.
use super::*;
use crate::world::config::WorldConfig;
use crate::world::transit_destinations::WorldTransitTarget;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(super) struct AcceptedWorldRoute {
    pub view: TransitRouteView,
    pub binding: TransitWorldBinding,
}
#[derive(Clone, Debug, Default)]
pub(super) struct StaticTransitScope {
    /// Actual source layer for each accepted body; None is the base World.
    pub bodies: Vec<(Option<String>, TransitGravityBody)>,
    pub layers: BTreeSet<String>,
}
/// Rebuilt at the existing world materialisation boundary. No mutable physical,
/// layer, fuel, navigation or continuation state is stored in these recipes.
#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct StaticTransitScopes {
    pub(super) routes: BTreeMap<String, AcceptedWorldRoute>,
    pub(super) scopes: BTreeMap<TransitWorldBinding, StaticTransitScope>,
    pub(super) parents: BTreeMap<String, Option<String>>,
    load_order: Vec<String>,
    origins: BTreeMap<String, TransitWorldBinding>,
}
impl StaticTransitScopes {
    pub(crate) fn build(root: &WorldConfig) -> Result<Self, String> {
        #[cfg(not(target_arch = "wasm32"))]
        let reader = crate::world::load::OverlayFsReader;
        #[cfg(target_arch = "wasm32")]
        let reader = crate::world::load::WasmReader;
        Self::from_reader(root, &reader)
    }
    fn from_reader(
        root: &WorldConfig,
        reader: &dyn crate::world::load::WorldReader,
    ) -> Result<Self, String> {
        let mut result = Self::default();
        if root.transit_destinations.is_empty() {
            return Ok(result);
        }
        let mut walk =
            phoenix_content_preparation::world::transit::SupportingWorldWalk::new("", root)?;
        let mut configs = BTreeMap::new();
        while let Some(path) = walk.next_path() {
            let text = reader.read(&path).ok_or_else(|| {
                format!("accepted Transit supporting World is unavailable: {path}")
            })?;
            let config = crate::world::config::parse_world(&text)?;
            walk.accept(&path, &config)?;
            result
                .parents
                .insert(path.clone(), walk.parent_path(&path).map(str::to_owned));
            result.load_order.push(path.clone());
            configs.insert(path, config);
        }
        result.origins = phoenix_content_preparation::world::transit::origin_bindings(
            root,
            configs.iter().map(|(path, config)| (path.as_str(), config)),
        )?;
        let base_layers = root.extra_worlds.iter().cloned().collect();
        result.scopes.insert(
            TransitWorldBinding::Base,
            Self::scope(Some(root), &configs, base_layers)?,
        );
        for row in &root.transit_destinations {
            let (binding, config) = match &row.target {
                WorldTransitTarget::Base { .. } => (TransitWorldBinding::Base, root),
                WorldTransitTarget::Layer { path, .. } => {
                    let binding = TransitWorldBinding::Layer { path: path.clone() };
                    let config = configs
                        .get(path)
                        .ok_or("accepted destination World is absent")?;
                    if !result.scopes.contains_key(&binding) {
                        let mut layers = BTreeSet::new();
                        let mut pending = vec![path.clone()];
                        while let Some(path) = pending.pop() {
                            if !layers.insert(path.clone()) {
                                continue;
                            }
                            if layers.len() > 128 {
                                return Err("Transit scope exceeds supported World bound".into());
                            }
                            let config = configs
                                .get(&path)
                                .ok_or("accepted destination support is absent")?;
                            pending.extend(config.extra_worlds.iter().cloned());
                        }
                        result
                            .scopes
                            .insert(binding.clone(), Self::scope(None, &configs, layers)?);
                    }
                    (binding, config)
                }
            };
            let point = *config
                .anchors
                .get(row.target.anchor())
                .ok_or("accepted Transit arrival anchor is absent")?;
            let view = TransitRouteView {
                id: row.id.clone(),
                label: row.label.clone(),
                point,
            };
            view.validate()?;
            if result
                .routes
                .values()
                .any(|route| route.view.point[0] == point[0] && route.view.point[2] == point[2])
            {
                return Err("Transit destinations share an arrival point; use distinct finite anchors for unambiguous Navigation selection".into());
            }
            result
                .routes
                .insert(row.id.clone(), AcceptedWorldRoute { view, binding });
        }
        for scope in result.scopes.values() {
            for path in &scope.layers {
                if result
                    .parents
                    .get(path)
                    .and_then(|parent| parent.as_ref())
                    .is_some_and(|parent| !scope.layers.contains(parent))
                {
                    return Err("Transit support loader ancestry leaves its selected World scope; use independent supporting Worlds".into());
                }
            }
        }
        Ok(result)
    }
    fn scope(
        base: Option<&WorldConfig>,
        configs: &BTreeMap<String, WorldConfig>,
        layers: BTreeSet<String>,
    ) -> Result<StaticTransitScope, String> {
        let mut bodies = Vec::new();
        if let Some(base) = base {
            bodies.extend(
                placement::accepted_bodies(base)?
                    .into_iter()
                    .map(|body| (None, body)),
            );
        }
        for path in &layers {
            let config = configs
                .get(path)
                .ok_or("Transit static supporting World is absent")?;
            bodies.extend(
                placement::accepted_bodies(config)?
                    .into_iter()
                    .map(|body| (Some(path.clone()), body)),
            );
        }
        bodies.sort_by(|a, b| a.1.body_id.cmp(&b.1.body_id));
        if bodies.len() > MAX_TRANSIT_GRAVITY_BODIES
            || bodies
                .windows(2)
                .any(|pair| pair[0].1.body_id == pair[1].1.body_id)
        {
            return Err("Transit scope has excessive or ambiguous static arrival sources".into());
        }
        Ok(StaticTransitScope { bodies, layers })
    }
    pub(crate) fn static_worlds(&self) -> BTreeSet<String> {
        self.parents.keys().cloned().collect()
    }
    pub(crate) fn validate_context(
        &self,
        state: &ShipTransitDrives,
        tick: u64,
    ) -> Result<(), String> {
        if state.1 != TransitWorldBinding::Base && !self.scopes.contains_key(&state.1) {
            return Err(
                "saved Transit current World is not an accepted destination context".into(),
            );
        }
        if let Some(receipt) = &state.2 {
            let route = self
                .routes
                .get(&receipt.route)
                .ok_or("latest World arrival has no accepted route")?;
            if receipt.tick > tick
                || state.1 != route.binding
                || !state.0.contains_key(&receipt.owner)
            {
                return Err(
                    "latest World-arrival frontier contradicts current context/owner/clock".into(),
                );
            }
        }
        for (owner, order_state) in &state.0 {
            let Some(order) = order_state.phase.order() else {
                continue;
            };
            if order.original_world != TransitWorldBinding::Base
                && !self.scopes.contains_key(&order.original_world)
            {
                return Err(
                    "historical Transit order lacks its accepted original World scope".into(),
                );
            }
            if order_state.phase.active() && order.original_world != state.1 {
                return Err("active Transit original World contradicts current context".into());
            }
            if let TransitDestination::World { id, point, .. } = &order.destination {
                self.routes
                    .get(id)
                    .filter(|route| route.view.point == *point)
                    .ok_or("saved Transit World order differs from its accepted closed route")?;
                if let TransitPhase::Arrived { arrival_tick, .. } = order_state.phase {
                    let receipt = state
                        .2
                        .as_ref()
                        .ok_or("completed World arrival lacks its current-context frontier")?;
                    if arrival_tick > receipt.tick
                        || arrival_tick == receipt.tick
                            && (&receipt.owner != owner || &receipt.route != id)
                    {
                        return Err(
                            "retained World arrival contradicts latest physical arrival frontier"
                                .into(),
                        );
                    }
                }
            }
        }
        Ok(())
    }
    pub(crate) fn views(&self) -> Vec<TransitRouteView> {
        self.routes
            .values()
            .map(|route| route.view.clone())
            .collect()
    }
    pub(super) fn for_order(&self, order: &TransitOrder) -> Option<&StaticTransitScope> {
        match &order.destination {
            TransitDestination::Local { .. } => self.scopes.get(&order.original_world),
            TransitDestination::World { id, point, .. } => {
                let route = self
                    .routes
                    .get(id)
                    .filter(|route| route.view.point == *point)?;
                self.scopes.get(&route.binding)
            }
        }
    }
    pub(crate) fn binding_for_origin(&self, layer: &str) -> Result<TransitWorldBinding, String> {
        self.origins.get(layer).cloned().ok_or_else(|| {
            "layer-authored Transit current World scope is unsupported or ambiguous".into()
        })
    }
    pub(crate) fn origin_worlds(&self) -> BTreeSet<String> {
        self.origins.keys().cloned().collect()
    }
}

/// Join the one existing tracked layer queue. No extra pending/activation state.
pub(super) fn ensure_layers(
    scope: &StaticTransitScope,
    recipes: &StaticTransitScopes,
    layers: Option<&crate::world::server::WorldLayerMap>,
    pending: Option<&mut crate::world::server::PendingWorldLayerChanges>,
    may_request: bool,
) -> bool {
    use crate::world::server::WorldLayerChange;
    let ready = scope.layers.iter().all(|path| {
        layers
            .and_then(|layers| layers.0.get(path))
            .is_some_and(|layer| {
                layer.is_active && recipes.parents.get(path) == Some(&layer.loader_path)
            })
    });
    if ready || !may_request {
        return ready;
    }
    let Some(pending) = pending else {
        return false;
    };
    for path in recipes
        .load_order
        .iter()
        .filter(|path| scope.layers.contains(*path))
    {
        // The existing loader may defer a scripted parent by a frame. Queue
        // children only after actual parent activation, so no orphaned runtime
        // or impossible captured topology can precede its loader.
        if recipes
            .parents
            .get(path)
            .and_then(|parent| parent.as_ref())
            .is_some_and(|parent| {
                layers
                    .and_then(|layers| layers.0.get(parent))
                    .is_none_or(|layer| !layer.is_active)
            })
        {
            continue;
        }
        if layers.is_some_and(|layers| layers.0.contains_key(path))
            || pending.0.iter().any(|change| match change {
                WorldLayerChange::Load { path: queued, .. }
                | WorldLayerChange::AwaitingScript { path: queued, .. }
                | WorldLayerChange::DeferredApply { path: queued, .. } => queued == path,
                WorldLayerChange::Unload(_) => false,
            })
        {
            continue;
        }
        pending.0.push(WorldLayerChange::Load {
            path: path.clone(),
            loader_path: recipes.parents.get(path).cloned().flatten(),
        });
    }
    false
}
