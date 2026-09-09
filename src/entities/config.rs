use crate::entities::ai_declaration_manifest::AiDeclarationMode;
use crate::entities::ai_flag_hosts as ai_hosts;
use crate::regions::effects::RegionEffectsConfig;
use crate::regions::shape::RegionShape;
use serde::de::Error as SerdeError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// The fine-system AI policy schema lives in its own module (issue #1196); this
// glob re-export preserves every historical `entities::config::<name>` path,
// including the `pub(crate)` `default_evaluate_every_ticks` serde default that
// several console servers reference by that path.
pub use crate::entities::ai_policy_schema::*;

mod ai;
pub use ai::*;
mod visual;
pub use visual::*;
mod celestial;
pub use celestial::*;
mod hull;
pub use hull::*;
mod helm;
pub use helm::*;
mod weapons;
pub use weapons::*;
mod consoles;
pub use consoles::*;
mod power;
pub use power::*;
mod shields;
pub use shields::*;
mod repair;
pub use repair::*;
mod comms;
pub use comms::*;
mod sensors_navigation;
pub use sensors_navigation::*;
mod scene;
pub use scene::*;
use visual::reject_relocated_mesh_lod;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EntityConfig {
    /// Display name (top-level scalar). Informational for most entities; used
    /// by triggers/comms to identify named instances.
    ///
    /// This doubles as a MACHINE identity: a world `[[entity]] name` overrides
    /// it with the instance name (e.g. `wave_1`) so triggers can target the
    /// spawned hull. That is why it cannot also be the crew-facing SHIP name —
    /// see [`Self::display_name`].
    pub name: Option<String>,
    /// The crew-facing PROPER NAME of a ship (e.g. "AEV Phoenix"), a
    /// `strings.csv` id resolved on the client. Distinct from [`Self::name`]:
    /// a world instance name overwrites `name` for trigger targeting, but a
    /// ship's proper name is a property of the HULL, not of the spawn, so it
    /// lives in its own field that the instance name never touches. When present
    /// it is what scans, comms, the radar and the ship picker SHOW; `name` (or
    /// the instance name) remains the identity underneath. Absent for anything
    /// that is not a named ship, which shows `name` exactly as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub hull: Option<HullConfig>,
    pub collider: Option<ColliderConfig>,
    pub appearance: Option<AppearanceConfig>,
    pub helm_console: Option<HelmConsoleConfig>,
    /// Optional helm capability declaration (`[helm_capability]`).
    /// Describes vertical movement mode and impulse steering policy.
    #[serde(default)]
    pub helm_capability: Option<HelmCapabilityConfig>,
    pub weapons_console: Option<WeaponsConsoleConfig>,
    pub engineering_console: Option<EngineeringConsoleConfig>,
    pub captain_console: Option<CaptainConsoleConfig>,
    /// Comms CONSOLE config (issue #786): the hail `selector` and the
    /// dialogue-response `ai` policy. Distinct from the `comms` field below,
    /// which is the per-entity comms RANGE.
    #[serde(default)]
    pub comms_console: Option<CommsConsoleConfig>,
    pub power: Option<PowerConfigSection>,
    pub sensors_console: Option<SensorsConsoleConfig>,
    pub navigation_console: Option<NavigationConsoleConfig>,
    /// Unified shields config. Contains focus tuning at the top level and
    /// `num_facings` / `max_hp` / `regen_per_sec` / `offline_duration` in
    /// the nested `.base` sub-block. Every ship (player + NPC) reads this
    /// section — the legacy `[shields]` block was removed as part of the
    /// ship parity audit.
    pub shields_console: Option<ShieldsConsoleConfig>,
    /// Torpedo system config (player ship and any NPC ship with torpedoes).
    pub torpedoes: Option<TorpedoesConfig>,
    /// Repair team timings (travel duration, repair rate).
    pub repair: Option<RepairConfig>,
    /// Ship audio: filenames and tuning for the ambient bed, engine, blaster,
    /// phaser loop, and forcefield. Server-only playback — the host page's JS
    /// builds its audio graph from this. `None` ⇒ the ship is silent.
    #[serde(default)]
    pub audio: Option<crate::audio_config::ShipAudioConfig>,
    /// Comms range — when present, the entity can send/receive comms within
    /// this radius of the player ship.
    pub comms: Option<CommsConfig>,
    /// Asteroid field section from entity template (donut params, grid, etc.)
    pub asteroid_field: Option<AsteroidFieldConfig>,
    /// Region shape section — present for region entities.
    pub shape: Option<RegionShape>,
    /// Region effects section — present for region entities with effects.
    pub effects: Option<RegionEffectsConfig>,
    /// Infrastructure condition + capacity (issue #1025). Present for authored
    /// world furniture — skyhooks, fuel depots, transfer platforms — that
    /// degrades and is repaired over a mission and publishes named capacities.
    /// Absent for everything else, which behaves exactly as it did before this
    /// section existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infrastructure: Option<crate::infrastructure::InfrastructureConfig>,
    /// The sensor suite's scan capability (issue #1032). Present on a hull whose
    /// science station can take a reading of an external structure; absent for
    /// everything else, which can scan nothing and is refused by name if asked.
    /// The mirror image of `infrastructure`: that table says what can be *done
    /// to* an entity, this one says what an entity can *read*.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<crate::science::ScanConfig>,
    /// The moving-hazard table (issue #1347): a drift, the asset this contact is
    /// on course for, the radius inside which it strikes, and the four world
    /// flags a scenario hangs its beat on. Present on debris; absent for
    /// everything else, which carries no `DebrisThreat` component, never drifts
    /// and can never be confirmed as a threat.
    ///
    /// A third relative of `infrastructure` and `scan`: those say what can be
    /// *done to* an entity and what an entity can *read*, and this one says what
    /// it is going to *do* if nobody stops it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debris: Option<crate::debris::DebrisConfig>,
    /// The tractor beam's coupling terms (issue #1156) — range, rig offset and
    /// minimum power level. Present on a hull whose engineering seat can take a
    /// derelict under tow; absent for everything else, which carries no
    /// `TractorBeam` component and is unchanged in every way. The `[[system]]
    /// kind = "tractor"` block declares the system's identity (power group,
    /// station, damage entry); this table carries what the coupling itself is,
    /// and a hull that authors one without the other is refused by name at load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tractor: Option<crate::tractor::TractorConfig>,
    /// What being held by a tractor DOES to this entity as a TARGET (issue
    /// #1158) — follow, arrest-decline, station-keep or formation-keep. The
    /// mirror of `tractor`: that table says what a hull can do the holding
    /// with, this one says what happens to the thing held. Absent for every
    /// entity that authors nothing, which is merely held in place (station-keep)
    /// exactly as #1156 held it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_response: Option<crate::tractor::HeldResponseConfig>,
    /// The dock terms (issue #1159) — range, engage distance, approach speed,
    /// mate tolerance, undock clearance and minimum power level. Its presence
    /// opts a hull into docking: it can be docked WITH (its dock markers are read
    /// from the rig sidecar into a `DockMarkers` component), and, when paired with
    /// a `[[system]] kind = "dock"` block, it can actively dock. A hull that
    /// authors no `[dock]` table carries no dock markers and no `DockControl`, can
    /// neither dock nor be docked with, and is unchanged in every way — which is
    /// why the shipped destroyer is untouched by this slice and the probe fields
    /// dedicated hulls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<crate::dock::DockConfig>,
    /// The transfer-umbilical terms (issue #1160) — the capacity id it moves, the
    /// rate, the direction and the minimum power level. Present on a hull whose
    /// engineering seat can pass a capacity across a dock; absent for everything
    /// else, which carries no `TransferUmbilical` component and is unchanged in
    /// every way. The `[[system]] kind = "umbilical"` block declares the system's
    /// identity (power group, station, damage entry); this table carries what the
    /// flow itself is, and a hull that authors one without the other is refused by
    /// name at load. Both docked ends must carry an `[[infrastructure.capacity]]`
    /// under the umbilical's `capacity` id for anything to move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub umbilical: Option<crate::umbilical::UmbilicalConfig>,
    /// The Security-team terms (issue #1346) — how many teams the hull musters,
    /// how long they take to cross and come back, and how far they can reach.
    /// Present on a hull whose crew can send teams across; absent for everything
    /// else, which carries no `ShipSecurityTeams` component and is unchanged in
    /// every way. The `[[system]] kind = "security"` block declares the system's
    /// identity (which STATION owns it, its damage entry); this table carries what
    /// the teams themselves are, and a hull that authors one without the other is
    /// refused by name at load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<crate::security::SecurityConfig>,
    /// What a Security team may be sent HERE to do (issue #1346) — the mirror of
    /// `security`: that table says what a hull can do the work with, this one says
    /// what work this entity offers, how long each action takes, how risky and how
    /// urgent it is, and the world flag its success raises. Absent for every entity
    /// that authors nothing, which cannot be dispatched to at all — which is why
    /// every shipped hull and every existing world is untouched by this slice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_target: Option<crate::security::SecurityTargetConfig>,
    /// The rescue transporter's terms (issue #1348) — range, per-civilian
    /// duration and minimum power level. Present on a hull whose engineering seat
    /// can recover civilians from a discovered contact; absent for everything
    /// else, which carries no `Transporter` component and is unchanged in every
    /// way. The `[[system]] kind = "transporter"` block declares the system's
    /// identity (power group, station, damage entry); this table carries what the
    /// transporter itself is, and a hull that authors one without the other is
    /// refused by name at load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transporter: Option<crate::transporter::TransporterConfig>,
    /// The civilians this entity carries, to be discovered by scan and recovered
    /// by transporter (issue #1348) — the mirror of `transporter`: that table
    /// says what a hull can do the rescuing with, this one says what a contact
    /// offers. Absent for every entity that authors nothing, which carries no
    /// `CivilianRescue` component and is unchanged in every way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub civilian_rescue: Option<crate::transporter::CivilianRescueConfig>,
    /// What a controlled demolition may do HERE (issue #1350) — the fourth stage
    /// of the sequence Security's `place_charges` action opens. Present on an
    /// obstruction that can be cleared by detonating placed charges; absent for
    /// everything else, which carries no `DemolitionTarget` component and cannot
    /// be detonated at all — which is why every shipped hull and every existing
    /// world is untouched. It carries only flag names: the one whose set means
    /// "charges placed" (authored to equal this entity's `place_charges`
    /// `outcome_flag`), the one raised on any detonation, and the three the four
    /// outcomes hang their consequences off. Nothing here needs a paired
    /// `[[system]]`: `DetonateCharges` is fired through the `security` system the
    /// team was dispatched from, and the operation is furniture in the world, not
    /// a thing aboard a ship.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demolition_target: Option<crate::demolition::DemolitionConfig>,
    /// The faint world-locked lattice drawn under this hull on the viewscreen.
    /// Present only on a hull meant to be FLOWN — the grid is a motion cue for
    /// the crew looking out of their own ship, and it is only ever read off the
    /// LOCAL ship's resolved config, so an NPC copy of an authored hull still
    /// draws nothing. Absent for everything else, which renders exactly as it
    /// did before this table existed.
    ///
    /// Inert outside a rendering build: nothing in the simulation reads it, no
    /// component carries it, and `server::reference_grid` — the only reader —
    /// is registered solely under `SimPluginOptions::render`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_grid: Option<crate::reference_grid::ReferenceGridConfig>,
    /// Civilian traffic (issue #1028). Present for a hull that flies an authored
    /// `[[route]]` and can be given `hold` / `divert` / `dock` orders. Absent for
    /// everything else, which behaves exactly as it did before this section
    /// existed — a warship is not traffic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub civilian: Option<crate::civilian::CivilianConfig>,
    /// Optional faction UUID this entity belongs to.
    #[serde(default)]
    pub faction: Option<Uuid>,
    /// Optional AI behaviour controller config.
    #[serde(default)]
    pub behaviour: Option<BehaviourConfig>,
    /// Optional AI profile (aggression, sensor range).
    #[serde(default)]
    pub ai_profile: Option<AiProfileConfig>,
    /// Optional high-fidelity bubble ([`crate::ai::server::LodBubble`]).
    #[serde(default)]
    pub lod_bubble: Option<LodBubbleConfig>,
    /// Radar appearance (colour, optional radius) for the helm radar blip.
    #[serde(default)]
    pub radar_appearance: Option<RadarAppearanceConfig>,
    /// Targetability section. When absent the entity is not targetable.
    #[serde(default)]
    pub target: Option<crate::entities::target::TargetSection>,
    /// 3-D mesh definition. When present the entity receives a visual on the viewscreen.
    #[serde(default)]
    pub mesh: Option<MeshConfig>,
    /// Procedural animated star/sun visual.
    #[serde(default)]
    pub star: Option<StarConfig>,
    /// Textured planet visual. Takes precedence over `[mesh]` when both are
    /// present (`[mesh]` stays as a fallback for headless/editor contexts).
    #[serde(default)]
    pub planet: Option<PlanetConfig>,
    /// Ship class identifier (e.g. "battleship", "cruiser"). Sourced from
    /// top-level TOML `class` field.
    #[serde(default)]
    pub class: Option<String>,
    /// Unique hull identifier/registry number (e.g. "AEV-1864"). Sourced from
    /// top-level TOML `hull_id` field.
    #[serde(default)]
    pub hull_id: Option<String>,
    /// Authored power rating for this ship. Sourced from top-level TOML
    /// `power_rating` field.
    #[serde(default)]
    pub power_rating: Option<i32>,
    /// Per-ship CSS theme URL or inline stylesheet. Sourced from top-level
    /// TOML `css` field.
    #[serde(default)]
    pub css: Option<String>,
    /// Authored mass, in the game's own mass unit (issue #1154). Every entity
    /// a crew could act on — a hull, a derelict, a structure — carries a real
    /// weight rather than a zero-weight tow: this is the property mass-driven
    /// mechanics (the tractor/tow helm penalty is the first of them) are a
    /// DETERMINISTIC FUNCTION of, so it has to be authored content rather than
    /// guessed from a hull's other stats, and it has to be the same number on
    /// every host. An entity that authors nothing takes [`default_mass`]
    /// rather than `0.0` — see that function for why, and
    /// [`validate_mass`] for what an authored value is refused for.
    #[serde(default = "default_mass")]
    pub mass: f32,
    /// Renderer light sources attached to this entity.
    #[serde(default)]
    pub light: Vec<LightConfig>,
    /// Ship stations/systems/power_groups block, populated by parsing the same
    /// `[[station]]` / `[[system]]` / `[power_groups.*]` TOML blocks that
    /// the ship entity TOML uses. Every ship-like entity (player + NPCs) reads
    /// its `ShipConfig` from this field via the same code path — no
    /// entity-type-specific branches.
    #[serde(skip)]
    pub ship_config: Option<crate::ship::config::ShipConfig>,
    /// Cinematic camera config. When present the ship supports the
    /// `ViewMode::Cinematic` viewscreen mode with dynamic entity tracking.
    #[serde(default)]
    pub cinematic_camera: Option<CinematicCameraConfig>,
    /// Designer-authored shield arcs (issue #514). Populated from
    /// top-level `[[shield_arc]]` TOML blocks. When non-empty, the parser
    /// auto-synthesises a matching `[[system]]` entry per arc with
    /// `kind = "shield_arc"` and `SystemId("shield-arc-<id>")`. Consumed
    /// by the runtime path (`ShieldSystem::from_arcs`) that spawns the
    /// ship's `ShipShields` component.
    #[serde(skip)]
    pub shield_arcs: Vec<ShieldArcConfig>,
}

