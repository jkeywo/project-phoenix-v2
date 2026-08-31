//! The bridge-display profile model (issue #1123) — **pure, Bevy-free**.
//!
//! A bridge is a room of monitors. One is the shared viewscreen; the others are
//! Station displays, each showing one or two console panes to the crew sitting at
//! it. This module is the operator's answer to "which monitor is which", written
//! down once so it survives a reboot: a [`BridgeProfile`] is an ordered list of
//! `(stable monitor id → role)` plus the touch-input mapping, and it round-trips
//! through a TOML file an operator can hand-edit.
//!
//! Everything here is a plain data transform with no Bevy, no winit and no
//! display — which is the point. The acceptance criteria that actually have
//! *logic* in them — a monitor keeps its identity across an OS-settings
//! rearrange, a Station refuses a third pane, a profile reloads to the same
//! roles, a vanished monitor is named rather than silently re-homed — are all
//! decided in this file and checked by the ordinary `cargo test` CI runs. The
//! winit adapter that enumerates real monitors and opens borderless-fullscreen
//! windows from a resolved profile is [`super::bridge_display`], and it is
//! provable only under the `#[ignore]`d integration test on a machine with
//! real displays.
//!
//! # It is NOT the player Accessibility profile
//!
//! Issue #1127 owns a private, per-player Accessibility profile (reduced motion,
//! colour, text size). This is a different file with a different owner: it is
//! **operator** configuration of the physical bridge, shared by everyone in the
//! room, and it carries nothing about any individual player. The two are kept
//! deliberately separate so that packing a bridge for transport and setting a
//! player's comfort options never touch the same file.
//!
//! # The stable-identity scheme, and its limits
//!
//! winit (and so Bevy's [`Monitor`](bevy::window::Monitor)) exposes no serial
//! number or EDID — nothing a display carries in hardware. What it does report
//! that does not change when displays are re-ordered or repositioned in the OS
//! settings is the **name** and the **native resolution**. So a monitor's
//! stable identity here is `name@WxH` ([`identify`]), deliberately excluding
//! position (which changes on every rearrange — surviving that is the whole
//! job) and scale factor (which a user can change). That survives the
//! rearrange an operator does most often, but it has two inherent limits —
//! inherent to what the OS exposes, not to this scheme — both documented on
//! [`identify`] where the code lives:
//!
//! - Two *identical* monitors — same model, same mode — report the same name
//!   and size and are genuinely indistinguishable to winit; those, and only
//!   those, are told apart by appending their physical position (`#x,y`), and
//!   that one case does not survive physically swapping the two.
//! - On Windows, the reported `name` is typically the GDI **device/slot name**
//!   (`\\.\DISPLAY5`), which is tied to the port a monitor is plugged into —
//!   so the identity does **not** necessarily survive re-plugging a monitor
//!   into a *different* port, only re-ordering or repositioning it in the OS
//!   settings while it stays connected where it is.

use serde::{Deserialize, Serialize};

/// The current [`BridgeProfile`] schema version.
///
/// Bumped only on a breaking change to the on-disk shape. A profile that names a
/// different version is refused with [`ProfileError::Version`] rather than
/// silently reinterpreted, because a display role read out of a schema this
/// build does not understand is exactly the "plausible mission, wrong numbers"
/// failure the native host guards against everywhere else.
pub const PROFILE_VERSION: u32 = 1;

/// The most console panes one Station monitor may host — the PRD's pane-density
/// rule (issue #1123).
///
/// Not a designer's tunable: it is a legibility bound. A bridge Station is one
/// or two operators sitting at one physical display at bridge viewing distance,
/// and a console is authored to be read one, or side-by-side two, to a screen.
/// Three or more consoles on one monitor is not a smaller version of the same
/// thing — it is unreadable — so the profile refuses it at author time rather
/// than rendering it. The PRD states the "exactly one or two fixed panes" rule;
/// this constant is that rule.
pub const MAX_PANES_PER_STATION: usize = 2;

/// The fewest console panes a Station monitor may host: a Station with no pane
/// is not a Station, it is an unassigned monitor, which is a different role.
pub const MIN_PANES_PER_STATION: usize = 1;

// ── stable monitor identity ────────────────────────────────────────────────

/// A monitor's stable hardware identity — see the [module note](self#the-stable-identity-scheme-and-its-limit).
///
/// A newtype over the composite key string so it cannot be confused with an
/// arbitrary `String` at a call site, and so it serialises transparently (the
/// profile's `id = "…"` fields are the bare key, readable and hand-editable).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MonitorIdentity(String);

impl MonitorIdentity {
    /// Wrap a pre-computed key. Prefer [`identify`], which computes keys for a
    /// whole enumeration at once so it can disambiguate identical monitors.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MonitorIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A monitor as the OS reported it, before identities are assigned.
///
/// The winit/Bevy-shaped fields of one display, lifted out of Bevy so
/// [`identify`] and every test can build one without a window. The adapter fills
/// it from a Bevy [`Monitor`](bevy::window::Monitor) component; a test fills it
/// by hand.
#[derive(Clone, Debug, PartialEq)]
pub struct RawMonitor {
    /// The OS-reported name, if any. On Windows this is typically the display's
    /// device or friendly name; it may be absent on some backends.
    pub name: Option<String>,
    /// Native resolution, physical pixels.
    pub physical_width: u32,
    pub physical_height: u32,
    /// Top-left corner in the virtual desktop, physical pixels. Part of the
    /// geometry a surface needs, but deliberately **not** part of the identity.
    pub position_x: i32,
    pub position_y: i32,
    /// The OS scale factor (DPI). Reported for geometry; not part of identity.
    pub scale_factor: f64,
    /// Whether the OS marks this the primary monitor.
    pub primary: bool,
}

/// A monitor's geometry: where it is and how big, in physical pixels, plus its
/// scale factor. Everything a surface needs to cover it; nothing about identity.
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorGeometry {
    pub physical_width: u32,
    pub physical_height: u32,
    pub position_x: i32,
    pub position_y: i32,
    pub scale_factor: f64,
}

/// A discovered monitor: its stable [`MonitorIdentity`], its [`MonitorGeometry`]
/// and the raw name, as [`identify`] produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredMonitor {
    pub identity: MonitorIdentity,
    pub geometry: MonitorGeometry,
    /// The OS name, kept for the setup report; the identity is derived from it.
    pub name: Option<String>,
    pub primary: bool,
}

