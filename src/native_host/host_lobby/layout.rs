//! What the lobby's **monitor row** is, in both directions (issue #1330) —
//! **pure, Bevy-free**.
//!
//! [`super::super::bridge_layout`] is the law: what a bridge arrangement may
//! become. This is the *conversation about it* the viewscreen's own lobby
//! surface has:
//!
//! ```text
//! BridgeLayout + DiscoveredMonitor[]  ──monitor_row_payload──▶  BridgeLayoutPayload
//!        ▲                                                            │ codec::encode_bridge_layout
//!        │                                                            ▼
//!  LayoutAction ◀─set_viewscreen_action─┐            window.__phoenixHostLobbyLayout(json)
//!                                       │                        gui/host-lobby-view.js
//!               HostLobbyRecord::SetViewscreen ◀───  phoenixHostLobbyOut.send
//!                    [super::scenario]     │ codec::decode_host_lobby_record
//! ```
//!
//! Both ends are here because they are one contract: the `identity` string a
//! button carries out is the same `identity` string the press carries back, and
//! splitting the two halves across two modules is how a renamed field becomes a
//! button that silently does nothing.
//!
//! # …but the record itself is not, and that is deliberate
//!
//! The press arrives as a variant of [`HostLobbyRecord`](super::HostLobbyRecord),
//! the surface's ONE vocabulary, beside the operator's scenario and hull picks
//! and their AI-launch press (issue #1328). It is not a record type of its own,
//! because `HostLobbyBridge::take_records` is a **drain**: a second record type
//! would want a second reader, the first reader to run would swallow the other's
//! records and warn about a vocabulary it does not speak, and the second would
//! see an empty queue forever — with a clean log on both sides. One vocabulary,
//! one drain. What stays here is the half that is genuinely this module's: the
//! [`LayoutAction`] a `set-viewscreen` press asks for.
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

use super::super::bridge_layout::{BridgeLayout, LayoutAction, LayoutAdoption, LayoutRefusal};
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stations: Vec<String>,
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

/// The whole monitor row, as the surface receives it.
///
/// A snapshot, like the lobby payload beside it: the newest one is the truth
/// and an older one has nothing to add.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeLayoutPayload {
    /// One entry per monitor this bridge has, in discovery order.
    pub monitors: Vec<MonitorButtonPayload>,
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
pub fn monitor_row_payload(
    layout: &BridgeLayout,
    discovered: &[DiscoveredMonitor],
    notices: &[LayoutNotice],
) -> BridgeLayoutPayload {
    let viewscreen = layout.viewscreen().clone();
    let monitors = layout
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
            })
        })
        .collect();
    BridgeLayoutPayload {
        monitors,
        notices: notices.iter().flat_map(LayoutNotice::payloads).collect(),
    }
}

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
        let payload = monitor_row_payload(&layout(), &two_monitors(), &[]);
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
        let payload = monitor_row_payload(&layout(), &two_monitors(), &[]);
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
        let payload = monitor_row_payload(&moved, &two_monitors(), &[]);
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
        let payload = monitor_row_payload(&layout, &one, &[]);
        assert_eq!(payload.monitors.len(), 1);
        assert!(payload.monitors[0].viewscreen);
    }

    #[test]
    fn a_monitor_the_layout_has_but_nothing_reported_is_left_out_of_the_row() {
        // The two only diverge while a reconcile is pending. Half a button —
        // named, sized 0x0 — is worse than no button.
        let payload = monitor_row_payload(&layout(), &two_monitors()[..1], &[]);
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
        let payload = monitor_row_payload(&seated, &two_monitors(), &[]);
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
        let payload = monitor_row_payload(&adopted, &two_monitors(), &[]);
        assert_eq!(payload.monitors[1].stations, vec!["Ada".to_string()]);
        assert!(payload.monitors[0].stations.is_empty());
    }

    #[test]
    fn a_refusal_crosses_as_an_id_and_its_parameters_rather_than_a_sentence() {
        let refusal = LayoutRefusal::UnknownMonitor {
            monitor: MonitorIdentity::new("Gone@1920x1080"),
        };
        let payload = monitor_row_payload(
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
            },
        };
        let payload =
            monitor_row_payload(&layout(), &two_monitors(), &[LayoutNotice::Adopted(note)]);
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
        let first = crate::core::codec::encode_bridge_layout(&monitor_row_payload(
            &layout(),
            &two_monitors(),
            &[LayoutNotice::Refused(LayoutRefusal::MonitorFull {
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
                occupants: vec![StationId("helm".to_string())],
            })],
        ))
        .unwrap();
        let second = crate::core::codec::encode_bridge_layout(&monitor_row_payload(
            &layout(),
            &two_monitors(),
            &[LayoutNotice::Refused(LayoutRefusal::MonitorFull {
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
                occupants: vec![StationId("helm".to_string())],
            })],
        ))
        .unwrap();
        assert_eq!(first, second);
        assert!(first.contains("\"identity\":\"BRAVIA@3840x2160\""));
    }
}
