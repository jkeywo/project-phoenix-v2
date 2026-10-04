//! What the lobby's **monitor row** and its **per-station screen rows** are, in
//! both directions (issues #1330, #1331) — **pure, Bevy-free**.
//!
//! [`super::super::bridge_layout`] is the law: what a bridge arrangement may
//! become. This is the *conversation about it* the viewscreen's own lobby
//! surface has:
//!
//! ```text
//! BridgeLayout + DiscoveredMonitor[] ──bridge_layout_payload──▶  BridgeLayoutPayload
//!        ▲                                                            │ codec::to_json
//!        │                                                            ▼
//!  LayoutAction ◀──*_action────────────┐              window.__phoenixHostLobbyLayout(json)
//!                                      │                         gui/host-lobby-view.js
//!    HostLobbyRecord::{SetViewscreen, AssignStation, UnassignStation}
//!                    [super::scenario]  ▲ codec::from_json
//!                                       └──────────────  phoenixHostLobbyOut.send
//! ```
//!
//! # Two rows, one payload
//!
//! The monitor row (issue #1330) chooses which display shows the shared
//! viewscreen; a station's screen row (issue #1331) chooses which display that
//! station's console opens on, or closes it. They travel together because they
//! are two projections of the same [`BridgeLayout`] and the surface renders
//! them in one pass — a press on either is judged by the same law, and a
//! payload that carried only one of them would let the two disagree about which
//! screen is holding what.
//!
//! Both ends are here because they are one contract: the `identity` string a
//! button carries out is the same `identity` string the press carries back, and
//! splitting the two halves across two modules is how a renamed field becomes a
//! button that silently does nothing.
//!
//! # …but the records themselves are not, and that is deliberate
//!
//! All three presses arrive as variants of
//! [`HostLobbyRecord`](super::HostLobbyRecord), the surface's ONE vocabulary,
//! beside the operator's scenario and hull picks and their AI-launch press
//! (issue #1328). They are not a record type of their own, because
//! `HostLobbyBridge::take_records` is a **drain**: a second record type would
//! want a second reader, the first reader to run would swallow the other's
//! records and warn about a vocabulary it does not speak, and the second would
//! see an empty queue forever — with a clean log on both sides. One vocabulary,
//! one drain. What stays here is the half that is genuinely this module's: the
//! [`LayoutAction`] each of `set-viewscreen`, `assign-station` and
//! `unassign-station` asks for.
//!
//! There is no `move-station` verb, because the law has no move action: naming
//! a different screen for a station that is already seated *is* the move. The
//! row therefore presses one button either way and cannot pick the wrong verb.
//!
//! # Why the payload is `serde` and not a `format!`
//!
//! [`super::super::panes::os_prefs::os_defaults_script`] hand-formats its JSON
//! and says exactly why it is allowed to: every field is a `bool` or a clamped
//! number, so none can carry a quote, a backslash or a `</script`. **This
//! payload carries strings the OS chose** — a monitor's own name — so it is the
//! case that note explicitly rules out. It is therefore `serde`-derived and
//! encoded through the one `serde_json` seam
//! ([`crate::core::codec`], AGENTS.md Key Constraint 1), and pushed with
//! `vellum_ultralight::bridge::push_call`, which escapes the JavaScript string
//! literal it lands in.
//!
//! # No English here
//!
//! Nothing in this module composes a sentence. A refusal and an adoption note
//! cross as a `strings.csv` id plus its parameters
//! ([`LayoutRefusal::string_id`](super::super::bridge_layout::LayoutRefusal::string_id)),
//! and a monitor crosses as its name, its size and two flags — so the row's
//! words are `gui/strings.js`'s, exactly like every other player-visible string
//! on this surface (AGENTS.md rule 11).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::messages::StationId;

use super::super::bridge_layout::{
    BridgeLayout, ExclusionReason, LayoutAction, LayoutAdoption, LayoutRefusal, MonitorChoice,
};
use super::super::bridge_profile::{DiscoveredMonitor, MonitorIdentity};

