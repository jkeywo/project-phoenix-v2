//! Authored GM Comms routes over the ordinary inbox and scripted dialogue path.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::command_admission::log::ShipKey;
use crate::comms::component::CommsHailable;
use crate::comms::server::{CommsInboxRes, CommsRuntime};
use crate::entities::spawner::{EntityName, EntitySystemHull, EntityUuid};
use crate::gm_action::{
    GmAction, GmActionJournal, GmActionKind, GmActionLog, GmActionRefusalReason,
    LocalGmActionRefusals, LoggedGmAction,
};
use crate::world::config::WorldConfig;
use crate::world::server::WorldScriptRuntime;

/// Wire/journal bounds, not scenario tuning. UTF-8 bytes are counted exactly.
pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_RECIPIENTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmCommsVisibility {
    SelectedShips,
    Fleet,
}

/// One explicitly authored root callable through ordinary Comms materialisation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmCommsHail {
    pub id: String,
    pub label: String,
    pub script_path: String,
    pub root_fn: String,
}

/// Root-world authored routing choices. No route is fabricated for old worlds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmCommsRoute {
    pub id: String,
    pub label: String,
    pub visibility: GmCommsVisibility,
    /// Existing authored entity reference names; resolved to live UUIDs.
    pub senders: Vec<String>,
    #[serde(default, rename = "hail")]
    pub hails: Vec<GmCommsHail>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum GmCommsContent {
    Literal { text: String },
    ScriptedHail { hail: String },
}

/// Complete immutable operator intent, retained verbatim by the GM journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmCommsTransmission {
    pub sender: String,
    pub route: String,
    /// Strictly UUID-sorted, unique. Even Fleet captures its actual recipients.
    pub recipients: Vec<ShipKey>,
    pub content: GmCommsContent,
}

impl GmCommsTransmission {
    pub fn valid_shape(&self) -> bool {
        let id = |s: &str| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control);
        id(&self.sender)
            && id(&self.route)
            && !self.recipients.is_empty()
            && self.recipients.len() <= MAX_RECIPIENTS
            && self.recipients.iter().all(|ship| id(&ship.0))
            && self.recipients.windows(2).all(|pair| pair[0].0 < pair[1].0)
            && match &self.content {
                GmCommsContent::Literal { text } => {
                    !text.is_empty() && text.len() <= MAX_TEXT_BYTES
                }
                GmCommsContent::ScriptedHail { hail } => id(hail),
            }
    }
}