/// The placeholder used in an identity key when the OS reported no name. A
/// constant so the key is stable across runs of a nameless display rather than
/// varying with whatever a formatter would print for `None`.
const UNNAMED_DISPLAY: &str = "unnamed-display";

/// Assign each raw monitor a stable identity, preserving input order.
///
/// The identity is `name@WxH` — the OS name and the native resolution — which
/// survives the rearrange an operator makes most often: re-ordering or
/// repositioning displays in the OS settings while they stay plugged into the
/// same ports. Position and scale factor are deliberately excluded: both
/// change under an ordinary rearrange, and an identity that changed with them
/// would defeat the whole purpose of a reusable profile.
///
/// **Two cases this cannot survive**, both inherent to what winit exposes
/// rather than a flaw in this scheme:
///
/// - Two monitors of the same model in the same mode report the same name and
///   size and are indistinguishable to winit, which exposes no per-unit
///   serial. Those — and *only* those — are disambiguated by appending their
///   physical position (`name@WxH#x,y`), so a profile referencing them is
///   stable while the physical arrangement holds but swaps the two if they are
///   physically swapped. A single monitor of a given name+size keeps the
///   short, position-free key and is fully stable.
/// - On Windows, `name` is typically the GDI device/slot name
///   (`\\.\DISPLAY5`), which is tied to the port the monitor is plugged into —
///   so re-plugging a monitor into a *different* port can change its reported
///   name, and so its identity here, even though the monitor itself did not
///   change.
///
/// The return order matches the input so a caller can zip it back against the
/// Bevy monitor entities it came from.
pub fn identify(raws: &[RawMonitor]) -> Vec<DiscoveredMonitor> {
    // Count base keys so a unique monitor keeps the short, position-free key and
    // only genuine collisions pay the position suffix.
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for raw in raws {
        *counts.entry(base_key(raw)).or_insert(0) += 1;
    }
    raws.iter()
        .map(|raw| {
            let base = base_key(raw);
            let key = if counts.get(&base).copied().unwrap_or(0) > 1 {
                format!("{base}#{},{}", raw.position_x, raw.position_y)
            } else {
                base
            };
            DiscoveredMonitor {
                identity: MonitorIdentity(key),
                geometry: MonitorGeometry {
                    physical_width: raw.physical_width,
                    physical_height: raw.physical_height,
                    position_x: raw.position_x,
                    position_y: raw.position_y,
                    scale_factor: raw.scale_factor,
                },
                name: raw.name.clone(),
                primary: raw.primary,
            }
        })
        .collect()
}

/// The position-free part of an identity: `name@WxH`.
fn base_key(raw: &RawMonitor) -> String {
    let name = raw.name.as_deref().unwrap_or(UNNAMED_DISPLAY);
    format!("{name}@{}x{}", raw.physical_width, raw.physical_height)
}

// ── pane layout ─────────────────────────────────────────────────────────────

/// One console pane on a Station monitor, and who sits at it.
///
/// The `label` is the participant name — the same name a `--pane <NAME>` flag
/// gives (issue #1122). A pane still joins the lobby and claims a Station from
/// inside its own console; the profile does not seat it, it only records which
/// named crew member this physical pane belongs to so the layout reloads the
/// same way.
///
/// # Why a pane also carries a station id ([ai] issue #1327)
///
/// The lobby's bridge layout ([`super::bridge_layout`]) places **stations**, not
/// participants: a console opened on a wall monitor is claimable by anyone, so
/// there is no crew member to name at layout time. That layout keys everything
/// by [`StationId`](crate::core::messages::StationId), and it persists through
/// this same profile — so the station id has to survive the TOML round-trip, and
/// this is where it rides: an optional `station = "…"` beside the label in a
/// `[[display.pane]]` table.
///
/// It is a **separate optional field rather than an overloaded `label`** on
/// purpose. `label` keeps its one meaning (the participant name that
/// `open_pane_for_name` resolves and that [`ProfileError::DuplicatePaneLabel`]
/// governs); a pane with no `station` is a hand-authored `--pane` pane and is
/// read back as exactly that, instead of being silently minted into a station
/// whose id happens to be somebody's name. Absent from the file it is `None`, and
/// it is skipped when serialising — so every profile authored before #1327
/// parses unchanged and round-trips byte-identically.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneSlot {
    pub label: String,
    /// The station id whose console this pane shows, when a layout placed it.
    /// `None` for a hand-authored `--pane <NAME>` pane — see the type note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
}

impl PaneSlot {
    /// A pane opened for a **named participant** — the `--pane <NAME>` shape
    /// (issue #1122). It belongs to no particular station; the crew member at it
    /// claims one from inside their own console, exactly as a phone does.
    pub fn for_participant(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            station: None,
        }
    }

    /// A pane opened for a **station** by the lobby layout (issue #1327).
    ///
    /// The pane is *named* for the station as well as keyed by it: a
    /// lobby-opened console has no participant yet (anyone may claim it), and
    /// station ids are unique across a ship, so naming the pane for its station
    /// satisfies the whole-profile label-uniqueness rule by construction.
    pub fn for_station(station: impl Into<String>) -> Self {
        let station = station.into();
        Self {
            label: station.clone(),
            station: Some(station),
        }
    }
}

