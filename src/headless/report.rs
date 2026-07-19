//! Run telemetry.
//!
//! The message stream is tapped at `OutboundMessage` and encoded with the same
//! [`JsonCodec`] the browser bridge uses, so what a headless run reports is the
//! real wire protocol rather than a parallel view of internal state. A test
//! asserting on this is asserting on something a player would actually receive.

use bevy::prelude::*;
use std::collections::BTreeMap;

use crate::codec::{JsonCodec, MessageCodec};
use crate::damage::DamageTier;
use crate::entity_spawner::{EntityName, EntitySystemHull};
use crate::lobby::OutboundMessage;
use crate::messages::{GamePhase, ServerMessage, ServerMessageDiscriminants};
use crate::server_app::{GameOverReason, LocalShip};
use crate::ship::state::ShipPhysics;

use super::args::{HeadlessArgs, ReportFormat};

/// Accumulates everything the exit summary needs, tick by tick.
#[derive(Resource, Default)]
pub struct RunTelemetry {
    pub ticks: u64,
    /// Count of each `ServerMessage` variant seen, keyed by variant name.
    /// `BTreeMap` so the report is byte-identical across runs.
    pub message_counts: BTreeMap<String, u64>,
    /// One JSON line per outbound message. Only populated for
    /// [`ReportFormat::Ndjson`] — at 10 Hz a minute of play is a lot of lines.
    pub stream: Vec<String>,
    pub capture_stream: bool,
}

/// `ServerMessage`'s variant name, for counting.
///
/// Taken from the `strum` discriminant rather than by scraping the encoded
/// JSON: `ServerMessage` is internally tagged (`#[serde(tag = "type")]`), so
/// the variant is a *value* inside the object, not the key, and any
/// key-scraping approach just reports `"type"` for everything.
fn variant_name(msg: &ServerMessage) -> String {
    format!("{:?}", ServerMessageDiscriminants::from(msg))
}

/// Records every outbound message. Runs in `Last` so it sees the whole tick's
/// traffic regardless of which `SimSet` produced it.
pub fn collect_outbound(
    mut telemetry: ResMut<RunTelemetry>,
    mut reader: MessageReader<OutboundMessage>,
    time: Res<Time>,
) {
    let codec = JsonCodec;
    let tick = telemetry.ticks;
    let sim_t = time.elapsed_secs_f64();
    for out in reader.read() {
        *telemetry
            .message_counts
            .entry(variant_name(&out.msg))
            .or_insert(0) += 1;
        if telemetry.capture_stream {
            let Ok(encoded) = codec.encode_server(&out.msg) else {
                continue;
            };
            telemetry.stream.push(format!(
                "{{\"tick\":{tick},\"sim_t\":{sim_t:.4},\"msg\":{encoded}}}"
            ));
        }
    }
}

/// Advances the tick counter. Separate from [`collect_outbound`] so messages
/// are attributed to the tick that produced them.
pub fn count_tick(mut telemetry: ResMut<RunTelemetry>) {
    telemetry.ticks += 1;
}

/// Final state of the player ship.
#[derive(Debug, Clone, Default)]
pub struct ShipSummary {
    pub name: Option<String>,
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    pub forward_speed: f32,
    pub hull_current: f32,
    pub hull_max: f32,
    /// Systems not at `Operational`, as `system_id -> tier`.
    pub damaged_systems: BTreeMap<String, String>,
}

/// The exit summary.
#[derive(Debug, Clone)]
pub struct RunReport {
    pub ticks: u64,
    pub sim_seconds: f64,
    pub wall_seconds: f64,
    pub ticks_per_second: f64,
    pub final_phase: String,
    pub game_over_reason: Option<String>,
    pub entity_count: usize,
    pub ship: Option<ShipSummary>,
    pub message_counts: BTreeMap<String, u64>,
}

impl RunReport {
    /// Whether this run should be treated as a failure under
    /// `--fail-on-game-over`.
    pub fn ended_in_game_over(&self) -> bool {
        self.final_phase == format!("{:?}", GamePhase::GameOver)
    }