/// One monitor, as one button in the lobby's monitor row.
///
/// Carries **data, never prose**: the row's words are composed on the page from
/// `server.monitor_row.*`, so a monitor with no OS name is `name: None` rather
/// than a Rust-side "Unnamed display".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorButtonPayload {
    /// The monitor's stable identity — the token a press carries back, so the
    /// host resolves the button to the same display the operator looked at.
    pub identity: String,
    /// The OS-reported display name. Absent when the OS gave none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Native resolution, physical pixels.
    pub width: u32,
    pub height: u32,
    /// Whether the OS calls this the primary display.
    pub primary: bool,
    /// Whether this monitor is showing the shared viewscreen right now.
    pub viewscreen: bool,
    /// The consoles open on it, if any — the reason a press may be refused,
    /// carried so the row can say so *before* the press rather than only after.
    ///
    /// Everything the law counts as an occupant, not only the seats: a station
    /// this layout placed, and the label of a console a hand-authored
    /// `--profile` opened that the layout does not own
    /// ([`BridgeLayout::reserved_on`](super::super::bridge_layout::BridgeLayout::reserved_on)).
    /// The two are one list here because the button is answering one question —
    /// what is on this screen — and the refusal a press would earn names them
    /// together too.
    ///
    /// In the order they are **drawn** on that screen
    /// ([`BridgeLayout::occupants_on`](super::super::bridge_layout::BridgeLayout::occupants_on)),
    /// so a person looking at the glass reads the button the way the glass reads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stations: Vec<String>,
    /// Which of `stations` the **lobby cannot free** — the consoles a
    /// hand-authored `--profile` opened
    /// ([`BridgeLayout::reserved_on`](super::super::bridge_layout::BridgeLayout::reserved_on)),
    /// which have no station row, no off button and no way back short of
    /// restarting the host with different arguments (issue #1332's fix round).
    ///
    /// A subset of `stations`, not a second list of a different kind of thing:
    /// the button answers "what is on this screen" with one list, and this says
    /// which of those entries an operator pressing around the lobby will never
    /// find a control for. Without it a `--pane` participant's console reads as
    /// an ordinary occupant of a full screen, and the operator hunts the station
    /// rows for the unassign button that would free it. There is none, and since
    /// issue #1332 that console costs the screen a real slot — so saying so is
    /// the difference between a screen an operator can act on and one they
    /// cannot.
    ///
    /// Empty on every host without a `--profile` that authors a participant
    /// pane, which is every host but that one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reserved: Vec<String>,
}

/// One sentence the lobby has to say, as an id and its parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutNoticePayload {
    /// A `strings.csv` id.
    pub id: String,
    /// The `{placeholder}` values that id interpolates. A `BTreeMap` so the
    /// encoded object's keys are ordered, which keeps the payload byte-stable
    /// and therefore keeps the bridge's identical-push drop working.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

/// One monitor's entry in one station's screen row (issue #1331).
///
/// The law's [`MonitorChoice`] on the wire, and **only** the law's: the page
/// never works out for itself why a screen is not offered, because two
/// implementations of one rule are how a greyed button and a refusal start
/// disagreeing. `choice` is the state; `excluded` is the reason, present
/// exactly when the state is `excluded`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationScreenPayload {
    /// The monitor's stable identity — the token a press carries back. The
    /// display's *name* is not repeated here: it is on this same payload's
    /// [`MonitorButtonPayload`], and the row joins the two by identity rather
    /// than carrying one OS string twice per station.
    pub identity: String,
    /// `selected`, `eligible` or `excluded` — see [`MonitorChoice`].
    pub choice: String,
    /// Why it is greyed: `is-viewscreen` or `full`
    /// ([`ExclusionReason::as_str`]). Absent unless `choice` is `excluded`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excluded: Option<String>,
}

/// One station's screen row (issue #1331).
///
/// Every station on the ship's roster gets one, in roster order, whether or not
/// its console is open — the row is how it is *opened*, so a station with no
/// row would be a station the operator could never seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationRowPayload {
    /// The station id — the ship's own authoring key, which is what a press
    /// carries back and what the layout law is keyed on.
    pub station: String,
    /// The assigned monitor, retained while its console or display is absent.
    /// The adapter overlays logical intent on the physical layout so Off and
    /// move remain actionable during recovery. Absent means explicitly Off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_to: Option<String>,
    /// One entry per monitor this bridge has, in monitor order — including the
    /// viewscreen's, marked `is-viewscreen`. The surface drops that one rather
    /// than the host omitting it, so the row acts on the law's own answer
    /// instead of re-deriving which screen is the shared view.
    pub monitors: Vec<StationScreenPayload>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameMasterRowPayload {
    pub assigned_to: Option<String>,
    pub role_mutable: bool,
    pub monitors: Vec<StationScreenPayload>,
}