/// How a two-pane Station divides its monitor.
///
/// One-pane Stations ignore it (the pane is the whole monitor). The default is
/// [`SideBySide`](PaneSplit::SideBySide), matching the left-to-right tiling the
/// pre-#1123 pane host already used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneSplit {
    /// Two panes left and right, each half the width, full height.
    #[default]
    SideBySide,
    /// Two panes top and bottom, each half the height, full width.
    Stacked,
}

/// One pane's rectangle within its Station window, in physical pixels, origin
/// top-left of the monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Tile `count` panes across `geometry` along `split`, so they exactly cover the
/// monitor with no gap and no overlap.
///
/// One pane is the whole monitor; two divide it in half along the split axis,
/// the second pane absorbing an odd pixel so the two rectangles still tile
/// exactly. `count` is only ever 1 or 2 in a validated profile
/// ([`MAX_PANES_PER_STATION`]); the general tiling handles any `count` so this
/// stays one code path rather than a pair of special cases that could drift.
pub fn pane_rects(geometry: &MonitorGeometry, split: PaneSplit, count: usize) -> Vec<PaneRect> {
    if count == 0 {
        return Vec::new();
    }
    let (w, h) = (geometry.physical_width, geometry.physical_height);
    let mut rects = Vec::with_capacity(count);
    let n = count as u32;
    match split {
        PaneSplit::SideBySide => {
            let base = w / n;
            let mut x = 0u32;
            for i in 0..n {
                let width = if i == n - 1 { w - x } else { base };
                rects.push(PaneRect {
                    x,
                    y: 0,
                    width,
                    height: h,
                });
                x += width;
            }
        }
        PaneSplit::Stacked => {
            let base = h / n;
            let mut y = 0u32;
            for i in 0..n {
                let height = if i == n - 1 { h - y } else { base };
                rects.push(PaneRect {
                    x: 0,
                    y,
                    width: w,
                    height,
                });
                y += height;
            }
        }
    }
    rects
}

// ── validated role ──────────────────────────────────────────────────────────

/// What a configured monitor is, after validation.
///
/// The strongly-typed role, produced by [`BridgeProfile::validate`] from the
/// permissive on-disk [`DisplayEntry`]. A [`Station`](DisplayRole::Station) here
/// is guaranteed to carry [`MIN_PANES_PER_STATION`]..=[`MAX_PANES_PER_STATION`]
/// panes — the density rule is enforced at the boundary, so nothing downstream
/// has to re-check it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayRole {
    /// The shared 3-D viewscreen the whole crew watches.
    Viewscreen,
    /// A crew Station showing one or two console panes.
    Station {
        split: PaneSplit,
        panes: Vec<PaneSlot>,
    },
}

impl DisplayRole {
    /// A one-line human summary for the setup report and logs.
    pub fn summary(&self) -> String {
        match self {
            DisplayRole::Viewscreen => "viewscreen".to_string(),
            DisplayRole::Station { panes, split } => {
                let each = if panes.len() == 1 { "pane" } else { "panes" };
                let who: Vec<&str> = panes.iter().map(|p| p.label.as_str()).collect();
                format!(
                    "station, {} {each} ({}) {}",
                    panes.len(),
                    who.join(", "),
                    match split {
                        PaneSplit::SideBySide => "side by side",
                        PaneSplit::Stacked => "stacked",
                    }
                )
            }
        }
    }
}

// ── on-disk profile ───────────────────────────────────────────────────────

/// The role string a [`DisplayEntry`] carries for the shared viewscreen.
pub const ROLE_VIEWSCREEN: &str = "viewscreen";
/// The role string a [`DisplayEntry`] carries for a crew Station.
pub const ROLE_STATION: &str = "station";

/// A touch input device mapped to a monitor (issue #1123 persists it).
///
/// #1123 owns making the mapping *reload*: which physical touchscreen drives
/// which display is part of a bridge's setup and must survive a reboot, so this
/// is deliberately just the recorded input, not a router.
///
/// #1124's router does **not** consult this mapping. With one borderless
/// -fullscreen Station window per touch display, winit already delivers a touch
/// against the right window and `native_host::panes::ultralight::route_touch_input`
/// routes by `touch.window` alone — see that function's note. This table is
/// therefore recorded for the future case it is actually needed: a touch panel
/// *decoupled* from its monitor, whose device-global coordinates a later revision
/// would map to a display through here and then to a pane via
/// `PaneRouter::resolve_desktop`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TouchMapping {
    /// The OS-reported name of the touch input device.
    pub device: String,
    /// The [`MonitorIdentity`] key of the monitor this device's surface covers.
    pub monitor: String,
}

/// One on-disk display assignment: the permissive shape the TOML deserialises to.
///
/// Kept separate from the validated [`DisplayRole`] so the file can round-trip
/// without serde having to encode an enum with a nested sequence (which the
/// `toml` crate handles poorly), and so the density rule and the role vocabulary
/// are checked in one explicit place — [`BridgeProfile::validate`] — rather than
/// smeared across `Deserialize`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayEntry {
    /// The monitor's stable identity key (see [`identify`]).
    pub id: String,
    /// `"viewscreen"` or `"station"` — see [`ROLE_VIEWSCREEN`]/[`ROLE_STATION`].
    pub role: String,
    /// How a two-pane Station divides its monitor. Absent (and ignored) for a
    /// viewscreen or a one-pane Station.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<PaneSplit>,
    /// The Station's panes, one or two. Empty for a viewscreen. Emitted as
    /// `[[display.pane]]` sub-tables.
    #[serde(default, rename = "pane", skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<PaneSlot>,
}

