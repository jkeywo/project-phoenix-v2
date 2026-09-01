//! What the lobby's **monitor row** and its **per-station screen rows** are, in
//! both directions (issues #1330, #1331) — **pure, Bevy-free**.
//!
//! [`super::super::bridge_layout`] is the law: what a bridge arrangement may
//! become. This is the *conversation about it* the viewscreen's own lobby
//! surface has:
//!
//! ```text
//! BridgeLayout + DiscoveredMonitor[] ──bridge_layout_payload──▶  BridgeLayoutPayload
//!        ▲                                                            │ codec::encode_bridge_layout
//!        │                                                            ▼
//!  LayoutAction ◀──*_action────────────┐              window.__phoenixHostLobbyLayout(json)
//!                                      │                         gui/host-lobby-view.js
//!    HostLobbyRecord::{SetViewscreen, AssignStation, UnassignStation}
//!                    [super::scenario]  ▲ codec::decode_host_lobby_record
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
    /// The monitor its console is open on, or absent when it is closed. The
    /// row's "off" state is this being absent, not a monitor entry of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_to: Option<String>,
    /// One entry per monitor this bridge has, in monitor order — including the
    /// viewscreen's, marked `is-viewscreen`. The surface drops that one rather
    /// than the host omitting it, so the row acts on the law's own answer
    /// instead of re-deriving which screen is the shared view.
    pub monitors: Vec<StationScreenPayload>,
}

/// The whole bridge layout, as the surface receives it: the monitor row and
/// every station's screen row.
///
/// A snapshot, like the lobby payload beside it: the newest one is the truth
/// and an older one has nothing to add.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeLayoutPayload {
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
    ExclusionReason::IsViewscreen | ExclusionReason::Full => {}
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
mod tests {
    use super::*;
    use crate::core::messages::StationId;
    use crate::native_host::bridge_profile::{identify, RawMonitor};

    fn raw(name: &str, w: u32, h: u32, x: i32, primary: bool) -> RawMonitor {
        RawMonitor {
            name: Some(name.to_string()),
            physical_width: w,
            physical_height: h,
            position_x: x,
            position_y: 0,
            scale_factor: 1.0,
            primary,
        }
    }

    /// A bridge of two monitors: the TV is primary and the viewscreen.
    fn two_monitors() -> Vec<DiscoveredMonitor> {
        identify(&[
            raw("BRAVIA", 3840, 2160, 0, true),
            raw("BenQ EX", 1920, 1080, 3840, false),
        ])
    }

    fn roster() -> Vec<StationId> {
        vec![StationId("helm".to_string())]
    }

    fn layout() -> BridgeLayout {
        BridgeLayout::from_discovered(&two_monitors(), roster()).expect("two monitors are a bridge")
    }

    #[test]
    fn a_button_carries_the_identity_a_press_names_back() {
        // The whole round trip in one claim: what the row shows is what the
        // press says, so the host resolves the button to the display the
        // operator was looking at rather than to an index into a list that
        // moved.
        let payload = bridge_layout_payload(&layout(), &two_monitors(), &[]);
        let pressed = &payload.monitors[1];

        // What the page sends back carries that identity verbatim, in the
        // surface's one vocabulary…
        assert_eq!(
            crate::core::codec::decode_host_lobby_record(&format!(
                r#"{{"kind":"set-viewscreen","monitor":"{}"}}"#,
                pressed.identity
            ))
            .unwrap(),
            super::super::HostLobbyRecord::SetViewscreen {
                monitor: pressed.identity.clone(),
            }
        );
        // …and it resolves to the display the operator was looking at.
        assert_eq!(
            set_viewscreen_action(pressed.identity.as_str()),
            LayoutAction::SetViewscreen {
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            }
        );
    }

    #[test]
    fn the_row_names_each_monitor_by_what_the_os_reported_and_marks_the_viewscreen() {
        let payload = bridge_layout_payload(&layout(), &two_monitors(), &[]);
        assert_eq!(payload.monitors.len(), 2);
        let tv = &payload.monitors[0];
        assert_eq!(tv.name.as_deref(), Some("BRAVIA"));
        assert_eq!((tv.width, tv.height), (3840, 2160));
        assert!(tv.primary);
        assert!(tv.viewscreen, "from_discovered seats it on the primary");
        let benq = &payload.monitors[1];
        assert!(!benq.primary);
        assert!(!benq.viewscreen);
        assert!(benq.stations.is_empty());
    }