    pub fn to_json(&self) -> String {
        let mut s = String::from("{\n");
        s.push_str(&format!("  \"ticks\": {},\n", self.ticks));
        s.push_str(&format!("  \"sim_seconds\": {:.4},\n", self.sim_seconds));
        s.push_str(&format!("  \"wall_seconds\": {:.4},\n", self.wall_seconds));
        s.push_str(&format!(
            "  \"ticks_per_second\": {:.1},\n",
            self.ticks_per_second
        ));
        s.push_str(&format!(
            "  \"speedup_vs_realtime\": {:.1},\n",
            if self.wall_seconds > 0.0 {
                self.sim_seconds / self.wall_seconds
            } else {
                0.0
            }
        ));
        s.push_str(&format!("  \"final_phase\": \"{}\",\n", self.final_phase));
        s.push_str(&format!(
            "  \"game_over_reason\": {},\n",
            match &self.game_over_reason {
                Some(r) => format!("{:?}", r),
                None => "null".to_string(),
            }
        ));
        s.push_str(&format!("  \"entity_count\": {},\n", self.entity_count));
        match &self.ship {
            None => s.push_str("  \"ship\": null,\n"),
            Some(ship) => {
                s.push_str("  \"ship\": {\n");
                s.push_str(&format!(
                    "    \"name\": {},\n",
                    match &ship.name {
                        Some(n) => format!("{:?}", n),
                        None => "null".to_string(),
                    }
                ));
                s.push_str(&format!(
                    "    \"position\": [{:.3}, {:.3}],\n    \"yaw\": {:.4},\n    \"forward_speed\": {:.3},\n",
                    ship.x, ship.z, ship.yaw, ship.forward_speed
                ));
                s.push_str(&format!(
                    "    \"hull\": [{:.1}, {:.1}],\n",
                    ship.hull_current, ship.hull_max
                ));
                s.push_str(&format!(
                    "    \"damaged_systems\": {{{}}}\n",
                    ship.damaged_systems
                        .iter()
                        .map(|(k, v)| format!("{:?}: {:?}", k, v))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                s.push_str("  },\n");
            }
        }
        s.push_str(&format!(
            "  \"message_counts\": {{{}}}\n",
            self.message_counts
                .iter()
                .map(|(k, v)| format!("{:?}: {}", k, v))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        s.push('}');
        s
    }
}

/// Read the finished world and produce the summary.
pub fn build_report(app: &mut App, args: &HeadlessArgs, wall_seconds: f64) -> RunReport {
    let telemetry = app.world().resource::<RunTelemetry>();
    let ticks = telemetry.ticks;
    let message_counts = telemetry.message_counts.clone();

    let sim_seconds = app.world().resource::<Time>().elapsed_secs_f64();
    let final_phase = format!("{:?}", app.world().resource::<State<GamePhase>>().get());
    let game_over_reason = app.world().resource::<GameOverReason>().0.clone();
    let entity_count = app.world().entities().len() as usize;

    let mut ship_q = app.world_mut().query_filtered::<(
        &ShipPhysics,
        Option<&EntityName>,
        Option<&EntitySystemHull>,
    ), With<LocalShip>>();
    let ship = ship_q.single(app.world()).ok().map(|(phys, name, hull)| {
        let mut summary = ShipSummary {
            name: name.map(|n| n.0.clone()),
            x: phys.x,
            z: phys.z,
            yaw: phys.yaw,
            forward_speed: phys.forward_speed,
            ..Default::default()
        };
        if let Some(hull) = hull {
            summary.hull_current = hull.0.total_current();
            summary.hull_max = hull.0.total_max();
            for (sid, _entry) in hull.0.iter() {
                let tier = hull.0.tier_for(sid);
                if tier != DamageTier::Operational {
                    summary
                        .damaged_systems
                        .insert(sid.0.clone(), format!("{tier:?}"));
                }
            }
        }
        summary
    });

    RunReport {
        ticks,
        sim_seconds,
        wall_seconds,
        ticks_per_second: if wall_seconds > 0.0 {
            ticks as f64 / wall_seconds
        } else {
            0.0
        },
        final_phase,
        game_over_reason,
        entity_count,
        ship,
        message_counts,
    }
    .tap_stream(app, args)
}

impl RunReport {
    /// Ndjson runs print the captured stream ahead of the summary, so a reader
    /// sees events in order and the summary last.
    fn tap_stream(self, app: &App, args: &HeadlessArgs) -> Self {
        if args.report_format == ReportFormat::Ndjson {
            for line in &app.world().resource::<RunTelemetry>().stream {
                println!("{line}");
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards against regressing to JSON key-scraping, which reported every
    /// message as `"type"` because `ServerMessage` is internally tagged.
    #[test]
    fn variant_name_reports_the_variant_not_the_serde_tag() {
        let name = variant_name(&ServerMessage::GameStarted);
        assert_eq!(name, "GameStarted");
        assert_ne!(name, "type");
    }

    #[test]
    fn report_json_is_parseable_and_carries_the_headline_numbers() {
        let report = RunReport {
            ticks: 601,
            sim_seconds: 10.0,
            wall_seconds: 0.5,
            ticks_per_second: 1202.0,
            final_phase: "InProgress".into(),
            game_over_reason: None,
            entity_count: 4,
            ship: Some(ShipSummary {
                name: Some("Alliance Cruiser".into()),
                x: 1.5,
                z: -2.5,
                hull_current: 90.0,
                hull_max: 100.0,
                damaged_systems: [("helm".to_string(), "Damaged".to_string())]
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            message_counts: [("SimState".to_string(), 100u64)].into_iter().collect(),
        };
        let json = report.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("report is not valid JSON: {e}\n{json}"));
        assert_eq!(parsed["ticks"], 601);
        assert_eq!(parsed["speedup_vs_realtime"], 20.0);
        assert_eq!(parsed["ship"]["name"], "Alliance Cruiser");
        assert_eq!(parsed["ship"]["damaged_systems"]["helm"], "Damaged");
        assert_eq!(parsed["message_counts"]["SimState"], 100);
        assert!(parsed["game_over_reason"].is_null());
    }

    #[test]
    fn report_json_is_parseable_with_no_ship() {
        let report = RunReport {
            ticks: 1,
            sim_seconds: 0.0,
            wall_seconds: 0.0,
            ticks_per_second: 0.0,
            final_phase: "GameOver".into(),
            game_over_reason: Some("hull breach".into()),
            entity_count: 0,
            ship: None,
            message_counts: BTreeMap::new(),
        };
        let parsed: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
        assert!(parsed["ship"].is_null());
        assert_eq!(parsed["game_over_reason"], "hull breach");
        assert!(report.ended_in_game_over());
    }
}