/// The whole bridge layout, as the surface receives it: the monitor row and
/// every station's screen row.
///
/// A snapshot, like the lobby payload beside it: the newest one is the truth
/// and an older one has nothing to add.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeLayoutPayload {
    #[serde(default)]
    pub gm: Option<GameMasterRowPayload>,
    /// One entry per monitor this bridge has, in discovery order.
    pub monitors: Vec<MonitorButtonPayload>,
    /// One entry per claimable station on this ship, in roster order
    /// (issue #1331). Empty for a host that has resolved no hull — a delivery
    /// host, or one still in a world-less lobby — which is a lawful bridge with
    /// nothing to seat rather than a missing field.
    #[serde(default)]
    pub stations: Vec<StationRowPayload>,
    /// What happened to the last action or bridge change — empty when nothing
    /// has.
    #[serde(default)]
    pub notices: Vec<LayoutNoticePayload>,
}

/// Something the lobby must be told about the layout it is looking at.
///
/// Two shapes because there are two sources: an action the operator took and
/// the law refused, and a change to the bridge itself that the law absorbed.
/// They render the same way and are kept apart so the host can log them
/// differently — a refusal is a person being answered, an adoption is hardware
/// moving.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutNotice {
    /// A screen assignment may not silently displace a connected phone.
    StationHeld { station: StationId, holder: String },
    /// A [`LayoutAction`] the law refused.
    Refused(LayoutRefusal),
    /// Something [`BridgeLayout::reconcile`] or `adopt_profile` had to degrade.
    Adopted(LayoutAdoption),
}

impl LayoutNotice {
    /// This notice as the lines a surface renders — **one or two**.
    ///
    /// An adoption note carrying a refusal contributes that refusal as its own
    /// line; see
    /// [`LayoutAdoption::cause`](super::super::bridge_layout::LayoutAdoption::cause)
    /// for why it is beside the note rather than inside it.
    pub fn payloads(&self) -> Vec<LayoutNoticePayload> {
        match self {
            LayoutNotice::StationHeld { station, holder } => vec![notice(
                "server.station_row.held_by_player",
                vec![("station", station.0.clone()), ("holder", holder.clone())],
            )],
            LayoutNotice::Refused(refusal) => vec![notice(refusal.string_id(), refusal.params())],
            LayoutNotice::Adopted(note) => {
                let mut out = vec![notice(note.string_id(), note.params())];
                if let Some(cause) = note.cause() {
                    out.push(notice(cause.string_id(), cause.params()));
                }
                out
            }
        }
    }
}

impl std::fmt::Display for LayoutNotice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutNotice::StationHeld { station, holder } => write!(
                f,
                "station {} is held by {holder}; release it before assigning a native screen",
                station.0
            ),
            LayoutNotice::Refused(refusal) => refusal.fmt(f),
            LayoutNotice::Adopted(note) => note.fmt(f),
        }
    }
}