/// A reusable bridge-display profile: an ordered list of monitor assignments
/// plus the touch mapping, round-tripped through a TOML file.
///
/// The list is **ordered** (an array of `[[display]]` tables) rather than a map,
/// because the operator's intent has an order — the viewscreen first, then the
/// Stations left to right — and a reload that reshuffled it would be a silent
/// rearrangement of exactly the kind issue #1123 forbids.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BridgeProfile {
    /// Schema version — see [`PROFILE_VERSION`].
    pub version: u32,
    /// The monitor assignments, in operator order. `[[display]]` tables.
    #[serde(default, rename = "display", skip_serializing_if = "Vec::is_empty")]
    pub displays: Vec<DisplayEntry>,
    /// Touch device → monitor mappings. `[[touch]]` tables.
    #[serde(default, rename = "touch", skip_serializing_if = "Vec::is_empty")]
    pub touch: Vec<TouchMapping>,
    /// Per-surface media-device assignments (issue #1126). `[[media]]` tables in
    /// the same file — a bridge profile records the physical room's cameras,
    /// microphones and speakers beside its monitors. Absent (and skipped) for a
    /// display-only profile; the model lives in [`super::bridge_media`].
    #[serde(default, rename = "media", skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<super::bridge_media::MediaSurfaceEntry>,
}

impl BridgeProfile {
    /// An empty profile at the current schema version.
    pub fn empty() -> Self {
        Self {
            version: PROFILE_VERSION,
            displays: Vec::new(),
            touch: Vec::new(),
            media: Vec::new(),
        }
    }

    /// Serialise to a TOML string.
    ///
    /// TOML rather than RON ([ai] decision): a bridge profile is operator
    /// configuration in the same family as the world and scenario files an
    /// author hand-edits, not an internal state dump, and `toml` is a dependency
    /// on every target while `ron` is native-only here. The output is stable and
    /// human-editable — that an operator can open it and read "which monitor is
    /// the viewscreen" is a feature.
    pub fn to_toml(&self) -> Result<String, ProfileError> {
        toml::to_string_pretty(self).map_err(|e| ProfileError::Serialize(e.to_string()))
    }

    /// Parse a TOML string into a profile. Does not validate roles or density —
    /// call [`validate`](Self::validate) for that.
    pub fn from_toml(text: &str) -> Result<Self, ProfileError> {
        toml::from_str(text).map_err(|e| ProfileError::Parse(e.to_string()))
    }

    /// Check the schema version, the role vocabulary, the pane-density rule,
    /// identity uniqueness and the one-viewscreen rule, producing the
    /// strongly-typed [`ValidatedProfile`].
    ///
    /// This is where the "refused with a clear explanation" acceptance criterion
    /// lives: a Station with three panes, an unknown role word, a viewscreen that
    /// carries panes, two displays claiming one monitor, two displays both
    /// claiming the viewscreen role, or a profile that assigns monitors but names
    /// **no** viewscreen ([`ProfileError::MissingViewscreen`], issue #1327) are
    /// each rejected here with an authored message, before any window is opened.
    ///
    /// Note what is *not* checked here, and does not need to be: "a Station on
    /// the viewscreen's monitor" is structurally impossible in a profile, because
    /// a monitor appears at most once ([`ProfileError::DuplicateId`]). The
    /// overlap the bridge law forbids is a property of a runtime *transition*,
    /// and it is refused there — see [`super::bridge_layout`].
    pub fn validate(&self) -> Result<ValidatedProfile, ProfileError> {
        if self.version != PROFILE_VERSION {
            return Err(ProfileError::Version {
                found: self.version,
            });
        }
        let mut displays = Vec::with_capacity(self.displays.len());
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        // Every participant label already claimed by a Station pane. A label must
        // be unique across the WHOLE profile: the runtime display-loss watcher
        // maps a lost monitor's pane labels to the panes to disconnect by NAME
        // (`open_pane_for_name`), with no monitor anchor, so the same label on two
        // monitors could not be resolved to the right one. Refused at author time
        // rather than mis-resolved at runtime.
        let mut seen_labels: std::collections::HashSet<&str> = std::collections::HashSet::new();
        // Named as soon as a second one turns up, rather than accumulated and
        // reported at the end: a bridge has exactly one shared viewscreen, and
        // `apply_bridge_profile` has no "last one wins" rule to fall back on —
        // it would just silently keep the last surface it saw and leave every
        // other configured monitor black with no diagnostic at all.
        let mut viewscreen_id: Option<&str> = None;
        for entry in &self.displays {
            if !seen.insert(entry.id.as_str()) {
                return Err(ProfileError::DuplicateId {
                    id: entry.id.clone(),
                });
            }
            let role = match entry.role.as_str() {
                ROLE_VIEWSCREEN => {
                    if !entry.panes.is_empty() {
                        return Err(ProfileError::ViewscreenHasPanes {
                            id: entry.id.clone(),
                        });
                    }
                    if let Some(first) = viewscreen_id {
                        return Err(ProfileError::MultipleViewscreens {
                            ids: vec![first.to_string(), entry.id.clone()],
                        });
                    }
                    viewscreen_id = Some(entry.id.as_str());
                    DisplayRole::Viewscreen
                }
                ROLE_STATION => {
                    let count = entry.panes.len();
                    if !(MIN_PANES_PER_STATION..=MAX_PANES_PER_STATION).contains(&count) {
                        return Err(ProfileError::Density {
                            id: entry.id.clone(),
                            count,
                        });
                    }
                    for pane in &entry.panes {
                        if !seen_labels.insert(pane.label.as_str()) {
                            return Err(ProfileError::DuplicatePaneLabel {
                                label: pane.label.clone(),
                            });
                        }
                    }
                    DisplayRole::Station {
                        split: entry.split.unwrap_or_default(),
                        panes: entry.panes.clone(),
                    }
                }
                other => {
                    return Err(ProfileError::UnknownRole {
                        id: entry.id.clone(),
                        role: other.to_string(),
                    })
                }
            };
            displays.push(ValidatedDisplay {
                identity: MonitorIdentity(entry.id.clone()),
                role,
            });
        }
        // A profile that assigns monitors but names no viewscreen (issue #1327).
        // The harm is concrete and is the ONE station-over-viewscreen path a
        // hand-authored profile still has: `apply_bridge_profile` puts the
        // viewscreen role on the process's primary window, so with no viewscreen
        // entry that window is never placed and stays on whatever monitor the OS
        // opened it on — a monitor one of these Station entries may also name,
        // whereupon a borderless-fullscreen console covers the one surface the
        // whole bridge watches. Gated on there being displays at all: a profile
        // that assigns no monitor opens no Station window, so it has nothing to
        // cover the viewscreen with (a `[[touch]]`/`[[media]]`-only profile is
        // the shipped shape of that, and stays valid).
        if !self.displays.is_empty() && viewscreen_id.is_none() {
            return Err(ProfileError::MissingViewscreen {
                stations: self.displays.iter().map(|d| d.id.clone()).collect(),
            });
        }

        // The media assignments (issue #1126) are validated in the same pass, so
        // a wrong-kind device, a duplicate or an unconsented share fails at the
        // prompt exactly as a bad display role does — see `bridge_media`.
        let media =
            super::bridge_media::validate_media(&self.media).map_err(ProfileError::Media)?;

        Ok(ValidatedProfile {
            displays,
            touch: self.touch.clone(),
            media,
        })
    }
}