impl EntityConfig {
    /// A stationary, ownerless phaser platform. Unlike a behaviour-driven NPC
    /// it has no helm or doctrine, but its AI-only Tactical systems need the
    /// shared combat substrate to acquire and fire.
    pub fn is_static_point_defence(&self) -> bool {
        self.behaviour.is_none()
            && self
                .weapons_console
                .as_ref()
                .is_some_and(|weapons| !weapons.phaser_banks.is_empty())
            && self.ship_config.as_ref().is_some_and(|ship| {
                ship.systems.iter().any(|system| {
                    system.ai_only
                        && system.kind == crate::ship::system_registry::TACTICAL_RADAR_KIND
                }) && ship.systems.iter().any(|system| {
                    system.ai_only && system.kind == crate::ship::system_registry::PHASER_BANK_KIND
                })
            })
    }

    /// Parse and validate an entity TOML in the default AI-declaration mode.
    ///
    /// That mode is [`AiDeclarationMode::DEFAULT`], and since #885b stage 5d it
    /// is `Strict`: an AI-capable fine system that declares neither a policy nor
    /// an explicit idle state is a LOAD ERROR here, on every path at once. The
    /// synthesisers that used to fill the gap are gone, so an undeclared system
    /// would simply never act — see [`crate::entities::ai_declaration_manifest`].
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        Self::from_toml_in_mode(s, AiDeclarationMode::DEFAULT)
    }

    /// [`Self::from_toml`] with the AI-declaration mode chosen explicitly.
    ///
    /// The only remaining caller of [`AiDeclarationMode::Lenient`] is a test
    /// fixture that is deliberately NOT a complete hull: a snippet exercising
    /// beam colours, torpedo fields or marker resolution declares no AI and has
    /// no business authoring twenty blocks to say so. Nothing in production
    /// passes it — `ai_declaration_manifest::tests` asserts the default is
    /// `Strict`, and the spawner attaches nothing for an undeclared system
    /// either way.
    pub fn from_toml_in_mode(
        s: &str,
        ai_declarations: AiDeclarationMode,
    ) -> Result<Self, toml::de::Error> {
        let mut value: toml::Value = toml::from_str(s)?;
        // The LOD ladder moved to the model rig sidecar (issue #914). Reject a
        // leftover `[[mesh.lod]]` here, BEFORE `deny_unknown_fields` turns it
        // into a generic "unknown field `lod`" — an author who reads that
        // learns only that the key is gone, not where it went. Checked on the
        // composed document, so a fragment that still carries one is caught
        // too.
        reject_relocated_mesh_lod(&value)?;
        // Extract [[shield_arc]] blocks BEFORE stripping so we can populate
        // `EntityConfig.shield_arcs` and synthesise matching `[[system]]`
        // entries during ship-config parsing.
        let shield_arcs_value = if let Some(table) = value.as_table_mut() {
            table.remove("shield_arc")
        } else {
            None
        };
        let shield_arcs: Vec<ShieldArcConfig> = match shield_arcs_value {
            Some(toml::Value::Array(arr)) => arr
                .into_iter()
                .map(|v| v.try_into::<ShieldArcConfig>())
                .collect::<Result<_, _>>()?,
            Some(other) => {
                return Err(SerdeError::custom(format!(
                    "[[shield_arc]] must be an array of tables, got {other:?}"
                )));
            }
            None => Vec::new(),
        };

        // Extract the ship-config sections BEFORE stripping so we can parse
        // them via ShipConfig::from_toml (the same path ship entity TOMLs use).
        let ship_config_toml = if let Some(table) = value.as_table_mut() {
            let has_station = table.contains_key("station");
            let has_system = table.contains_key("system");
            let has_power_groups = table.contains_key("power_groups");
            let out = if has_station || has_system || has_power_groups {
                let mut ship_table = toml::value::Table::new();
                if let Some(v) = table.get("station").cloned() {
                    ship_table.insert("station".to_string(), v);
                }
                if let Some(v) = table.get("system").cloned() {
                    ship_table.insert("system".to_string(), v);
                }
                if let Some(v) = table.get("power_groups").cloned() {
                    ship_table.insert("power_groups".to_string(), v);
                }
                Some(toml::Value::Table(ship_table))
            } else {
                None
            };
            // Now strip so deny_unknown_fields doesn't reject.
            table.remove("stations");
            table.remove("station");
            table.remove("system");
            table.remove("power_groups");
            out
        } else {
            None
        };
        let mut config: EntityConfig = value.try_into()?;
        config.shield_arcs = shield_arcs;
        if let Some(power) = config.power.as_ref() {
            validate_power_config(power).map_err(SerdeError::custom)?;
        }
        if let Some(collider) = config.collider.as_ref() {
            validate_collider_config(collider).map_err(SerdeError::custom)?;
        }
        validate_mass(config.mass).map_err(SerdeError::custom)?;

        // Parse the ship_config sub-block via the shared ShipConfig code path.
        // If the ship declares `[[shield_arc]]` blocks, they auto-generate
        // matching `[[system]]` entries with `kind = "shield_arc"` — appended
        // to whatever the ship's TOML already declared before we re-validate.
        if !config.shield_arcs.is_empty()
            || ship_config_toml.is_some()
            || config.behaviour.is_some()
        {
            let registry = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
                .map_err(|e| {
                    serde::de::Error::custom(format!("system registry init failed: {e:?}"))
                })?;
            let kinds: Vec<&str> = registry.kinds().collect();

            let mut ship_config = if let Some(ship_toml_value) = ship_config_toml {
                let ship_toml_str = toml::to_string(&ship_toml_value).map_err(|e| {
                    serde::de::Error::custom(format!("ship_config re-serialise failed: {e}"))
                })?;
                // Parse only (no validation) so ships that declare stations
                // but no `[[system]]` blocks (relying on `[[shield_arc]]`
                // synthesis to populate systems) don't hit `EmptySystems`
                // during the initial pass. Final validation runs after
                // shield_arc synthesis below.
                toml::from_str::<crate::ship::config::ShipConfig>(&ship_toml_str).map_err(|e| {
                    serde::de::Error::custom(format!("ship_config parse failed: {e}"))
                })?
            } else {
                // No station/system/power_groups TOML at all but we do have
                // shield_arcs — synthesise a minimal ShipConfig so the
                // per-arc systems have a home. This path is used by NPC
                // ships that don't otherwise declare `[[system]]` blocks.
                crate::ship::config::ShipConfig {
                    stations: Vec::new(),
                    systems: Vec::new(),
                    power_groups: std::collections::HashMap::new(),
                    coordination_lag_secs: 2.0,
                }
            };

            // Synthesise `[[system]]` entries from `[[shield_arc]]` blocks.
            //
            // For player ships (has a `shields` station or a system with
            // kind="shields" in `systems`), each arc is owned by that
            // station with `ai_only = false`. For NPC ships (no shields
            // station and no shields system), arcs are ownerless AI-only
            // systems, matching how NPC phaser banks / power reactors work.
            // If a kind="shields" system exists, its station assignment
            // is used (allowing shields to live on e.g. Science).
            let shields_station_id = crate::core::messages::StationId("shields".into());
            let shields_system = ship_config
                .systems
                .iter()
                .find(|s| s.kind == crate::ship::system_registry::SHIELDS_KIND);
            let has_shields_station = shields_system.is_some()
                || ship_config
                    .stations
                    .iter()
                    .any(|s| s.id == shields_station_id);
            let effective_shields_station = shields_system
                .and_then(|s| s.station.clone())
                .unwrap_or(shields_station_id);
            for arc in &config.shield_arcs {
                let sid = crate::ship::system_registry::shield_arc_system_id(&arc.id).ok_or_else(
                    || SerdeError::custom(format!("shield_arc id {:?} is empty", arc.id)),
                )?;
                let mut synthesised_config = toml::value::Table::new();
                synthesised_config.insert(
                    "center_deg".into(),
                    toml::Value::Float(arc.center_deg as f64),
                );
                synthesised_config
                    .insert("width_deg".into(), toml::Value::Float(arc.width_deg as f64));
                if let Some(max_hp) = arc.max_hp {
                    synthesised_config.insert("max_hp".into(), toml::Value::Integer(max_hp as i64));
                }
                if let Some(regen) = arc.regen_per_sec {
                    synthesised_config
                        .insert("regen_per_sec".into(), toml::Value::Float(regen as f64));
                }
                if let Some(offline) = arc.offline_duration {
                    synthesised_config.insert(
                        "offline_duration".into(),
                        toml::Value::Float(offline as f64),
                    );
                }

                ship_config
                    .systems
                    .push(crate::ship::config::SystemInstanceConfig {
                        id: sid,
                        kind: crate::ship::system_registry::SHIELD_ARC_KIND.into(),
                        station: if has_shields_station {
                            Some(effective_shields_station.clone())
                        } else {
                            None
                        },
                        ai_only: !has_shields_station,
                        human_seeking: false,
                        seek_order: Vec::new(),
                        // Shield arcs are governed by the shields group as a
                        // whole through ShieldRegen; they are not an extra
                        // allocatable Operations channel.
                        power_group: None,
                        marker: None,
                        config: Some(toml::Value::Table(synthesised_config)),
                    });
            }

            // Provision a mandatory AI-only Red Alert capability for every
            // behaviour-driven NPC ship (issue #749). Behaviour-driven ships run
            // an `operate_captain_ai` loop that can raise its own Red Alert, but
            // only when the Red Alert system's control source resolves to `Ai` —
            // which requires the system to be *listed* on the ship. Authors who
            // omit an explicit `[[system]] kind = "red_alert"` block (every Harrow
            // and pirate NPC) would otherwise leave it defaulting to `Human`,
            // silently blocking the AI captain from ever going to Red Alert.
            //
            // Ownerless (`station = None`) + `ai_only = true`, mirroring how NPC
            // shield-arc / phaser / reactor systems are synthesised above and
            // satisfying `OwnerlessSystemWithoutAiOnly`. Idempotent: an explicit
            // authored red_alert system (the Alliance ships) is left untouched.
            if config.behaviour.is_some()
                && !ship_config
                    .systems
                    .iter()
                    .any(|s| s.kind == crate::ship::system_registry::RED_ALERT_KIND)
            {
                ship_config
                    .systems
                    .push(crate::ship::config::SystemInstanceConfig {
                        id: crate::core::messages::SystemId(
                            crate::ship::system_registry::RED_ALERT_SYSTEM_ID.into(),
                        ),
                        kind: crate::ship::system_registry::RED_ALERT_KIND.into(),
                        station: None,
                        ai_only: true,
                        human_seeking: false,
                        seek_order: Vec::new(),
                        power_group: None,
                        marker: None,
                        config: None,
                    });
            }

            // Re-run validation after synthesis (catches duplicate SystemIds,
            // dangling rating refs, etc.).
            if !ship_config.systems.is_empty() {
                crate::ship::config::validate(&ship_config, &kinds).map_err(|e| {
                    serde::de::Error::custom(format!(
                        "ship_config revalidate after shield_arc synthesis failed: {e:?}"
                    ))
                })?;
            }

            config.ship_config = Some(ship_config);
        }

        // Validation: region entity with effects but no shape is an error.
        if let Some(ref effects) = config.effects {
            if !effects.is_empty() && config.shape.is_none() {
                return Err(SerdeError::custom(
                    "region entity has effects but no [shape] section",
                ));
            }
        }

        // Validation: an [infrastructure] table has to describe a track that
        // can actually degrade (issue #1025). A ceiling of zero, a threshold
        // authored in points rather than fractions, an inverted hysteresis
        // band, or two capacities sharing an id are all author mistakes whose
        // only other symptom would be a structure that silently never crosses
        // anything.
        if let Some(ref infrastructure) = config.infrastructure {
            infrastructure.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [scan] table has to describe a fidelity ladder that can
        // actually answer (issue #1032). No bands at all, two bands claiming
        // the same id, ranges that do not strictly increase, an unlabelled band
        // or a reporting step outside (0, 1] are all author mistakes whose only
        // other symptom would be a science console that quietly returns nothing
        // for the rest of the mission.
        if let Some(ref scan) = config.scan {
            scan.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [debris] table has to describe a hazard that can arrive
        // (issue #1347). A non-finite drift, a negative radius, or a contact
        // that names a protected asset without a radius to reach it by are all
        // author mistakes whose only other symptom would be a rock that drifts
        // through a depot forever with nothing ever happening — the silently
        // inert hazard the [scan] check above is written against the mirror of.
        if let Some(ref debris) = config.debris {
            debris.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [tractor] table has to describe a beam that can hold
        // (issue #1156), and it has to be paired with the system that gives it
        // its identity. A zero range or zero minimum power is caught by
        // `TractorConfig::validate`; the pairing is checked here because the
        // coupling terms live in a table and the power group, station and damage
        // entry live on a `[[system]] kind = "tractor"` block — a hull that
        // authored one without the other would carry a control the crew can press
        // that grips nothing, or a system with terms nobody reads.
        if let Some(ref tractor) = config.tractor {
            tractor.validate().map_err(SerdeError::custom)?;
            let system = config
                .ship_config
                .as_ref()
                .and_then(|sc| {
                    sc.systems
                        .iter()
                        .find(|s| s.kind == crate::ship::system_registry::TRACTOR_KIND)
                })
                .ok_or_else(|| {
                    SerdeError::custom(
                        "a [tractor] table needs a matching [[system]] kind = \"tractor\" block \
                         to declare its power group, station and damage entry",
                    )
                })?;
            if system.power_group.is_none() {
                return Err(SerdeError::custom(
                    "the [[system]] kind = \"tractor\" block must declare a power_group — the \
                     tractor's power allocation is what an interruption checks",
                ));
            }
        }

        // Validation: a [transporter] table has to describe a transporter that
        // can recover (issue #1348), paired with the system that gives it its
        // identity — the tractor's shape exactly. A zero range, per-civilian
        // duration or minimum power is caught by `TransporterConfig::validate`;
        // the pairing is checked here because the terms live in a table and the
        // power group, station and damage entry live on a `[[system]] kind =
        // "transporter"` block.
        if let Some(ref transporter) = config.transporter {
            transporter.validate().map_err(SerdeError::custom)?;
            let system = config
                .ship_config
                .as_ref()
                .and_then(|sc| {
                    sc.systems
                        .iter()
                        .find(|s| s.kind == crate::ship::system_registry::TRANSPORTER_KIND)
                })
                .ok_or_else(|| {
                    SerdeError::custom(
                        "a [transporter] table needs a matching [[system]] kind = \"transporter\" \
                         block to declare its power group, station and damage entry",
                    )
                })?;
            if system.power_group.is_none() {
                return Err(SerdeError::custom(
                    "the [[system]] kind = \"transporter\" block must declare a power_group — the \
                     transporter's power allocation is what an interruption checks",
                ));
            }
        }

        // Validation: a [civilian_rescue] table has to carry someone (issue
        // #1348).
        if let Some(ref civilian_rescue) = config.civilian_rescue {
            civilian_rescue.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [held_response] table has to match its own kind (issue
        // #1158). A missing recover_per_sec on an arrest-decline, a zero-length
        // formation bearing, or a per-kind field on the wrong kind are all
        // author mistakes whose only other symptom would be a hold that arrests
        // nothing, or holds a target on top of the operator that grabbed it.
        if let Some(ref held_response) = config.held_response {
            held_response.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [dock] table has to describe a dock that can mate (issue
        // #1159), and a `kind = "dock"` SYSTEM has to be paired with a [dock]
        // table for its terms. Unlike the tractor, the pairing is one-directional:
        // a [dock] table alone makes a hull DOCKABLE (a passive berth carries its
        // dock markers but no active control), while the system is what makes it
        // an active docker — so a [dock] table without a dock system is allowed,
        // but a dock system without a [dock] table (or a power group) is a control
        // the crew can press that has no terms to run under.
        if let Some(ref dock) = config.dock {
            dock.validate().map_err(SerdeError::custom)?;
        }
        if let Some(system) = config.ship_config.as_ref().and_then(|sc| {
            sc.systems
                .iter()
                .find(|s| s.kind == crate::ship::system_registry::DOCK_KIND)
        }) {
            if config.dock.is_none() {
                return Err(SerdeError::custom(
                    "a [[system]] kind = \"dock\" block needs a matching [dock] table to declare \
                     its range and approach terms",
                ));
            }
            if system.power_group.is_none() {
                return Err(SerdeError::custom(
                    "the [[system]] kind = \"dock\" block must declare a power_group — the dock's \
                     power allocation is what an interruption checks",
                ));
            }
        }

        // Validation: a [repair.external_dispatch] table has to describe a
        // dispatch that can do something (issue #1161). A non-positive reach or
        // repair rate is an author mistake whose only other symptom would be a
        // repair-console control the crew can press that sends a team nowhere or
        // helps nobody.
        if let Some(external) = config
            .repair
            .as_ref()
            .and_then(|rc| rc.external_dispatch.as_ref())
        {
            external.validate().map_err(SerdeError::custom)?;
        }

        // Validation: an [umbilical] table has to describe a flow that can run
        // (issue #1160), and it has to be paired with the system that gives it
        // its identity. A blank capacity, a non-positive rate or a zero minimum
        // power is caught by `UmbilicalConfig::validate`; the pairing is checked
        // here for the tractor's reason — the flow terms live in a table and the
        // power group, station and damage entry live on a `[[system]] kind =
        // "umbilical"` block, so a hull that authored one without the other would
        // carry a control the crew can start that moves nothing, or a system with
        // terms nobody reads.
        if let Some(ref umbilical) = config.umbilical {
            umbilical.validate().map_err(SerdeError::custom)?;
            let system = config
                .ship_config
                .as_ref()
                .and_then(|sc| {
                    sc.systems
                        .iter()
                        .find(|s| s.kind == crate::ship::system_registry::UMBILICAL_KIND)
                })
                .ok_or_else(|| {
                    SerdeError::custom(
                        "an [umbilical] table needs a matching [[system]] kind = \"umbilical\" \
                         block to declare its power group, station and damage entry",
                    )
                })?;
            if system.power_group.is_none() {
                return Err(SerdeError::custom(
                    "the [[system]] kind = \"umbilical\" block must declare a power_group — the \
                     umbilical's power allocation is what an interruption checks",
                ));
            }
        }

        // Validation: a [security] table has to describe a capability that can
        // do something (issue #1346), and it has to be paired with the system
        // that gives it its identity. A teamless muster, a negative crossing time
        // or a zero reach is caught by `SecurityConfig::validate`; the pairing is
        // checked here for the umbilical's reason — the team terms live in a table
        // and the owning station and damage entry live on a `[[system]] kind =
        // "security"` block, so a hull that authored one without the other would
        // carry a console control that dispatches nobody, or a system with terms
        // nobody reads. No `power_group` is required: Security is people, not an
        // allocation, and a hull that wants its teams to fail with the lights may
        // still declare one.
        if let Some(ref security) = config.security {
            security.validate().map_err(SerdeError::custom)?;
            config
                .ship_config
                .as_ref()
                .and_then(|sc| {
                    sc.systems
                        .iter()
                        .find(|s| s.kind == crate::ship::system_registry::SECURITY_KIND)
                })
                .ok_or_else(|| {
                    SerdeError::custom(
                        "a [security] table needs a matching [[system]] kind = \"security\" block \
                         to declare which station owns the teams and its damage entry",
                    )
                })?;
        }
        if let Some(system) = config.ship_config.as_ref().and_then(|sc| {
            sc.systems
                .iter()
                .find(|s| s.kind == crate::ship::system_registry::SECURITY_KIND)
        }) {
            if config.security.is_none() {
                return Err(SerdeError::custom(
                    "a [[system]] kind = \"security\" block needs a matching [security] table to \
                     declare its team count, crossing times and reach",
                ));
            }
            if system.station.is_none() {
                return Err(SerdeError::custom(
                    "the [[system]] kind = \"security\" block must declare a station — Security is \
                     assigned through ship configuration, and a system nobody owns can be \
                     commanded by nobody",
                ));
            }
        }

        // Validation: a [security_target] table has to offer work a team could
        // actually be sent to do (issue #1346) — at least one action, each with a
        // positive duration, a risk inside the band the console renders, and no
        // verb authored twice (the second could never be reached, because a
        // dispatch names one action id and the first match answers).
        if let Some(ref security_target) = config.security_target {
            security_target.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [demolition_target] table has to name five distinct,
        // non-blank world flags (issue #1350), so a detonation cannot silently
        // fire the wrong outcome's consequence or hang off a flag that names
        // nothing. Checked by `DemolitionConfig::validate`.
        if let Some(ref demolition_target) = config.demolition_target {
            demolition_target.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [reference_grid] table has to describe a lattice that
        // can actually be drawn and read. A spacing of zero, a major spacing
        // that is not a whole multiple of the minor one, a fade band wider than
        // the patch it fades, or an over-range colour that would bloom on an
        // HDR viewscreen are all author mistakes whose only other symptom would
        // be a grid that is invisible, doubled, or louder than the ships.
        if let Some(ref reference_grid) = config.reference_grid {
            reference_grid.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [civilian] table has to name a lane something can fly
        // (issue #1028). An empty route id, a negative priority or a disposition
        // authored with negative delays are all author mistakes whose only other
        // symptom would be traffic that sits still and never answers.
        if let Some(ref civilian) = config.civilian {
            civilian.validate().map_err(SerdeError::custom)?;
        }

        // Validation: a [radar_appearance] table must declare at least one
        // of icon/region_colour. An empty table is always an author mistake
        // (omit the whole section to mean "don't show on radar").
        if let Some(ref ra) = config.radar_appearance {
            if ra.icon.is_none() && ra.region_colour.is_none() {
                return Err(SerdeError::custom(
                    "[radar_appearance] must set icon and/or region_colour",
                ));
            }
        }

        // Validate an authored inline Captain AI policy before world
        // activation (issue #775). The Captain's single AI-capable fine system
        // (Red Alert) drives one output channel with one verb. Structural,
        // expression, channel, verb, and parameter errors are deterministic
        // content errors surfaced through serde so the entity fails to load.
        //
        // Every validator below is the `_for` variant, naming the host whose
        // runtime evaluation the block feeds (issue #891 stage 1): that is what
        // lets a `flag(...)`/`counter(...)` guard be rejected on the sixteen
        // hosts that evaluate with an empty flag chain, instead of parsing,
        // validating, and then reading false for ever. The bare
        // `validate_fine_system_ai_*` entry points are host-less and must NOT be
        // used here — `production_validation_names_its_host` asserts that.
        if let Some(ai) = config.captain_console.as_ref().and_then(|c| c.ai.as_ref()) {
            validate_fine_system_ai_policy_for(
                &ai_hosts::CAPTAIN_RED_ALERT,
                ai,
                &[CAPTAIN_RED_ALERT_CHANNEL],
                &[CAPTAIN_SET_RED_ALERT_VERB],
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate authored inline Engines/Steering AI policies before world
        // activation (issue #779). These are the first continuous fine
        // actuators on the #775 spine: Engines drives the `longitudinal`
        // channel, Steering the `yaw` channel, each with its own single mode
        // verb. Unknown channels/verbs, unparseable guards, and undeclared
        // parameter references fail the entity load here, before any live tick.
        if let Some(hc) = config.helm_console.as_ref() {
            if let Some(ai) = hc.engines_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_ENGINES,
                    ai,
                    &[HELM_LONGITUDINAL_CHANNEL],
                    &[HELM_ACTUATE_DESIRED_TRAVEL_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.steering_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_STEERING,
                    ai,
                    &[HELM_YAW_CHANNEL],
                    HELM_STEERING_VERBS,
                )
                .map_err(SerdeError::custom)?;
                validate_helm_steering_param_sets(ai).map_err(SerdeError::custom)?;
            }
            // Secondary helm fine-actuator policies (issue #780): each drives its
            // own single channel with its own single mode verb. Wrong-axis verbs,
            // unknown channels, unparseable guards, and undeclared parameter
            // references fail the entity load here, before any live tick.
            if let Some(ai) = hc.lateral_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_LATERAL,
                    ai,
                    &[HELM_LATERAL_CHANNEL],
                    &[HELM_ACTUATE_LATERAL_THRUST_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.vertical_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_VERTICAL,
                    ai,
                    &[HELM_VERTICAL_CHANNEL],
                    &[HELM_ACTUATE_VERTICAL_THRUST_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.impulse_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_IMPULSE,
                    ai,
                    &[HELM_IMPULSE_CHANNEL],
                    &[HELM_ENGAGE_IMPULSE_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
            if let Some(ai) = hc.boost_ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::HELM_BOOST,
                    ai,
                    &[HELM_BOOST_CHANNEL],
                    &[HELM_ENGAGE_BOOST_VERB],
                )
                .map_err(SerdeError::custom)?;
            }
        }

        // Validate authored inline per-bank weapon AI policies before world
        // activation (issue #781). Each AI-capable phaser and blaster bank may
        // declare an inline `ai` block driving its single `phaser_fire` /
        // `blaster_fire` channel with its single fire verb. Unknown
        // channels/verbs, unparseable guards, and undeclared parameter
        // references fail the entity load here, before any live tick — mirroring
        // the helm validation block above.
        if let Some(wc) = config.weapons_console.as_ref() {
            for bank in &wc.phaser_banks {
                if let Some(ai) = bank.ai.as_ref() {
                    validate_fine_system_ai_policy_for(
                        &ai_hosts::PHASER_BANK,
                        ai,
                        PHASER_BANK_CHANNELS,
                        PHASER_BANK_VERBS,
                    )
                    .map_err(SerdeError::custom)?;
                }
            }
            for bank in &wc.blaster_banks {
                if let Some(ai) = bank.ai.as_ref() {
                    validate_fine_system_ai_policy_for(
                        &ai_hosts::BLASTER_BANK,
                        ai,
                        BLASTER_BANK_CHANNELS,
                        BLASTER_BANK_VERBS,
                    )
                    .map_err(SerdeError::custom)?;
                }
            }
            // The ship-level WEAPONS DOCTRINE (issue #956), validated here with
            // the per-bank policies rather than down among the target selectors:
            // it is a `[weapons_console.ai]` POLICY, and it belongs beside the
            // other weapons-console policies its host resolves alongside. Its
            // channels are the three arc-bearing ranks and its verbs the three
            // weapon families; a rule on an unknown rank, a misspelled family,
            // an unparseable guard or an undeclared `param(...)` fails the
            // entity load here rather than resolving to a silent "no family
            // qualifies" for the rest of the ship's life.
            if let Some(ai) = wc.ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::WEAPONS_DOCTRINE,
                    ai,
                    ARC_BEARING_CHANNELS,
                    WEAPONS_DOCTRINE_VERBS,
                )
                .map_err(SerdeError::custom)?;
            }
        }

        // Validate authored inline torpedo tube + magazine AI policies before
        // world activation (issue #782). Each AI-capable tube may declare an
        // inline `ai` block driving its `torpedo_load` / `torpedo_launch`
        // channels; the shared magazine may declare its own `ai` block driving
        // the `torpedo_magazine_grant` channel. Unknown channels/verbs (a launch
        // verb on the magazine channel, a grant verb on a tube), unparseable
        // guards, and undeclared parameter references fail the entity load here,
        // before any live tick — mirroring the weapon-bank validation above.
        if let Some(tc) = config.torpedoes.as_ref() {
            for tube in &tc.tubes {
                if let Some(ai) = tube.ai.as_ref() {
                    validate_fine_system_ai_policy_for(
                        &ai_hosts::TORPEDO_TUBE,
                        ai,
                        TORPEDO_TUBE_CHANNELS,
                        TORPEDO_TUBE_VERBS,
                    )
                    .map_err(SerdeError::custom)?;
                }
            }
            if let Some(ai) = tc.ai.as_ref() {
                validate_fine_system_ai_policy_for(
                    &ai_hosts::TORPEDO_MAGAZINE,
                    ai,
                    TORPEDO_MAGAZINE_CHANNELS,
                    TORPEDO_MAGAZINE_VERBS,
                )
                .map_err(SerdeError::custom)?;
            }
        }

        // Validate an authored inline Shields focus AI policy before world
        // activation (issue #783). The Shields fine system drives its single
        // `shield_focus` channel with its single value-less `focus_shield_arc`
        // verb. A wrong-axis verb (e.g. a fire verb), an unknown channel, an
        // unparseable guard, and undeclared `param(...)` references fail the
        // entity load here, before any live tick — mirroring the helm/weapon
        // validation blocks above.
        if let Some(ai) = config
            .shields_console
            .as_ref()
            .and_then(|sc| sc.ai_policy.as_ref())
        {
            validate_fine_system_ai_policy_for(
                &ai_hosts::SHIELDS_FOCUS,
                ai,
                SHIELD_FOCUS_CHANNELS,
                SHIELD_FOCUS_VERBS,
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Power allocation policy before world
        // activation (issue #784). Unlike every fine system above, the Power
        // reactor has NO fixed channel catalogue: its output channels are the
        // ship's AUTHORED `[power_groups.*]` keys, so the valid-channel set is
        // built dynamically from ship data here (AC1 "no fixed system
        // catalogue"). The single verb is the value-carrying
        // `set_power_group_allocation`. A rule targeting a non-authored group,
        // a wrong verb, an unparseable guard, or an undeclared `param(...)`
        // reserve fails the entity load here, before any live tick.
        if let Some(ai) = config.power.as_ref().and_then(|p| p.ai_policy.as_ref()) {
            // …and where the hull authors NO `[power_groups.*]` at all, the
            // channel set is the canonical trio the runtime seeds it with.
            // `PowerSystem::from_authored_groups` falls back to
            // `seeded_with_defaults` (helm / weapons / sensors at level 2) for
            // an empty authored map, and `ai_power_allocation` then resolves the
            // policy against exactly those groups — so validating against an
            // empty set would reject a policy the runtime is about to run.
            // Nothing shipped hit this until #885b made every hull author
            // `[power.ai_policy]`, including the six NPC hulls that declare no
            // power groups.
            let authored_channels: Vec<&str> = config
                .ship_config
                .as_ref()
                .map(|sc| sc.power_groups.keys().map(|g| g.0.as_str()).collect())
                .unwrap_or_default();
            let valid_channels: Vec<&str> = if authored_channels.is_empty() {
                crate::modifiers::power_system::POWER_GROUP_ORDER.to_vec()
            } else {
                authored_channels
            };
            validate_fine_system_ai_policy_for(
                &ai_hosts::POWER_ALLOCATION,
                ai,
                &valid_channels,
                &[POWER_SET_ALLOCATION_VERB],
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Sensors target selector before world
        // activation (issue #776). Unknown sources, unparseable
        // eligibility/score expressions, and undeclared `param(...)` references
        // are deterministic content errors surfaced through serde so the entity
        // fails to load before any live tick evaluates it.
        if let Some(sel) = config
            .sensors_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector_for(
                &ai_hosts::SENSORS_SELECTOR,
                sel,
                SENSORS_SELECTOR_SOURCES,
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Tactical target selector before world
        // activation (issue #777). Same deterministic content-error surface as
        // the Sensors selector above — the Tactical host is the sole writer of
        // the authoritative weapons target, so a malformed ranking must fail
        // the entity load rather than reach a live tick.
        if let Some(sel) = config
            .weapons_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector_for(
                &ai_hosts::TACTICAL_SELECTOR,
                sel,
                TACTICAL_SELECTOR_SOURCES,
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Navigation target selector before world
        // activation (issue #778). Same deterministic content-error surface as
        // the Sensors/Tactical selectors above — the Navigation host emits the
        // authoritative waypoint through admission from this ranking, so a
        // malformed selector must fail the entity load rather than reach a live
        // tick.
        if let Some(sel) = config
            .navigation_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector_for(
                &ai_hosts::NAVIGATION_SELECTOR,
                sel,
                NAVIGATION_SELECTOR_SOURCES,
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate an authored inline Repair target selector before world
        // activation (issue #785). Same deterministic content-error surface as
        // the Sensors/Tactical/Navigation selectors above — `operate_repair_ai`
        // emits admitted `DispatchRepairTeam` inputs from this ranking, so a
        // malformed selector must fail the entity load rather than reach a live
        // tick. `[repair.selector]` is the first selector block outside a
        // `*_console` section.
        if let Some(sel) = config.repair.as_ref().and_then(|c| c.selector.as_ref()) {
            validate_fine_system_ai_selector_for(
                &ai_hosts::REPAIR_SELECTOR,
                sel,
                REPAIR_SELECTOR_SOURCES,
            )
            .map_err(SerdeError::custom)?;
        }

        // Validate the authored Comms console AI blocks before world activation
        // (issue #786). Comms is the first system to author BOTH machines, so
        // both validators run: the hail SELECTOR against its registered
        // candidate sources, and the dialogue-response POLICY against the single
        // `comms_respond` channel and its `respond_to_message` verb. Both emit
        // admitted commands into the shared comms router, so a malformed block
        // must fail the entity load rather than reach a live tick.
        if let Some(sel) = config
            .comms_console
            .as_ref()
            .and_then(|c| c.selector.as_ref())
        {
            validate_fine_system_ai_selector_for(
                &ai_hosts::COMMS_SELECTOR,
                sel,
                COMMS_SELECTOR_SOURCES,
            )
            .map_err(SerdeError::custom)?;
        }
        if let Some(ai) = config.comms_console.as_ref().and_then(|c| c.ai.as_ref()) {
            validate_fine_system_ai_policy_for(
                &ai_hosts::COMMS_RESPONSE,
                ai,
                COMMS_RESPOND_CHANNELS,
                COMMS_RESPOND_VERBS,
            )
            .map_err(SerdeError::custom)?;
        }

        // Reject a doctrine entry whose `directive_*` fields do not match its
        // own `directive_kind` — see `validate_doctrine_directives`. Runs on the
        // same surface as the selector/policy validators above, so a world
        // `spawn_entity` override that authors a mismatched directive fails at
        // load rather than resolving to a directive that can never fire.
        //
        // ── Overrides merge per-field, so flipping a kind can trip this ──
        //
        // An override reaches this check already merged: `merge_keyed_array`
        // (`src/entities/entity_override.rs`) deep-merges a doctrine override
        // into the template entry that shares its `id`, field by field. Change
        // an existing entry's `directive_kind` and the template's directive
        // fields come along with it — overriding `ship_harrow_patrol.toml`'s
        // Patrol entry with `{ id = "patrol-ironveil", directive_kind = "Reach",
        // directive_anchor = "x" }` yields a merged entry carrying BOTH
        // `directive_anchors` (from the template) and `directive_anchor`, which
        // this check rejects.
        //
        // The escape hatch is to clear the stale field inside the same override
        // entry — `directive_anchors = []`. That still works after issue #911:
        // `behaviour.doctrine.directive_anchors` is in neither identity table,
        // so a nested array inside a reconciled entry keeps replacing wholesale
        // at both merge layers.
        //
        // That matters most on the `spawn_entity` path, where the rejection is
        // NOT fatal: `world::dispatch::dispatch_spawn_entity` warns and keeps
        // the template, so an author who misses this gets the very doctrine they
        // were trying to replace — the silent-wrong-doctrine failure mode #838
        // set out to end. Nothing shipped is in this shape.
        if let Some(ref b) = config.behaviour {
            validate_doctrine_directives(&b.doctrine).map_err(SerdeError::custom)?;
        }

        // Clamp target_speed in every doctrine entry.
        if let Some(ref mut b) = config.behaviour {
            for d in &mut b.doctrine {
                d.target_speed = d.target_speed.clamp(0.0, 1.0);
            }
        }

        // Reject an AI-capable fine system that declares NEITHER a policy nor an
        // explicit idle state (PRD #774 US7), when strict mode is on.
        //
        // Runs last, after every `if let Some(ai) = ...` validator above, and it
        // is deliberately the mirror image of them: those check what an author
        // DID write, this one checks what they did not. Until #885b flips
        // `AiDeclarationMode::DEFAULT` the branch never runs on a shipped path,
        // and the nineteen synthesisers keep filling the gap exactly as before.
        if ai_declarations == AiDeclarationMode::Strict {
            if let Some(err) = crate::entities::ai_declaration_manifest::strict_error(&config) {
                return Err(SerdeError::custom(err));
            }
        }

        Ok(config)
    }

    /// Serialize this config to a `toml::Value` **losslessly** — re-emitting the
    /// `[[station]]` / `[[system]]` / `[power_groups]` and `[[shield_arc]]`
    /// blocks that [`from_toml`](Self::from_toml) assembles into the
    /// `#[serde(skip)]` [`ship_config`](Self::ship_config) /
    /// [`shield_arcs`](Self::shield_arcs) fields.
    ///
    /// # Why this exists (issue #838)
    ///
    /// The override-merge path (`entity_loader::resolve_entity_via` and
    /// `world::dispatch::dispatch_spawn_entity`) resolves a `spawn_entity` /
    /// `[[entity]]` override by round-tripping the template through TOML:
    /// `template → toml::Value → merge(overrides) → EntityConfig::from_toml`.
    /// A plain `toml::to_string(&config)` drops `ship_config` and `shield_arcs`
    /// because both are `#[serde(skip)]` (they have no serialized representation
    /// — `from_toml` reconstructs them from the raw blocks at parse time). The
    /// merged string therefore carried **no ship systems at all**, and the
    /// re-parsed config spawned a hull with zero stations, zero weapons, and
    /// nothing under AI control: a world-spawned "hostile" that could never lock
    /// a target or fire. Re-emitting the blocks here makes the round-trip
    /// faithful, so an override preserves the template's whole system suite.
    ///
    /// Synthesized `shield_arc` systems are filtered out of the emitted `system`
    /// array on purpose: `from_toml` re-synthesizes exactly one per
    /// `[[shield_arc]]` block, and emitting both would trip `DuplicateSystemId`.
    pub fn to_toml_value(&self) -> Result<toml::Value, toml::ser::Error> {
        let mut value = toml::Value::try_from(self)?;
        let table = value
            .as_table_mut()
            .expect("EntityConfig always serializes to a TOML table");

        if let Some(ship_config) = &self.ship_config {
            if !ship_config.stations.is_empty() {
                table.insert(
                    "station".to_string(),
                    toml::Value::try_from(&ship_config.stations)?,
                );
            }
            let declared_systems: Vec<&crate::ship::config::SystemInstanceConfig> = ship_config
                .systems
                .iter()
                .filter(|s| s.kind != crate::ship::system_registry::SHIELD_ARC_KIND)
                .collect();
            if !declared_systems.is_empty() {
                table.insert(
                    "system".to_string(),
                    toml::Value::try_from(&declared_systems)?,
                );
            }
            if !ship_config.power_groups.is_empty() {
                table.insert(
                    "power_groups".to_string(),
                    toml::Value::try_from(&ship_config.power_groups)?,
                );
            }
        }

        if !self.shield_arcs.is_empty() {
            table.insert(
                "shield_arc".to_string(),
                toml::Value::try_from(&self.shield_arcs)?,
            );
        }

        Ok(value)
    }
}

fn validate_power_config(power: &PowerConfigSection) -> Result<(), String> {
    let minimum = power
        .max_commanded_total
        .checked_sub(power.rates.len().saturating_sub(1) as u8)
        .ok_or_else(|| "power.max_commanded_total is too small for its rates ladder".to_string())?;
    if power.sustainable_total < minimum || power.sustainable_total > power.max_commanded_total {
        return Err(format!(
            "power.sustainable_total {} must fall within the rates ladder {}..={}",
            power.sustainable_total, minimum, power.max_commanded_total
        ));
    }
    for (offset, rate) in power.rates.iter().enumerate() {
        let total = minimum + offset as u8;
        if total <= power.sustainable_total && *rate < 0.0 {
            return Err(format!(
                "power rate at total {total} drains below sustainable_total {}",
                power.sustainable_total
            ));
        }
        if total > power.sustainable_total && *rate >= 0.0 {
            return Err(format!(
                "power rate at total {total} must drain above sustainable_total {}",
                power.sustainable_total
            ));
        }
    }
    Ok(())
}

/// Reject a `[collider]` whose numbers cannot describe the shape it names.
///
/// Only [`ColliderShape::Cylinder`] has anything to check, and the check is the
/// one that matters: a cylinder with no `half_height` is a zero-thickness disc
/// that nothing can ever be inside, which is exactly the pass-through bug the
/// station-collider correction was fixing. Serde cannot catch it — the field is
/// optional so that every Ball and Capsule already on disk parses unchanged —
/// so it is caught here, at load, in the same place and the same style as
/// [`validate_power_config`].
///
/// Ball and Capsule are deliberately left alone. Their fields were never
/// validated (a `radius = 0` Ball has always been authorable), and starting now
/// would reject templates that load today for reasons this change has nothing
/// to do with.
fn validate_collider_config(collider: &ColliderConfig) -> Result<(), String> {
    if collider.shape == ColliderShape::Cylinder {
        match collider.half_height {
            None => {
                return Err(
                    "collider.half_height is required for shape = \"Cylinder\" — a cylinder \
                     with no half-height is a zero-thickness disc nothing can collide with"
                        .to_string(),
                )
            }
            // NaN spelled out rather than left to `!(h > 0.0)`: it is the same
            // failure as zero — a body rapier cannot make sense of — and it
            // arrives from the same place, an author typing a number wrong.
            Some(h) if h.is_nan() || h <= 0.0 => {
                return Err(format!(
                    "collider.half_height must be positive for shape = \"Cylinder\", got {h}"
                ))
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// Documented parse-time default for [`EntityConfig::mass`] (issue #1154): the
/// weight an entity that authors no `mass` is given, rather than `0.0`.
///
/// A zero-mass tow is a physics-defeating exploit dressed as an unauthored
/// field, not an empty one — so the fallback has to be a real weight, and it
/// has to be a NUMBER, chosen once here, rather than a per-caller guess that
/// could disagree with itself between the spawner and a scan readout. `10_000`
/// sits mid-ladder between the lightest shipped hull (a courier) and the
/// heaviest (a battleship), so a template nobody has tuned yet behaves like an
/// ordinary mid-weight hull under a mass-driven mechanic rather than like
/// nothing (too light) or like a battleship (too heavy).
pub const DEFAULT_ENTITY_MASS: f32 = 10_000.0;

fn default_mass() -> f32 {
    DEFAULT_ENTITY_MASS
}

/// Reject an authored `mass` that could never be a real weight.
///
/// Non-positive and non-finite are the same author mistake wearing different
/// faces — a stray `0`, a typo'd negative, a `nan`/`inf` TOML can spell out
/// directly — and every one of them would hand a mass-driven mechanic (the
/// tow/tractor helm penalty divides by this number) either a divide-by-zero or
/// a constant that always wins or always loses, rather than an authored
/// weight. Caught here, at load, in the same style as
/// [`validate_collider_config`], so the failure names the file rather than
/// surfacing as a silent NaN three systems downstream.
fn validate_mass(mass: f32) -> Result<(), String> {
    if !mass.is_finite() || mass <= 0.0 {
        return Err(format!(
            "mass = {mass} is not a valid weight — mass must be a positive, finite number"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