    #[test]
    fn moving_the_viewscreen_moves_the_mark_and_nothing_else() {
        let moved = layout()
            .apply(&LayoutAction::SetViewscreen {
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            })
            .expect("a free monitor takes the viewscreen");
        let payload = bridge_layout_payload(&moved, &two_monitors(), &[]);
        assert!(!payload.monitors[0].viewscreen);
        assert!(payload.monitors[1].viewscreen);
        assert!(
            payload.monitors[0].primary,
            "the OS primary does not move because the viewscreen did"
        );
    }

    #[test]
    fn a_single_monitor_bridge_still_shows_its_one_monitor_marked() {
        // The inert row (issue #1330 AC4): one button, already the viewscreen,
        // and pressing it is the no-op the law defines.
        let one = identify(&[raw("BRAVIA", 3840, 2160, 0, true)]);
        let layout = BridgeLayout::from_discovered(&one, roster()).unwrap();
        let payload = bridge_layout_payload(&layout, &one, &[]);
        assert_eq!(payload.monitors.len(), 1);
        assert!(payload.monitors[0].viewscreen);
    }

    #[test]
    fn a_monitor_the_layout_has_but_nothing_reported_is_left_out_of_the_row() {
        // The two only diverge while a reconcile is pending. Half a button —
        // named, sized 0x0 — is worse than no button.
        let payload = bridge_layout_payload(&layout(), &two_monitors()[..1], &[]);
        assert_eq!(payload.monitors.len(), 1);
        assert_eq!(payload.monitors[0].identity, "BRAVIA@3840x2160");
    }

    #[test]
    fn a_monitor_holding_consoles_says_which_ones() {
        let seated = layout()
            .apply(&LayoutAction::AssignStation {
                station: StationId("helm".to_string()),
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            })
            .expect("a free non-viewscreen monitor takes a console");
        let payload = bridge_layout_payload(&seated, &two_monitors(), &[]);
        assert_eq!(payload.monitors[1].stations, vec!["helm".to_string()]);
    }

    #[test]
    fn a_monitor_holding_a_console_the_layout_does_not_own_says_so_too() {
        // The `--pane`-shaped profile: its panes name a person rather than a
        // station, so the layout seats nothing for them — but the display
        // adapter opens a real console on that screen, and a press to move the
        // viewscreen there is refused. The button has to say so BEFORE the
        // press, not only in the sentence that comes back.
        use crate::native_host::bridge_profile::{BridgeProfile, DisplayEntry, PaneSlot};
        use crate::native_host::bridge_profile::{ROLE_STATION, ROLE_VIEWSCREEN};

        let mut profile = BridgeProfile::empty();
        profile.displays = vec![
            DisplayEntry {
                id: "BRAVIA@3840x2160".to_string(),
                role: ROLE_VIEWSCREEN.to_string(),
                split: None,
                panes: Vec::new(),
            },
            DisplayEntry {
                id: "BenQ EX@1920x1080".to_string(),
                role: ROLE_STATION.to_string(),
                split: None,
                panes: vec![PaneSlot::for_participant("Ada")],
            },
        ];
        let (adopted, _) = layout().adopt_profile(&profile.validate().unwrap());
        let payload = bridge_layout_payload(&adopted, &two_monitors(), &[]);
        assert_eq!(payload.monitors[1].stations, vec!["Ada".to_string()]);
        assert!(payload.monitors[0].stations.is_empty());
    }

    #[test]
    fn a_refusal_crosses_as_an_id_and_its_parameters_rather_than_a_sentence() {
        let refusal = LayoutRefusal::UnknownMonitor {
            monitor: MonitorIdentity::new("Gone@1920x1080"),
        };
        let payload = bridge_layout_payload(
            &layout(),
            &two_monitors(),
            &[LayoutNotice::Refused(refusal)],
        );
        assert_eq!(payload.notices.len(), 1);
        assert_eq!(
            payload.notices[0].id,
            "server.bridge_layout.unknown_monitor"
        );
        assert_eq!(
            payload.notices[0].params.get("monitor").map(String::as_str),
            Some("Gone@1920x1080")
        );
    }