/// A profile whose roles and density have been checked — see
/// [`BridgeProfile::validate`].
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedProfile {
    pub displays: Vec<ValidatedDisplay>,
    pub touch: Vec<TouchMapping>,
    /// The validated media-device assignments (issue #1126). Empty for a
    /// display-only profile.
    pub media: super::bridge_media::ValidatedMedia,
}

/// One validated monitor assignment.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedDisplay {
    pub identity: MonitorIdentity,
    pub role: DisplayRole,
}

/// Why a [`BridgeProfile`] could not be validated, serialised or parsed.
///
/// Every variant carries enough to name the offending display, because the
/// acceptance criterion is a *clear explanation*, not a boolean.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileError {
    /// The file's schema version is not one this build understands.
    Version { found: u32 },
    /// A Station names a number of panes outside one-or-two — the density rule.
    Density { id: String, count: usize },
    /// A display's `role` word is neither `viewscreen` nor `station`.
    UnknownRole { id: String, role: String },
    /// A viewscreen display carries pane assignments, which only a Station may.
    ViewscreenHasPanes { id: String },
    /// Two `[[display]]` entries name the same monitor id.
    DuplicateId { id: String },
    /// Two Station panes across the profile carry the same participant label. A
    /// lost monitor's panes are resolved to disconnect by name, so a label shared
    /// by two panes could not be resolved to the right monitor — each participant
    /// label is unique across the whole bridge.
    DuplicatePaneLabel { label: String },
    /// Two or more `[[display]]` entries claim the `viewscreen` role. A bridge
    /// has exactly one shared viewscreen; without this refusal
    /// `apply_bridge_profile` would silently keep only the last one and leave
    /// every other configured monitor black with no diagnostic.
    MultipleViewscreens { ids: Vec<String> },
    /// The profile assigns monitors but names **no** viewscreen (issue #1327).
    /// The viewscreen role is what places the process's primary window, so a
    /// profile without one leaves that window on whatever monitor the OS opened
    /// it on — a monitor a `station` entry here may also claim, covering the
    /// viewscreen with a console. Carries the ids of the displays it does assign.
    MissingViewscreen { stations: Vec<String> },
    /// A `[[media]]` assignment is invalid (issue #1126): a wrong-kind device in
    /// a slot, a duplicate, an unconsented share, and so on. The wrapped
    /// [`MediaError`](super::bridge_media::MediaError) carries the detail.
    Media(super::bridge_media::MediaError),
    /// The TOML did not parse.
    Parse(String),
    /// The profile could not be serialised to TOML.
    Serialize(String),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::Version { found } => write!(
                f,
                "bridge profile is schema version {found}, but this build understands version \
                 {PROFILE_VERSION}; a profile from a newer build must not be reinterpreted"
            ),
            ProfileError::Density { id, count } => write!(
                f,
                "the Station on monitor {id:?} is configured with {count} panes, but a Station \
                 supports {MIN_PANES_PER_STATION} or {MAX_PANES_PER_STATION}: a console is \
                 authored to be read one, or side-by-side two, to a screen, and three or more \
                 at bridge distance is unreadable. Split the extra panes onto another Station \
                 monitor"
            ),
            ProfileError::UnknownRole { id, role } => write!(
                f,
                "monitor {id:?} has role {role:?}, which is neither {ROLE_VIEWSCREEN:?} nor \
                 {ROLE_STATION:?}"
            ),
            ProfileError::ViewscreenHasPanes { id } => write!(
                f,
                "monitor {id:?} is the viewscreen but also lists console panes; only a \
                 {ROLE_STATION:?} monitor hosts panes"
            ),
            ProfileError::DuplicateId { id } => write!(
                f,
                "two displays both claim monitor {id:?}; each monitor is assigned exactly once"
            ),
            ProfileError::DuplicatePaneLabel { label } => write!(
                f,
                "two Station panes both carry the participant label {label:?}; a lost monitor's \
                 panes are resolved to disconnect by name, so each label must be unique across \
                 the whole bridge"
            ),
            ProfileError::MultipleViewscreens { ids } => write!(
                f,
                "monitors {} are both configured as the {ROLE_VIEWSCREEN:?} role, but a bridge \
                 has exactly one shared viewscreen; give every monitor but one a \
                 {ROLE_STATION:?} role instead",
                ids.iter()
                    .map(|id| format!("{id:?}"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
            ProfileError::MissingViewscreen { stations } => write!(
                f,
                "the profile assigns {} but gives no monitor the {ROLE_VIEWSCREEN:?} role; the \
                 viewscreen is what places this process's primary window, so without one that \
                 window stays on whatever monitor the OS opened it on — a monitor {} may also \
                 claim, covering the shared viewscreen with a console. Give exactly one monitor \
                 the {ROLE_VIEWSCREEN:?} role",
                match stations.len() {
                    1 => "one monitor".to_string(),
                    n => format!("{n} monitors"),
                },
                stations
                    .iter()
                    .map(|id| format!("{id:?}"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
            ProfileError::Media(e) => write!(f, "bridge profile media assignment: {e}"),
            ProfileError::Parse(detail) => write!(f, "bridge profile did not parse: {detail}"),
            ProfileError::Serialize(detail) => {
                write!(f, "bridge profile could not be written: {detail}")
            }
        }
    }
}

impl std::error::Error for ProfileError {}

// ── resolution against real monitors ───────────────────────────────────────

/// One monitor a profile assigns AND that is actually present: a surface to open.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSurface {
    pub identity: MonitorIdentity,
    pub geometry: MonitorGeometry,
    pub role: DisplayRole,
    /// Whether this is the OS primary monitor — the winit adapter puts the
    /// viewscreen on the primary window, so it needs to know which is which.
    pub primary: bool,
}

/// Something the profile and the present displays disagree about — surfaced, not
/// silently resolved.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileProblem {
    /// The profile assigns a monitor that is not present. Its role is **not**
    /// re-homed onto another display; it is named and reported.
    MonitorMissing { id: MonitorIdentity, role: String },
    /// A present monitor has no assignment in the profile. It is left uncovered
    /// and reported, rather than being guessed at.
    MonitorUnassigned {
        id: MonitorIdentity,
        name: Option<String>,
    },
}

impl std::fmt::Display for ProfileProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileProblem::MonitorMissing { id, role } => write!(
                f,
                "the profile assigns monitor {id} as {role}, but it is not connected; its \
                 role is left unfilled rather than moved to another display"
            ),
            ProfileProblem::MonitorUnassigned { id, name } => {
                let named = name.as_deref().unwrap_or("(no name reported)");
                write!(
                    f,
                    "monitor {id} ({named}) is connected but the profile assigns it no role; \
                     it is left uncovered"
                )
            }
        }
    }
}