fn notice(id: &str, params: Vec<(&'static str, String)>) -> LayoutNoticePayload {
    LayoutNoticePayload {
        id: id.to_string(),
        params: params
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

/// Project a live layout, the monitors it was built against and whatever the
/// lobby is owed into the payload the surface renders.
///
/// The row is built from the **layout's** monitor list rather than from
/// `discovered` directly, because the layout is what a press is judged against:
/// a button for a monitor the layout does not have would be refused the instant
/// it was pressed. `discovered` supplies the geometry and the OS name, and a
/// layout monitor missing from it is skipped rather than drawn at an invented
/// size — the two only diverge while a reconcile is pending, and half a button
/// is worse than no button.
pub fn bridge_layout_payload(
    layout: &BridgeLayout,
    discovered: &[DiscoveredMonitor],
    notices: &[LayoutNotice],
) -> BridgeLayoutPayload {
    let viewscreen = layout.viewscreen().clone();
    let monitors: Vec<MonitorButtonPayload> = layout
        .monitors()
        .iter()
        .filter_map(|identity| {
            let found = discovered.iter().find(|d| &d.identity == identity)?;
            Some(MonitorButtonPayload {
                identity: identity.as_str().to_string(),
                name: found.name.clone(),
                width: found.geometry.physical_width,
                height: found.geometry.physical_height,
                primary: found.primary,
                viewscreen: identity == &viewscreen,
                stations: layout.occupants_on(identity),
                reserved: layout.reserved_on(identity),
            })
        })
        .collect();
    // The screen rows are drawn against the SAME filtered monitor list: a
    // display the layout knows but nothing reported has no button in the
    // monitor row, and offering it a station is offering a press the row cannot
    // even draw. Both halves therefore skip it together.
    let drawn: Vec<&str> = monitors.iter().map(|m| m.identity.as_str()).collect();
    let stations = layout
        .eligibility()
        .into_iter()
        .map(|row| StationRowPayload {
            station: row.station.0,
            assigned_to: row.assigned_to.map(|m| m.as_str().to_string()),
            monitors: row
                .monitors
                .into_iter()
                .filter(|c| drawn.contains(&c.monitor.as_str()))
                .map(|c| StationScreenPayload {
                    identity: c.monitor.as_str().to_string(),
                    choice: choice_token(c.choice).to_string(),
                    excluded: c.choice.exclusion().map(|r| r.as_str().to_string()),
                })
                .collect(),
        })
        .collect();
    BridgeLayoutPayload {
        gm: Some(GameMasterRowPayload {
            assigned_to: layout.game_master_monitor().map(|m| m.as_str().to_string()),
            role_mutable: true,
            monitors: monitors
                .iter()
                .map(|m| {
                    let id = MonitorIdentity::new(&m.identity);
                    StationScreenPayload {
                        identity: m.identity.clone(),
                        choice: if layout.game_master_monitor() == Some(&id) {
                            "selected"
                        } else if layout.game_master_eligible(&id) {
                            "eligible"
                        } else {
                            "excluded"
                        }
                        .into(),
                        excluded: (!layout.game_master_eligible(&id)).then(|| "occupied".into()),
                    }
                })
                .collect(),
        }),
        monitors,
        stations,
        notices: notices.iter().flat_map(LayoutNotice::payloads).collect(),
    }
}

/// The wire token for one [`MonitorChoice`] state.
///
/// Spelled here rather than on the law, because it is this wire's vocabulary
/// rather than the law's: [`ExclusionReason::as_str`] already crosses as its own
/// field, and a `Display` for the whole choice would have to fold the reason
/// into the state and make the page split a string apart again.
fn choice_token(choice: MonitorChoice) -> &'static str {
    match choice {
        MonitorChoice::Selected => "selected",
        MonitorChoice::Eligible => "eligible",
        MonitorChoice::Excluded(_) => "excluded",
    }
}

/// Assert at compile time that the two exclusion reasons the page styles are
/// the two the law has — a third would otherwise reach `gui/` as a token
/// nothing renders, and the button would grey with no reason beside it.
const _: fn(ExclusionReason) = |reason| match reason {
    ExclusionReason::IsViewscreen | ExclusionReason::Full | ExclusionReason::GameMaster => {}
};

/// The layout action a
/// [`HostLobbyRecord::SetViewscreen`](super::HostLobbyRecord::SetViewscreen)
/// press asks for.
///
/// Total and infallible: the record's own vocabulary is the action vocabulary.
/// Whether the bridge can *take* it is [`BridgeLayout::apply`]'s answer, not
/// this function's — a press naming a monitor that was unplugged in between is
/// a [`LayoutRefusal::UnknownMonitor`], which is a sentence the operator reads,
/// rather than a parse failure nobody sees.
///
/// Here rather than beside the record for the reason the module note gives: the
/// identity a button carries out and the identity a press carries back are one
/// contract, and this is the line that closes it.
pub fn set_viewscreen_action(monitor: impl Into<String>) -> LayoutAction {
    LayoutAction::SetViewscreen {
        monitor: MonitorIdentity::new(monitor.into()),
    }
}

/// The layout action an
/// [`HostLobbyRecord::AssignStation`](super::HostLobbyRecord::AssignStation)
/// press asks for (issue #1331): open — or re-seat — this station's console on
/// this screen.
///
/// Total and infallible for the same reason [`set_viewscreen_action`] is: a
/// screen the row offered and the law has since filled is a
/// [`LayoutRefusal::MonitorFull`] the operator reads, not a parse failure.
pub fn assign_station_action(
    station: impl Into<String>,
    monitor: impl Into<String>,
) -> LayoutAction {
    LayoutAction::AssignStation {
        station: StationId(station.into()),
        monitor: MonitorIdentity::new(monitor.into()),
    }
}

/// The layout action an
/// [`HostLobbyRecord::UnassignStation`](super::HostLobbyRecord::UnassignStation)
/// press asks for (issue #1331): close this station's console. The row's "off"
/// button.
pub fn unassign_station_action(station: impl Into<String>) -> LayoutAction {
    LayoutAction::UnassignStation {
        station: StationId(station.into()),
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