    #[test]
    fn an_adoption_note_with_a_cause_renders_as_two_lines() {
        let note = LayoutAdoption::SeatRefused {
            station: StationId("helm".to_string()),
            monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            refusal: LayoutRefusal::MonitorFull {
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
                occupants: vec![StationId("weapons".to_string())],
                panes: Vec::new(),
            },
        };
        let payload =
            bridge_layout_payload(&layout(), &two_monitors(), &[LayoutNotice::Adopted(note)]);
        assert_eq!(
            payload
                .notices
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "server.bridge_layout.adopt_seat_refused",
                "server.bridge_layout.monitor_full",
            ]
        );
    }

    #[test]
    fn a_record_the_page_sends_is_tagged_by_the_verb_it_asks_for() {
        // The wire shape, pinned: `gui/`'s side builds this object by hand, so
        // a rename here is a button that silently does nothing. The tag stays
        // KEBAB (`set-viewscreen`) even though its siblings in
        // `HostLobbyRecord` are snake_case, because the page-side JS that
        // writes it shipped before the two vocabularies were folded into one
        // and there is no reason to make it a wire break.
        let json = r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#;
        assert_eq!(
            crate::core::codec::decode_host_lobby_record(json).unwrap(),
            super::super::HostLobbyRecord::SetViewscreen {
                monitor: "BenQ EX@1920x1080".to_string(),
            }
        );
        assert!(crate::core::codec::decode_host_lobby_record(r#"{"monitor":"x"}"#).is_err());
    }

    #[test]
    fn the_encoded_payload_is_byte_stable_for_an_unchanged_layout() {
        // The bridge drops a push identical to the last one accepted, which is
        // what keeps a quiet lobby free of `evaluate_script` calls on the
        // simulation's own thread. That only works if encoding the same layout
        // twice yields the same bytes — hence the ordered parameter map.
        let first = crate::core::codec::encode_bridge_layout(&bridge_layout_payload(
            &layout(),
            &two_monitors(),
            &[LayoutNotice::Refused(LayoutRefusal::MonitorFull {
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
                occupants: vec![StationId("helm".to_string())],
                panes: Vec::new(),
            })],
        ))
        .unwrap();
        let second = crate::core::codec::encode_bridge_layout(&bridge_layout_payload(
            &layout(),
            &two_monitors(),
            &[LayoutNotice::Refused(LayoutRefusal::MonitorFull {
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
                occupants: vec![StationId("helm".to_string())],
                panes: Vec::new(),
            })],
        ))
        .unwrap();
        assert_eq!(first, second);
        assert!(first.contains("\"identity\":\"BRAVIA@3840x2160\""));
    }

    // ── the per-station screen rows (issue #1331) ───────────────────────────

    /// A three-monitor bridge, so a row can hold a full screen beside a free
    /// one and still exclude the viewscreen's.
    fn three_monitors() -> Vec<DiscoveredMonitor> {
        identify(&[
            raw("BRAVIA", 3840, 2160, 0, true),
            raw("BenQ EX", 1920, 1080, 3840, false),
            raw("Acer VG", 1280, 1024, 5760, false),
        ])
    }

    fn two_station_roster() -> Vec<StationId> {
        vec![
            StationId("helm".to_string()),
            StationId("weapons".to_string()),
        ]
    }

    fn row_for<'a>(payload: &'a BridgeLayoutPayload, station: &str) -> &'a StationRowPayload {
        payload
            .stations
            .iter()
            .find(|r| r.station == station)
            .expect("every roster station has a row")
    }

    fn screen_for<'a>(row: &'a StationRowPayload, identity: &str) -> &'a StationScreenPayload {
        row.monitors
            .iter()
            .find(|m| m.identity == identity)
            .expect("every drawn monitor has an entry")
    }

    #[test]
    fn every_station_on_the_roster_gets_a_row_whether_or_not_its_console_is_open() {
        // The row is how a console is OPENED, so a station without one is a
        // station the operator could never seat.
        let payload = bridge_layout_payload(&layout(), &two_monitors(), &[]);
        assert_eq!(
            payload
                .stations
                .iter()
                .map(|r| r.station.as_str())
                .collect::<Vec<_>>(),
            vec!["helm"]
        );
        assert!(payload.stations[0].assigned_to.is_none(), "nothing is open");
    }

    #[test]
    fn a_rows_entries_carry_the_laws_own_answer_for_every_monitor() {
        // Including the viewscreen's, marked with its reason: the surface drops
        // that entry rather than the host omitting it, so the row acts on the
        // law's answer instead of re-deriving which screen is the shared view.
        let payload = bridge_layout_payload(&layout(), &two_monitors(), &[]);
        let helm = row_for(&payload, "helm");
        assert_eq!(
            screen_for(helm, "BRAVIA@3840x2160").choice,
            "excluded",
            "the viewscreen's own display is never offered a console"
        );
        assert_eq!(
            screen_for(helm, "BRAVIA@3840x2160").excluded.as_deref(),
            Some("is-viewscreen")
        );
        assert_eq!(screen_for(helm, "BenQ EX@1920x1080").choice, "eligible");
        assert!(screen_for(helm, "BenQ EX@1920x1080").excluded.is_none());
    }

    #[test]
    fn a_seated_console_marks_its_own_screen_and_says_where_it_is() {
        let seated = layout()
            .apply(&LayoutAction::AssignStation {
                station: StationId("helm".to_string()),
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            })
            .expect("a free non-viewscreen monitor takes a console");
        let payload = bridge_layout_payload(&seated, &two_monitors(), &[]);
        let helm = row_for(&payload, "helm");
        assert_eq!(helm.assigned_to.as_deref(), Some("BenQ EX@1920x1080"));
        assert_eq!(screen_for(helm, "BenQ EX@1920x1080").choice, "selected");
    }

    #[test]
    fn a_full_screen_is_excluded_with_its_reason_rather_than_left_out() {
        // Two consoles is the per-screen maximum, so a THIRD station's row
        // greys that screen — and says `full`, which is a different sentence
        // from `is-viewscreen` and a different button state.
        let mut roster = two_station_roster();
        roster.push(StationId("comms".to_string()));
        let base = BridgeLayout::from_discovered(&three_monitors(), roster)
            .expect("three monitors are a bridge");
        let seated = base
            .apply(&LayoutAction::AssignStation {
                station: StationId("helm".to_string()),
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            })
            .and_then(|l| {
                l.apply(&LayoutAction::AssignStation {
                    station: StationId("weapons".to_string()),
                    monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
                })
            })
            .expect("two consoles fit one screen");
        let payload = bridge_layout_payload(&seated, &three_monitors(), &[]);
        let comms = row_for(&payload, "comms");
        let benq = screen_for(comms, "BenQ EX@1920x1080");
        assert_eq!(benq.choice, "excluded");
        assert_eq!(benq.excluded.as_deref(), Some("full"));
        assert_eq!(
            screen_for(comms, "Acer VG@1280x1024").choice,
            "eligible",
            "and the free screen is still offered"
        );
        // A station already ON the full screen still reads as selected there —
        // capacity is judged after the no-op case, so re-pressing its own
        // button is not a refusal.
        assert_eq!(
            screen_for(row_for(&payload, "helm"), "BenQ EX@1920x1080").choice,
            "selected"
        );
    }

    #[test]
    fn a_screen_an_authored_console_shares_offers_one_slot_and_then_none() {
        // The carried #1331 defect at the WIRE (issue #1332). The screen row is
        // built from the law's eligibility, so counting an authored `--pane`
        // console against the two-per-screen cap has to reach the page as a
        // greyed button — and the button has to be able to say who is on it,
        // which is why the monitor entry lists the authored label beside the
        // seats. A participant has no station row of their own, so without that
        // list the screen would grey for no visible reason at all.
        use crate::native_host::bridge_profile::{BridgeProfile, DisplayEntry, PaneSlot};
        use crate::native_host::bridge_profile::{ROLE_STATION, ROLE_VIEWSCREEN};

        let mut profile = BridgeProfile::empty();
        profile.displays = vec![
            DisplayEntry {
                id: "BRAVIA@3840x2160".to_string(),
                role: ROLE_VIEWSCREEN.to_string(),
                split: None,
                panes: Vec::new(),
            },
            DisplayEntry {
                id: "BenQ EX@1920x1080".to_string(),
                role: ROLE_STATION.to_string(),
                split: None,
                panes: vec![PaneSlot::for_participant("Ada")],
            },
        ];
        let base = BridgeLayout::from_discovered(&three_monitors(), two_station_roster())
            .expect("three monitors are a bridge");
        let (adopted, _) = base.adopt_profile(&profile.validate().unwrap());

        // One authored console: the screen still has a slot, so it is offered.
        let payload = bridge_layout_payload(&adopted, &three_monitors(), &[]);
        assert_eq!(
            screen_for(row_for(&payload, "helm"), "BenQ EX@1920x1080").choice,
            "eligible"
        );

        // Take it, and the screen is full for everybody else — the state that
        // used to read `eligible`, and used to let the two consoles overlap.
        let shared = adopted
            .apply(&LayoutAction::AssignStation {
                station: StationId("helm".to_string()),
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            })
            .expect("a screen with one authored console takes one station");
        let payload = bridge_layout_payload(&shared, &three_monitors(), &[]);
        let weapons = screen_for(row_for(&payload, "weapons"), "BenQ EX@1920x1080");
        assert_eq!(weapons.choice, "excluded");
        assert_eq!(weapons.excluded.as_deref(), Some("full"));
        assert_eq!(
            payload.monitors[1].stations,
            vec!["Ada".to_string(), "helm".to_string()],
            "and the button names both, so the greying has a visible reason — in the order they \
             are DRAWN on that screen (issue #1332's fix round), so a person looking at the glass \
             reads the button left to right and finds them where it says they are. Ada was \
             authored into the first pane slot, so she is the left half and the station seated \
             beside her is the right"
        );
        assert_eq!(
            payload.monitors[1].reserved,
            vec!["Ada".to_string()],
            "and the one of them the lobby cannot free is named as such: there is no unassign \
             button for an authored console, and a row that only said `full` would send the \
             operator hunting for one"
        );
    }

    #[test]
    fn a_single_monitor_bridge_offers_a_station_no_screen_at_all() {
        // Issue #1331's single-monitor acceptance criterion, at the model
        // boundary: the one display is the viewscreen, the law excludes it, and
        // what is left is nothing — which is what the surface renders its
        // "consoles need a second monitor" line from.
        let one = identify(&[raw("BRAVIA", 3840, 2160, 0, true)]);
        let layout = BridgeLayout::from_discovered(&one, roster()).unwrap();
        let payload = bridge_layout_payload(&layout, &one, &[]);
        let helm = row_for(&payload, "helm");
        assert_eq!(helm.monitors.len(), 1);
        assert_eq!(helm.monitors[0].excluded.as_deref(), Some("is-viewscreen"));
        assert!(
            !helm.monitors.iter().any(|m| m.choice == "eligible"),
            "no screen may hold a console on a one-screen bridge"
        );
    }

    #[test]
    fn a_monitor_the_row_cannot_draw_is_not_offered_to_a_station_either() {
        // The monitor row skips a display the layout knows but nothing
        // reported (half a button is worse than no button). Offering a station
        // that same display would offer a press the operator cannot see.
        let payload = bridge_layout_payload(&layout(), &two_monitors()[..1], &[]);
        assert_eq!(payload.monitors.len(), 1);
        assert_eq!(row_for(&payload, "helm").monitors.len(), 1);
    }

    #[test]
    fn a_host_with_no_hull_has_a_monitor_row_and_no_station_rows() {
        // A delivery host, or one still in a world-less lobby: a lawful bridge
        // with an empty roster. The viewscreen still moves; there is simply
        // nothing to seat.
        let bare = BridgeLayout::from_discovered(&two_monitors(), []).unwrap();
        let payload = bridge_layout_payload(&bare, &two_monitors(), &[]);
        assert_eq!(payload.monitors.len(), 2);
        assert!(payload.stations.is_empty());
    }

    #[test]
    fn the_two_station_verbs_round_trip_from_the_page_as_the_law_reads_them() {
        // The wire shape, pinned: `gui/`'s side builds these objects by hand.
        // They arrive in the surface's ONE vocabulary, beside the picks, and
        // KEBAB like the `set-viewscreen` they were written to match.
        use super::super::HostLobbyRecord;
        assert_eq!(
            crate::core::codec::decode_host_lobby_record(
                r#"{"kind":"assign-station","station":"helm","monitor":"BenQ EX@1920x1080"}"#
            )
            .unwrap(),
            HostLobbyRecord::AssignStation {
                station: "helm".to_string(),
                monitor: "BenQ EX@1920x1080".to_string(),
            }
        );
        assert_eq!(
            assign_station_action("helm", "BenQ EX@1920x1080"),
            LayoutAction::AssignStation {
                station: StationId("helm".to_string()),
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            }
        );
        assert_eq!(
            crate::core::codec::decode_host_lobby_record(
                r#"{"kind":"unassign-station","station":"helm"}"#
            )
            .unwrap(),
            HostLobbyRecord::UnassignStation {
                station: "helm".to_string(),
            }
        );
        assert_eq!(
            unassign_station_action("helm"),
            LayoutAction::UnassignStation {
                station: StationId("helm".to_string()),
            }
        );
        // There is no `move-station` verb, because the law has no move action:
        // assigning a seated station elsewhere IS the move.
        assert!(crate::core::codec::decode_host_lobby_record(
            r#"{"kind":"move-station","station":"helm","monitor":"x"}"#
        )
        .is_err());
    }
}