/// The result of matching a validated profile to the monitors actually present.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedBridge {
    /// The surfaces to open, in profile order.
    pub surfaces: Vec<ResolvedSurface>,
    /// Everything that did not line up, named — see [`ProfileProblem`].
    pub problems: Vec<ProfileProblem>,
}

impl ResolvedBridge {
    /// Whether anything failed to line up.
    pub fn has_problems(&self) -> bool {
        !self.problems.is_empty()
    }

    /// The resolved viewscreen surface, if the profile assigned one and it is
    /// present.
    pub fn viewscreen(&self) -> Option<&ResolvedSurface> {
        self.surfaces
            .iter()
            .find(|s| s.role == DisplayRole::Viewscreen)
    }

    /// The resolved Station surfaces, in profile order.
    pub fn stations(&self) -> impl Iterator<Item = &ResolvedSurface> {
        self.surfaces
            .iter()
            .filter(|s| matches!(s.role, DisplayRole::Station { .. }))
    }
}

/// Match a validated profile against the monitors actually present.
///
/// A profile entry whose monitor is present becomes a [`ResolvedSurface`]
/// carrying that monitor's live geometry (so a repositioned-but-same monitor is
/// honoured — position is not part of identity, by design). A profile entry
/// whose monitor is gone becomes a [`ProfileProblem::MonitorMissing`]: its role
/// is **not** moved to a different display. A present monitor with no assignment
/// becomes a [`ProfileProblem::MonitorUnassigned`]. This is the whole of the
/// "missing or changed displays are reported explicitly and are not silently
/// replaced or rearranged" criterion: a display whose resolution *changed* has a
/// different identity, so it surfaces as one `MonitorMissing` (the old id) and
/// one `MonitorUnassigned` (the new id) — both named, nothing moved.
pub fn resolve(profile: &ValidatedProfile, discovered: &[DiscoveredMonitor]) -> ResolvedBridge {
    let by_id: std::collections::HashMap<&str, &DiscoveredMonitor> = discovered
        .iter()
        .map(|d| (d.identity.as_str(), d))
        .collect();

    let mut surfaces = Vec::new();
    let mut problems = Vec::new();
    let mut assigned: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for display in &profile.displays {
        match by_id.get(display.identity.as_str()) {
            Some(found) => {
                assigned.insert(display.identity.as_str());
                surfaces.push(ResolvedSurface {
                    identity: display.identity.clone(),
                    geometry: found.geometry.clone(),
                    role: display.role.clone(),
                    primary: found.primary,
                });
            }
            None => problems.push(ProfileProblem::MonitorMissing {
                id: display.identity.clone(),
                role: display.role.summary(),
            }),
        }
    }

    for monitor in discovered {
        if !assigned.contains(monitor.identity.as_str()) {
            problems.push(ProfileProblem::MonitorUnassigned {
                id: monitor.identity.clone(),
                name: monitor.name.clone(),
            });
        }
    }

    ResolvedBridge { surfaces, problems }
}