pub fn validate_routes(routes: &[GmCommsRoute]) -> Result<(), String> {
    let valid =
        |id: &str| !id.trim().is_empty() && id.len() <= 128 && !id.chars().any(char::is_control);
    let mut ids = std::collections::BTreeSet::new();
    for route in routes {
        if !valid(&route.id)
            || !ids.insert(&route.id)
            || !valid(&route.label)
            || route.senders.is_empty()
            || route.senders.iter().any(|s| !valid(s))
        {
            return Err(format!(
                "invalid or duplicate [[gm_comms_route]] '{}'",
                route.id
            ));
        }
        let mut hails = std::collections::BTreeSet::new();
        for hail in &route.hails {
            if !valid(&hail.id)
                || !hails.insert(&hail.id)
                || !valid(&hail.label)
                || !valid(&hail.script_path)
                || !valid(&hail.root_fn)
            {
                return Err(format!("invalid or duplicate GM Comms hail '{}'", hail.id));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmCommsIdentity {
    pub id: String,
    pub name: String,
    /// Existing Fleet ordinal distinguishes hulls sharing an authored name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet_slot: Option<u32>,
}

fn alive(entity: &EntityRef<'_>) -> bool {
    entity
        .get::<EntitySystemHull>()
        .is_none_or(|hull| hull.0.total_current() > 0.0)
}

fn recipient(entity: &EntityRef<'_>) -> Option<GmCommsIdentity> {
    if !entity.contains::<crate::server_app::Ship>()
        || !entity.contains::<crate::lockstep::FleetSlotOf>()
        || !alive(entity)
    {
        return None;
    }
    let config = entity.get::<crate::ship::components::ShipConfigComponent>()?;
    config
        .0
        .system(&crate::ship::system_registry::comms_system_id())?;
    let uuid = entity.get::<EntityUuid>()?;
    Some(GmCommsIdentity {
        id: uuid.0.clone(),
        name: entity
            .get::<EntityName>()
            .map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
        fleet_slot: entity
            .get::<crate::lockstep::FleetSlotOf>()
            .map(|slot| slot.0 .0),
    })
}

fn sender(entity: &EntityRef<'_>, route: &GmCommsRoute) -> Option<GmCommsIdentity> {
    let uuid = entity.get::<EntityUuid>()?;
    let name = entity.get::<EntityName>()?;
    let comms = entity.get::<CommsHailable>()?;
    entity.get::<crate::comms::component::CommsRange>()?;
    if !alive(entity) || !route.senders.contains(&name.0) {
        return None;
    }
    Some(GmCommsIdentity {
        id: uuid.0.clone(),
        name: comms.display_name.clone().unwrap_or_else(|| name.0.clone()),
        fleet_slot: None,
    })
}

fn hail_available(script: Option<&WorldScriptRuntime>, hail: &GmCommsHail) -> bool {
    script.is_some_and(|script| {
        script
            .ast_owners
            .get(&hail.script_path)
            .is_some_and(|owners| owners.contains(&None))
            && script.asts.get(&hail.script_path).is_some_and(|ast| {
                ast.iter_functions()
                    .any(|f| f.name == hail.root_fn && f.params.len() == 1)
            })
    })
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct GmCommsParams<'w, 's> {
    world: Option<Res<'w, WorldConfig>>,
    entities: Query<'w, 's, EntityRef<'static>>,
    inbox: Option<ResMut<'w, CommsInboxRes>>,
    comms: Option<Res<'w, CommsRuntime>>,
    script: Option<ResMut<'w, WorldScriptRuntime>>,
    mint: crate::world_id::LiveMint<'w, { crate::world_id::IdNamespace::Message as usize }>,
    narrative: Option<ResMut<'w, Messages<crate::core::narrative::NarrativeEvent>>>,
}

impl GmCommsParams<'_, '_> {
    /// Revalidate the complete intent before mutating anything. Partial delivery
    /// is never a result of a stale recipient or an unsupported speaker.
    pub fn apply(&mut self, intent: &GmCommsTransmission) -> Result<(), GmActionRefusalReason> {
        use GmActionRefusalReason as Refusal;
        if !intent.valid_shape() {
            return Err(Refusal::InvalidAction);
        }
        let world = self.world.as_deref().ok_or(Refusal::WorldUnavailable)?;
        let route = world
            .gm_comms_routes
            .iter()
            .find(|r| r.id == intent.route)
            .ok_or(Refusal::UnknownCommsRoute)?;
        let mut matches = self.entities.iter().filter(|e| {
            e.get::<EntityUuid>()
                .is_some_and(|id| id.0 == intent.sender)
        });
        let entity = matches.next().ok_or(Refusal::UnavailableCommsIdentity)?;
        if matches.next().is_some() {
            return Err(Refusal::UnavailableCommsIdentity);
        }
        let identity = sender(&entity, route).ok_or(Refusal::UnavailableCommsIdentity)?;
        let from = entity
            .get::<EntityName>()
            .ok_or(Refusal::UnavailableCommsIdentity)?
            .0
            .clone();
        let mut available: Vec<_> = self
            .entities
            .iter()
            .filter_map(|e| recipient(&e))
            .map(|s| ShipKey(s.id))
            .collect();
        available.sort_by(|a, b| a.0.cmp(&b.0));
        if available.windows(2).any(|p| p[0] == p[1])
            || intent
                .recipients
                .iter()
                .any(|ship| !available.contains(ship))
            || (route.visibility == GmCommsVisibility::Fleet && intent.recipients != available)
        {
            return Err(Refusal::UnavailableCommsRecipient);
        }
        match &intent.content {
            GmCommsContent::Literal { text } => {
                let inbox = self.inbox.as_deref_mut().ok_or(Refusal::WorldUnavailable)?;
                let comms = self.comms.as_deref().ok_or(Refusal::WorldUnavailable)?;
                let mint = self.mint.as_deref().ok_or(Refusal::WorldUnavailable)?;
                for ship in &intent.recipients {
                    let id = crate::world_id::mint_live_id_with(
                        Some(mint),
                        crate::world_id::IdNamespace::Message,
                    );
                    let mut message = crate::core::messages::CommsMessage::injected(
                        id.clone(),
                        identity.id.clone(),
                        identity.name.clone(),
                        text.clone(),
                        Default::default(),
                        Vec::new(),
                        id,
                        crate::comms::server::sender_in_range_for_fleet(comms, &identity.id),
                        crate::core::messages::CommsPriority::Routine,
                    );
                    message.recipient_ship = Some(ship.clone());
                    message.literal_body = true;
                    crate::console::comms::server::deliver_comms_message(
                        message,
                        inbox,
                        self.narrative.as_deref_mut(),
                    );
                }
            }
            GmCommsContent::ScriptedHail { hail } => {
                let hail = route
                    .hails
                    .iter()
                    .find(|h| h.id == *hail)
                    .filter(|h| hail_available(self.script.as_deref(), h))
                    .ok_or(Refusal::UnavailableCommsHail)?;
                let script = self
                    .script
                    .as_deref_mut()
                    .ok_or(Refusal::UnavailableCommsHail)?;
                for ship in &intent.recipients {
                    script
                        .pending_comms_opens
                        .push(crate::comms::content::OpenCommsRequest {
                            recipient_ship: Some(ship.clone()),
                            sender_uuid: Some(identity.id.clone()),
                            from: from.clone(),
                            display_name: Some(identity.name.clone()),
                            root_fn: hail.root_fn.clone(),
                            script_path: hail.script_path.clone(),
                            ..Default::default()
                        });
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmCommsRouteOption {
    pub id: String,
    pub label: String,
    pub visibility: GmCommsVisibility,
    pub senders: Vec<GmCommsIdentity>,
    pub hails: Vec<GmCommsIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmCommsResult {
    #[serde(flatten)]
    pub result: LoggedGmAction,
    /// Original exact intent from the attributed canonical grant, including
    /// refused apply-time requests. Ingress failures have no accepted grant.
    pub transmission: Option<GmCommsTransmission>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmCommsProjection {
    pub routes: Vec<GmCommsRouteOption>,
    pub recipients: Vec<GmCommsIdentity>,
    pub max_text_bytes: usize,
    pub results: Vec<GmCommsResult>,
}

#[derive(Resource, Default)]
pub struct LastGmCommsProjection(pub Option<GmCommsProjection>);

fn unique_identities(mut rows: Vec<GmCommsIdentity>) -> Vec<GmCommsIdentity> {
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    let mut counts = std::collections::BTreeMap::new();
    for row in &rows {
        *counts.entry(row.id.clone()).or_insert(0) += 1;
    }
    rows.retain(|row| counts[&row.id] == 1);
    rows
}

pub fn publish_comms_projection(
    world: Option<Res<WorldConfig>>,
    entities: Query<EntityRef>,
    script: Option<Res<WorldScriptRuntime>>,
    journal: Res<GmActionJournal>,
    log: Res<GmActionLog>,
    refusals: Res<LocalGmActionRefusals>,
    mut last: ResMut<LastGmCommsProjection>,
    mut writer: MessageWriter<crate::console_bridge::GmCommsChanged>,
) {
    let recipients = unique_identities(entities.iter().filter_map(|e| recipient(&e)).collect());
    let routes = world
        .as_deref()
        .map(|w| {
            w.gm_comms_routes
                .iter()
                .map(|route| {
                    let senders = unique_identities(
                        entities.iter().filter_map(|e| sender(&e, route)).collect(),
                    );
                    GmCommsRouteOption {
                        id: route.id.clone(),
                        label: route.label.clone(),
                        visibility: route.visibility,
                        senders,
                        hails: route
                            .hails
                            .iter()
                            .filter(|h| hail_available(script.as_deref(), h))
                            .map(|h| GmCommsIdentity {
                                id: h.id.clone(),
                                name: h.label.clone(),
                                fleet_slot: None,
                            })
                            .collect(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let results = crate::gm_action::projected_results(GmActionKind::Comms, &log, &refusals)
        .into_iter()
        .map(|result| {
            let transmission = journal
                .grant_for(&result.operator_id, &result.correlation)
                .and_then(|grant| match &grant.action {
                    GmAction::TransmitComms { transmission } => Some(transmission.clone()),
                    _ => None,
                });
            GmCommsResult {
                result,
                transmission,
            }
        })
        .collect();
    let next = GmCommsProjection {
        routes,
        recipients,
        max_text_bytes: MAX_TEXT_BYTES,
        results,
    };
    if last.0.as_ref() != Some(&next) {
        last.0 = Some(next.clone());
        writer.write(crate::console_bridge::GmCommsChanged { payload: next });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transmission(text: &str) -> GmCommsTransmission {
        GmCommsTransmission {
            sender: "existing-speaker".into(),
            route: "private".into(),
            recipients: vec![ShipKey("ship-a".into()), ShipKey("ship-b".into())],
            content: GmCommsContent::Literal { text: text.into() },
        }
    }

    #[test]
    fn gm_comms_codec_preserves_literal_text_and_bounds_the_complete_request() {
        let intent = transmission("  server.gm.comms.heading\n🌒 {crew}<b>\t");
        let wire = serde_json::json!({ "operator_id": "gm-a", "correlation": "literal-1", "action": "transmit_comms", "transmission": intent });
        let request = crate::core::codec::decode_gm_action_request(&wire.to_string()).unwrap();
        assert_eq!(
            request.action,
            GmAction::TransmitComms {
                transmission: intent
            }
        );
        // The existing codec exposes the same postcard serialization used by
        // authoritative digests without adding a second direct dependency.
        let codec = vellum_digest::ShareCodec::new("GM-COMMS-TEST-");
        let binary = codec.encode(&request.action).unwrap();
        assert_eq!(codec.decode::<GmAction>(&binary).unwrap(), request.action);
        for text in [String::new(), "🌒".repeat(MAX_TEXT_BYTES / 4 + 1)] {
            let mut invalid = wire.clone();
            invalid["transmission"]["content"]["literal"]["text"] = serde_json::Value::String(text);
            assert!(crate::core::codec::decode_gm_action_request(&invalid.to_string()).is_none());
        }
        let mut limit = wire.clone();
        limit["transmission"]["content"]["literal"]["text"] =
            serde_json::Value::String("🌒".repeat(MAX_TEXT_BYTES / 4));
        assert!(crate::core::codec::decode_gm_action_request(&limit.to_string()).is_some());
        let mut extra = wire.clone();
        extra["transmission"]["invented_identity"] = true.into();
        assert!(crate::core::codec::decode_gm_action_request(&extra.to_string()).is_none());
        let mut duplicate = wire.clone();
        duplicate["transmission"]["recipients"] = serde_json::json!(["ship-a", "ship-a"]);
        assert!(crate::core::codec::decode_gm_action_request(&duplicate.to_string()).is_none());
        let mut unsorted = wire;
        unsorted["transmission"]["recipients"] = serde_json::json!(["ship-b", "ship-a"]);
        assert!(crate::core::codec::decode_gm_action_request(&unsorted.to_string()).is_none());
    }

    #[test]
    fn old_comms_catalogue_fields_decode_before_snapshot_format_refusal() {
        let message = crate::core::messages::CommsMessage::injected(
            "m".into(),
            "s".into(),
            "n".into(),
            "body".into(),
            Default::default(),
            Vec::new(),
            "thread".into(),
            true,
            crate::core::messages::CommsPriority::Routine,
        );
        let mut value = serde_json::to_value(&message).unwrap();
        value.as_object_mut().unwrap().remove("recipient_ship");
        value.as_object_mut().unwrap().remove("literal_body");
        let decoded: crate::core::messages::CommsMessage = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, message);
        let open = crate::comms::content::OpenCommsRequest::default();
        let mut value = serde_json::to_value(&open).unwrap();
        value.as_object_mut().unwrap().remove("recipient_ship");
        value.as_object_mut().unwrap().remove("sender_uuid");
        assert_eq!(
            serde_json::from_value::<crate::comms::content::OpenCommsRequest>(value).unwrap(),
            open
        );
        let dialogue: crate::comms::content::ScriptedDialogue = serde_json::from_value(serde_json::json!({ "script_path":"p", "origin_layer":null, "node_fn":"root", "on_pick":[] })).unwrap();
        assert_eq!(dialogue.recipient_ship, None);
    }

    #[test]
    fn recipient_and_literal_mode_are_authoritative_digest_inputs() {
        let mut message = crate::core::messages::CommsMessage::injected(
            "m".into(),
            "s".into(),
            "n".into(),
            "known.id".into(),
            Default::default(),
            Vec::new(),
            "thread".into(),
            true,
            crate::core::messages::CommsPriority::Routine,
        );
        let digest = |message| {
            let mut world = World::new();
            let mut inbox = CommsInboxRes::default();
            inbox.0.inject(message);
            world.insert_resource(inbox);
            crate::sim_digest::world_digest(&world)
        };
        let global = digest(message.clone());
        message.recipient_ship = Some(ShipKey("ship-a".into()));
        let private = digest(message.clone());
        assert_ne!(global, private);
        message.literal_body = true;
        assert_ne!(private, digest(message));
    }

    #[test]
    fn authored_routes_refuse_duplicates_and_unknown_visibility() {
        let route = GmCommsRoute {
            id: "private".into(),
            label: "label".into(),
            visibility: GmCommsVisibility::SelectedShips,
            senders: vec!["existing".into()],
            hails: Vec::new(),
        };
        assert!(validate_routes(std::slice::from_ref(&route)).is_ok());
        assert!(validate_routes(&[route.clone(), route]).is_err());
        assert!(serde_json::from_value::<GmCommsRoute>(
            serde_json::json!({"id":"a", "label":"a", "visibility":"invented", "senders":["a"]})
        )
        .is_err());
    }
}