// ── runtime display loss (issue #1125) ──────────────────────────────────────

/// One monitor the profile assigns a role, and the participant panes that ride
/// on it (issue #1125).
///
/// Derived from a [`ValidatedProfile`] by [`ValidatedProfile::assigned_surfaces`].
/// It is the bridge between "which monitor went away" and "which panes must
/// therefore disconnect": the labels are the `--pane <NAME>` participant names a
/// Station carries, so a lost Station monitor names exactly the panes whose
/// tokens flip to Backfill.
#[derive(Clone, Debug, PartialEq)]
pub struct AssignedSurface {
    /// The stable identity of the monitor this role is assigned to.
    pub identity: MonitorIdentity,
    /// A one-line role summary, for the operator report.
    pub role: String,
    /// The participant labels whose panes live on this monitor — empty for the
    /// viewscreen, one or two for a Station.
    pub pane_labels: Vec<String>,
}

impl ValidatedProfile {
    /// The monitors this profile assigns, each with its role summary and the
    /// participant panes it carries (issue #1125).
    ///
    /// The runtime display watcher diffs this against the monitors actually
    /// present, so a monitor that was driving panes and is unplugged mid-mission
    /// names both its role (to report) and its panes (to fail).
    pub fn assigned_surfaces(&self) -> Vec<AssignedSurface> {
        self.displays
            .iter()
            .map(|d| AssignedSurface {
                identity: d.identity.clone(),
                role: d.role.summary(),
                pane_labels: match &d.role {
                    DisplayRole::Viewscreen => Vec::new(),
                    DisplayRole::Station { panes, .. } => {
                        panes.iter().map(|p| p.label.clone()).collect()
                    }
                },
            })
            .collect()
    }
}

/// A configured display that was present and driving panes, and has been lost
/// **mid-mission** (issue #1125).
///
/// The runtime companion to [`ProfileProblem::MonitorMissing`]. That one is the
/// setup-time report — a profile naming a monitor that was never connected, found
/// when a host boots. This is a monitor that *was* connected and has been
/// unplugged or lost while the mission ran; the doctrine is identical (name it
/// exactly, never re-home its role), but the consequence is new: the panes it
/// carried disconnect and their stations flip to Backfill, and the mission and
/// every other display carry on.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeDisplayLoss {
    pub identity: MonitorIdentity,
    pub role: String,
    /// The participant labels whose panes must disconnect. Empty when the
    /// viewscreen monitor is the one lost — the shared 3-D view simply has
    /// nowhere to draw until the display returns, and no participant is affected.
    pub pane_labels: Vec<String>,
}

impl std::fmt::Display for RuntimeDisplayLoss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.pane_labels.is_empty() {
            write!(
                f,
                "the {} monitor {} was connected and has been lost mid-mission; the shared 3-D \
                 view it carried now has nowhere to draw until it returns, but no station is \
                 affected — the simulation and every other surface keep running, and its role is \
                 left unfilled rather than moved to another display",
                self.role, self.identity
            )
        } else {
            write!(
                f,
                "the {} monitor {} was connected and has been lost mid-mission; the panes it \
                 carried ({}) disconnect and their stations fall back to AI control, exactly as a \
                 dropped phone's would — its role is left unfilled rather than moved to another \
                 display",
                self.role,
                self.identity,
                self.pane_labels.join(", ")
            )
        }
    }
}

/// A configured display that had been missing and is present again (issue #1125).
///
/// Reported, and nothing more: bringing a returned display back into use is an
/// **explicit** repair (re-apply the profile), never something the host does
/// silently — the same no-silent-rehome doctrine [`resolve`] holds at setup. A
/// pane is never moved onto a monitor without a deliberate act.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeDisplayReturn {
    pub identity: MonitorIdentity,
    pub role: String,
}

impl std::fmt::Display for RuntimeDisplayReturn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the {} monitor {} is connected again; it is not brought back into use automatically, \
             so re-apply the bridge profile to place its surface — no pane is moved onto it \
             without an explicit repair",
            self.role, self.identity
        )
    }
}

/// The configured monitors that were present and are now gone (issue #1125).
///
/// A loss is an assigned monitor that `previous_present` held and `current_present`
/// does not. Pure and display-free — the winit adapter reads the live monitor set
/// and calls this, so the "named exactly, never re-homed" judgement is checked by
/// the ordinary `cargo test` runs rather than only on a machine you can unplug a
/// monitor from.
pub fn runtime_display_losses(
    assigned: &[AssignedSurface],
    previous_present: &std::collections::HashSet<MonitorIdentity>,
    current_present: &std::collections::HashSet<MonitorIdentity>,
) -> Vec<RuntimeDisplayLoss> {
    assigned
        .iter()
        .filter(|a| {
            previous_present.contains(&a.identity) && !current_present.contains(&a.identity)
        })
        .map(|a| RuntimeDisplayLoss {
            identity: a.identity.clone(),
            role: a.role.clone(),
            pane_labels: a.pane_labels.clone(),
        })
        .collect()
}

/// The configured monitors that had been missing and are present again
/// (issue #1125) — see [`RuntimeDisplayReturn`].
pub fn runtime_display_returns(
    assigned: &[AssignedSurface],
    previous_present: &std::collections::HashSet<MonitorIdentity>,
    current_present: &std::collections::HashSet<MonitorIdentity>,
) -> Vec<RuntimeDisplayReturn> {
    assigned
        .iter()
        .filter(|a| {
            !previous_present.contains(&a.identity) && current_present.contains(&a.identity)
        })
        .map(|a| RuntimeDisplayReturn {
            identity: a.identity.clone(),
            role: a.role.clone(),
        })
        .collect()
}

/// Which of a profile's assigned monitors are present, matched **stably** against
/// the raw monitors this frame (issue #1125).
///
/// This is the set the runtime watcher diffs frame to frame, and it must NOT be
/// computed with [`identify`], for a subtle reason: `identify` gives a monitor a
/// *context-dependent* identity. Two identical monitors (same name and mode) each
/// get a `#x,y` position suffix while both are present, but the survivor alone
/// gets the short, position-free key. So if one of two identical Station monitors
/// is unplugged, the survivor's live-computed identity would shift from
/// `base#x1,y1` to `base`, no longer match its profile assignment, and be misread
/// as *also* lost — disconnecting a station that never moved.
///
/// So instead of re-deriving identities against the shrinking live set, this
/// matches each assignment against the raw monitors directly. A present monitor
/// satisfies *both* candidate forms of its identity — the bare `base` key and the
/// position-suffixed `base#x,y` key — so:
///
/// * a bare assignment `base` is present iff some monitor carries that base;
/// * a suffixed assignment `base#x,y` is present iff a monitor with that base
///   sits at `(x,y)`.
///
/// An assignment's presence therefore never depends on whether its identical twin
/// is still connected. Only assigned identities appear in the result — a present
/// monitor the profile says nothing about is not part of the bridge and is not
/// tracked here (its coming and going is neither a loss nor a return).
pub fn present_assigned_identities(
    assigned: &[AssignedSurface],
    raws: &[RawMonitor],
) -> std::collections::HashSet<MonitorIdentity> {
    // Every identity form a present monitor could satisfy: its position-free base
    // key AND its position-suffixed key. Matching an assignment against this set
    // makes presence independent of the live disambiguation `identify` would do.
    let mut present_forms: std::collections::HashSet<String> = std::collections::HashSet::new();
    for raw in raws {
        let base = base_key(raw);
        present_forms.insert(format!("{base}#{},{}", raw.position_x, raw.position_y));
        present_forms.insert(base);
    }
    assigned
        .iter()
        .filter(|a| present_forms.contains(a.identity.as_str()))
        .map(|a| a.identity.clone())
        .collect()
}

// ── setup report ────────────────────────────────────────────────────────────

/// Render the human-readable monitor-discovery report the `--setup` mode prints.
///
/// Pure so the whole of the setup surface's *content* is testable without a
/// display — the winit adapter's only job is to enumerate real monitors and hand
/// them here. Lists every discovered monitor with its identity, geometry and
/// current assignment; when a profile is supplied it also validates it (surfacing
/// a density or role error) and reports any missing or unassigned monitors.
///
/// The media half of the report (issue #1126) is appended by
/// [`render_setup_report_with_media`], which this delegates to with no discovered
/// devices — the default when no OS media backend is compiled in. A caller that
/// has enumerated real devices calls that function directly.
pub fn render_setup_report(
    discovered: &[DiscoveredMonitor],
    profile: Option<&BridgeProfile>,
) -> String {
    render_setup_report_with_media(discovered, &[], profile)
}

/// [`render_setup_report`] plus the media-device half (issue #1126): the display
/// report, then [`super::bridge_media::render_media_setup_report`] over
/// `media_devices`. Split out so the display half stays exactly what #1123 tests
/// assert, and so a `--setup` path that has enumerated real media devices can
/// pass them here while the default path passes none.
pub fn render_setup_report_with_media(
    discovered: &[DiscoveredMonitor],
    media_devices: &[super::bridge_media::DiscoveredMediaDevice],
    profile: Option<&BridgeProfile>,
) -> String {
    let mut out = render_display_setup_report(discovered, profile);
    out.push_str(&super::bridge_media::render_media_setup_report(
        media_devices,
        profile,
    ));
    out
}

/// The display half of the `--setup` report — the original #1123 body.
fn render_display_setup_report(
    discovered: &[DiscoveredMonitor],
    profile: Option<&BridgeProfile>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("Discovered {} monitor(s):\n", discovered.len()));

    // Resolve the profile once so each monitor line can show its assignment.
    let validated = profile.map(BridgeProfile::validate);
    let resolved = match &validated {
        Some(Ok(v)) => Some(resolve(v, discovered)),
        _ => None,
    };
    let role_for = |id: &MonitorIdentity| -> String {
        resolved
            .as_ref()
            .and_then(|r| r.surfaces.iter().find(|s| &s.identity == id))
            .map(|s| s.role.summary())
            .unwrap_or_else(|| "unassigned".to_string())
    };

    for monitor in discovered {
        let g = &monitor.geometry;
        out.push_str(&format!(
            "  [{id}]{primary}\n      name: {name}\n      geometry: {w}x{h} @ ({x},{y}), scale {scale}\n      assignment: {role}\n",
            id = monitor.identity,
            primary = if monitor.primary { "  (primary)" } else { "" },
            name = monitor.name.as_deref().unwrap_or("(no name reported)"),
            w = g.physical_width,
            h = g.physical_height,
            x = g.position_x,
            y = g.position_y,
            scale = g.scale_factor,
            role = role_for(&monitor.identity),
        ));
    }

    match &validated {
        Some(Ok(_)) => {
            if let Some(resolved) = &resolved {
                if resolved.problems.is_empty() {
                    out.push_str("\nProfile matches the connected displays.\n");
                } else {
                    out.push_str("\nProfile problems:\n");
                    for problem in &resolved.problems {
                        out.push_str(&format!("  - {problem}\n"));
                    }
                }
            }
        }
        Some(Err(e)) => {
            out.push_str(&format!("\nProfile is invalid: {e}\n"));
        }
        None => {
            out.push_str("\nNo profile supplied; every monitor is unassigned.\n");
        }
    }

    out
}

#[cfg(test)]
#[path = "bridge_profile_tests.rs"]
mod tests;
