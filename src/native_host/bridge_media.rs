//! The bridge-media profile model (issue #1126) — **pure, Bevy-free**.
//!
//! A bridge is a room of monitors ([`super::bridge_profile`]); it is also a room
//! of **media devices** — the cameras, microphones and speakers a crew talks to
//! the rest of the fleet through. This module is the media-device analogue of the
//! display profile: it writes down, once, which camera/mic(s)/output(s) each
//! bridge *surface* (the shared viewscreen, and each named Station — Comms above
//! all) captures from and plays to, so the assignment survives a reboot and an
//! operator can hand-edit it in the same TOML file the display roles live in.
//!
//! Everything here is a plain data transform with no Bevy, no OS media API and no
//! device I/O — which is the point, exactly as it is for [`super::bridge_profile`].
//! The acceptance criteria that actually have *logic* in them — a device keeps a
//! stable identity, a slot refuses a wrong-kind device, a duplicate is refused, a
//! device is shared between two surfaces only by an **explicit** choice (and then
//! warns), a missing or denied device is named rather than making the Station
//! unusable, a deterministic default is chosen when the operator has not — are all
//! decided in this file and checked by the ordinary `cargo test` CI runs. The
//! part that needs real hardware — enumerating the machine's actual devices,
//! previewing a camera, metering a microphone, tone-testing an output — is the OS
//! media backend, which this repository does **not** yet carry a crate for (see
//! [`enumerate_note`]); it is the winit-adapter analogue and is proven only by the
//! human acceptance kit, `docs/acceptance/1126-media.md`.
//!
//! # It shares the display profile's file, not its identity scheme's source
//!
//! Media assignments are a `[[media]]` table in the very same
//! [`BridgeProfile`](super::bridge_profile::BridgeProfile) TOML the `[[display]]`
//! roles live in: a bridge profile is one operator's record of the physical room,
//! and camera/mic/speaker layout belongs beside monitor layout, not in a second
//! file. It is still **not** the private per-player Accessibility profile (#1127):
//! it carries nothing about any individual, only which hardware each shared
//! surface uses.
//!
//! # The stable-identity scheme, and its limit
//!
//! A media device's stable identity here is `kind:name` ([`identify_media`]) — the
//! OS-reported kind tag (`camera` / `mic` / `output`) and the device name — so an
//! operator reads it and knows what it is, exactly as `name@WxH` does for a
//! monitor. Encoding the kind **into** the identity is load-bearing: it is what
//! lets [`validate_media`] reject a microphone dropped into the camera slot as a
//! pure string check, before any device is opened. The scheme inherits the display
//! scheme's one honest limit: two devices of the same kind and name are
//! indistinguishable by name alone and fall back to a disambiguating suffix
//! (`kind:name#2`, or `kind:name#<hardware-id>` when the OS gives one), which is
//! stable while the enumeration order holds but is not guaranteed across a
//! re-plug — the limit of what a name-based key can promise.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

// ── device kind ─────────────────────────────────────────────────────────────

/// The three kinds of media device a bridge surface is assigned, one slot each:
/// the [`Camera`](MediaKind::Camera) it captures video from, the
/// [`Microphone`](MediaKind::Microphone)(s) it captures audio from, and the
/// audio [`Output`](MediaKind::Output)(s) it plays remote audio to.
///
/// The kind is encoded as a short tag at the head of every device identity
/// (`camera:` / `mic:` / `output:`), so a slot's expected kind is checked against
/// a device id with a string comparison and no device ever has to be opened to
/// find out a profile put the wrong sort of thing in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKind {
    /// A video capture device — a webcam. One per surface.
    Camera,
    /// An audio capture device — a microphone. A surface may list more than one.
    Microphone,
    /// An audio playback device — a speaker or headset. A surface may list more
    /// than one.
    Output,
}

impl MediaKind {
    /// The short tag that heads a device identity of this kind, and names the
    /// profile slot. Stable wire/file text — do not rename without a schema bump.
    pub fn tag(&self) -> &'static str {
        match self {
            MediaKind::Camera => "camera",
            MediaKind::Microphone => "mic",
            MediaKind::Output => "output",
        }
    }

    /// Parse a kind from its identity tag. `None` for anything else.
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "camera" => Some(MediaKind::Camera),
            "mic" => Some(MediaKind::Microphone),
            "output" => Some(MediaKind::Output),
            _ => None,
        }
    }

    /// A human word for the kind, for an operator-facing message.
    pub fn label(&self) -> &'static str {
        match self {
            MediaKind::Camera => "camera",
            MediaKind::Microphone => "microphone",
            MediaKind::Output => "audio output",
        }
    }
}

// ── stable device identity ──────────────────────────────────────────────────

/// A media device's stable identity — `kind:name`, see the [module
/// note](self#the-stable-identity-scheme-and-its-limit).
///
/// A newtype over the composite key so it cannot be confused with an arbitrary
/// `String` at a call site, and so it serialises transparently — the profile's
/// device fields are the bare key, readable and hand-editable.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MediaDeviceIdentity(String);

impl MediaDeviceIdentity {
    /// Wrap a pre-computed key. Prefer [`identify_media`], which computes keys for
    /// a whole enumeration at once so it can disambiguate identical devices.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The kind this identity's tag names, if it carries a recognised one. The
    /// tag is the text before the first `:`; a key with no `:` or an unknown tag
    /// yields `None`, which [`validate_media`] reports as a malformed id.
    pub fn kind(&self) -> Option<MediaKind> {
        self.0
            .split_once(':')
            .and_then(|(tag, _)| MediaKind::from_tag(tag))
    }
}

impl std::fmt::Display for MediaDeviceIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether a discovered device can actually be used, or is present-but-refused.
///
/// The OS can report a device that exists but whose access is **denied** — a
/// camera or microphone the operator has switched off in the privacy settings, a
/// device another application holds exclusively. That is a distinct state from a
/// device that is simply gone, and issue #1126's acceptance criterion 4 names all
/// three (missing, removed, denied) as things reported without making the Station
/// unusable. A denied device resolves to a [`MediaProblem::DeviceDenied`], not a
/// silent drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceAvailability {
    /// Present and usable.
    Available,
    /// Present but access is refused (privacy setting, exclusive hold, …).
    Denied,
}

/// A media device as the OS reported it, before identities are assigned.
///
/// Lifted out of any OS media API so [`identify_media`] and every test can build
/// one by hand without a device, exactly as [`super::bridge_profile::RawMonitor`]
/// is built without a display. A real enumeration backend fills it; a test fills
/// it directly.
#[derive(Clone, Debug, PartialEq)]
pub struct RawMediaDevice {
    /// Which slot this device can fill.
    pub kind: MediaKind,
    /// The OS-reported device name, if any.
    pub name: Option<String>,
    /// A hardware-stable id, when the OS exposes one (a WASAPI endpoint id, a
    /// camera symbolic link). Used only to disambiguate two same-named devices
    /// of one kind; the readable `kind:name` stays the identity when it is unique.
    pub hardware_id: Option<String>,
    /// Whether the OS marks this the default device of its kind — the seed for a
    /// deterministic default assignment (see [`default_media_assignment`]).
    pub default: bool,
    /// Whether the device is usable or present-but-denied.
    pub availability: DeviceAvailability,
}

/// A discovered media device: its stable [`MediaDeviceIdentity`], its kind, the
/// raw name, whether it is the OS default and whether it is available — as
/// [`identify_media`] produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredMediaDevice {
    pub identity: MediaDeviceIdentity,
    pub kind: MediaKind,
    /// The OS name, kept for the setup report; the identity is derived from it.
    pub name: Option<String>,
    pub default: bool,
    pub availability: DeviceAvailability,
}

/// The placeholder used in an identity key when the OS reported no name, so the
/// key is stable across runs of a nameless device rather than varying with
/// whatever a formatter would print for `None`.
fn unnamed_for(kind: MediaKind) -> String {
    format!("unnamed-{}", kind.tag())
}

/// The position-free part of an identity: `kind:name`.
fn base_key(raw: &RawMediaDevice) -> String {
    let name = raw.name.clone().unwrap_or_else(|| unnamed_for(raw.kind));
    format!("{}:{name}", raw.kind.tag())
}

/// Assign each raw device a stable identity, preserving input order.
///
/// The identity is `kind:name` — the OS kind tag and the device name — which an
/// operator can read and paste into the matching slot of a profile. Two devices
/// of the same kind and name are indistinguishable by name alone; those, and
/// only those, are disambiguated by a suffix: `#<hardware-id>` when the OS gave
/// one (stable), else `#<ordinal>` in enumeration order (stable while the order
/// holds). A device of a unique kind+name keeps the short, suffix-free key.
///
/// The return order matches the input so a caller can zip it back against the
/// enumeration it came from.
pub fn identify_media(raws: &[RawMediaDevice]) -> Vec<DiscoveredMediaDevice> {
    // Count base keys so a unique device keeps the short key and only genuine
    // collisions pay a suffix.
    let mut counts: HashMap<String, usize> = HashMap::new();
    for raw in raws {
        *counts.entry(base_key(raw)).or_insert(0) += 1;
    }
    // A per-base running ordinal, assigned in enumeration order, for the
    // hardware-id-less fallback.
    let mut seen: HashMap<String, usize> = HashMap::new();
    raws.iter()
        .map(|raw| {
            let base = base_key(raw);
            let ordinal = {
                let n = seen.entry(base.clone()).or_insert(0);
                *n += 1;
                *n
            };
            let key = if counts.get(&base).copied().unwrap_or(0) > 1 {
                match &raw.hardware_id {
                    Some(hw) => format!("{base}#{hw}"),
                    None => format!("{base}#{ordinal}"),
                }
            } else {
                base
            };
            DiscoveredMediaDevice {
                identity: MediaDeviceIdentity(key),
                kind: raw.kind,
                name: raw.name.clone(),
                default: raw.default,
                availability: raw.availability,
            }
        })
        .collect()
}

// ── on-disk profile shape ───────────────────────────────────────────────────

/// One on-disk media assignment: the permissive shape a `[[media]]` table
/// deserialises to.
///
/// Kept separate from the validated [`ValidatedMediaSurface`] so the file can
/// round-trip without serde having to encode kind-checked ids, and so the
/// wrong-kind, duplicate and sharing rules are checked in one explicit place —
/// [`validate_media`] — rather than smeared across `Deserialize`. Every device
/// field is a bare [`MediaDeviceIdentity`] key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaSurfaceEntry {
    /// The named surface these devices are assigned to — `"viewscreen"`,
    /// `"comms"`, or a Station label. Operator-chosen; unique across the profile.
    pub surface: String,
    /// The camera this surface captures video from. One or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    /// The microphone(s) this surface captures audio from. `[[media]]` emits this
    /// as `microphone = [ … ]`.
    #[serde(default, rename = "microphone", skip_serializing_if = "Vec::is_empty")]
    pub microphones: Vec<String>,
    /// The audio output(s) this surface plays remote audio to. Emitted as
    /// `output = [ … ]`.
    #[serde(default, rename = "output", skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    /// Device ids this surface **explicitly consents** to share with another
    /// surface. A device assigned to two surfaces is refused unless every surface
    /// using it lists it here (acceptance criterion 2 — sharing is an explicit
    /// choice); a consented share is allowed and warned about.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_shared: Vec<String>,
}

// ── validated media ─────────────────────────────────────────────────────────

/// One validated media assignment: the same surface, with every device id parsed
/// to a kind-checked [`MediaDeviceIdentity`] and the consent set collected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedMediaSurface {
    pub surface: String,
    pub camera: Option<MediaDeviceIdentity>,
    pub microphones: Vec<MediaDeviceIdentity>,
    pub outputs: Vec<MediaDeviceIdentity>,
    pub allow_shared: HashSet<String>,
}

/// A profile's media assignments after validation, plus any non-fatal
/// [`MediaWarning`]s (a consented device share).
///
/// The default is the no-media case — an empty set of surfaces and no warnings —
/// which is what a display-only profile (a pre-#1126 one, or one that assigns no
/// media) validates its media half to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidatedMedia {
    pub surfaces: Vec<ValidatedMediaSurface>,
    pub warnings: Vec<MediaWarning>,
}

impl ValidatedMedia {
    /// Whether the profile assigns any media at all.
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }
}

/// A non-fatal thing worth telling the operator about a validated media profile.
///
/// A [`Contention`](MediaWarning::Contention) is the only one so far: a device
/// two surfaces both use and both consented to share. It is not an error — the
/// operator asked for it — but it is worth a line, because a single device rarely
/// captures or plays for two surfaces at once and the OS/driver may refuse the
/// second open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaWarning {
    /// A device is assigned to more than one surface by explicit consent.
    Contention { id: String, surfaces: Vec<String> },
}

impl std::fmt::Display for MediaWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaWarning::Contention { id, surfaces } => write!(
                f,
                "device {id:?} is shared by explicit consent across surfaces {}; one device rarely \
                 captures or plays for two surfaces at once, so the OS or driver may refuse the \
                 second use — confirm both work before relying on the share",
                surfaces
                    .iter()
                    .map(|s| format!("{s:?}"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
        }
    }
}

/// Why a set of `[[media]]` assignments could not be validated.
///
/// Every variant carries enough to name the offending surface and device, because
/// the acceptance criterion is a *clear* refusal, not a boolean.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaError {
    /// Two `[[media]]` entries name the same surface.
    DuplicateSurface { surface: String },
    /// A device id sits in a slot whose kind it is not: a `mic:` id in the camera
    /// slot, an `output:` id in the microphone list, and so on.
    WrongKind {
        surface: String,
        expected: MediaKind,
        found: MediaKind,
        id: String,
    },
    /// A device id carries no recognised `kind:` tag, so no slot can accept it.
    MalformedId {
        surface: String,
        expected: MediaKind,
        id: String,
    },
    /// The same device id appears twice within one surface's assignments.
    DuplicateDevice { surface: String, id: String },
    /// A device is assigned to more than one surface, but not every surface using
    /// it consented to the share (see [`MediaSurfaceEntry::allow_shared`]).
    SharedWithoutConsent { id: String, surfaces: Vec<String> },
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::DuplicateSurface { surface } => write!(
                f,
                "two media assignments both name surface {surface:?}; each surface is assigned \
                 exactly once"
            ),
            MediaError::WrongKind {
                surface,
                expected,
                found,
                id,
            } => write!(
                f,
                "surface {surface:?} puts {id:?} — a {found} — in its {expected} slot; that slot \
                 takes a {expected} device",
                expected = expected.label(),
                found = found.label(),
            ),
            MediaError::MalformedId {
                surface,
                expected,
                id,
            } => write!(
                f,
                "surface {surface:?} assigns {id:?} to its {expected} slot, but that id carries no \
                 known device kind; a device id reads {tag}:<name> (run --setup to list them)",
                expected = expected.label(),
                tag = expected.tag(),
            ),
            MediaError::DuplicateDevice { surface, id } => write!(
                f,
                "surface {surface:?} assigns device {id:?} more than once; list each device at most \
                 once per surface"
            ),
            MediaError::SharedWithoutConsent { id, surfaces } => write!(
                f,
                "device {id:?} is assigned to surfaces {}, but sharing a device is an explicit \
                 choice: add {id:?} to the allow_shared list of every surface that uses it, or give \
                 each surface its own device",
                surfaces
                    .iter()
                    .map(|s| format!("{s:?}"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
        }
    }
}

impl std::error::Error for MediaError {}

/// Validate a set of `[[media]]` assignments into a [`ValidatedMedia`].
///
/// This is where issue #1126's "clear refusal" acceptance criteria live: a
/// wrong-kind device in a slot, a malformed id, a device listed twice on one
/// surface, two surfaces named the same, and a device shared without both
/// surfaces consenting are each rejected here with an authored message, before a
/// host opens a single device. A device shared *with* both surfaces' consent is
/// allowed and returned as a [`MediaWarning::Contention`].
pub fn validate_media(entries: &[MediaSurfaceEntry]) -> Result<ValidatedMedia, MediaError> {
    let mut surfaces = Vec::with_capacity(entries.len());
    let mut seen_surfaces: HashSet<&str> = HashSet::new();
    // device id -> the surfaces that assign it, in profile order, for the
    // cross-surface sharing rule.
    let mut users: Vec<(String, Vec<String>)> = Vec::new();
    let mut user_index: HashMap<String, usize> = HashMap::new();

    for entry in entries {
        if !seen_surfaces.insert(entry.surface.as_str()) {
            return Err(MediaError::DuplicateSurface {
                surface: entry.surface.clone(),
            });
        }

        // Every device this surface uses, to catch a duplicate within the surface
        // and to feed the cross-surface map.
        let mut within: HashSet<String> = HashSet::new();
        let mut record = |id: &str, within: &mut HashSet<String>| -> Result<(), MediaError> {
            if !within.insert(id.to_string()) {
                return Err(MediaError::DuplicateDevice {
                    surface: entry.surface.clone(),
                    id: id.to_string(),
                });
            }
            match user_index.get(id) {
                Some(&i) => users[i].1.push(entry.surface.clone()),
                None => {
                    user_index.insert(id.to_string(), users.len());
                    users.push((id.to_string(), vec![entry.surface.clone()]));
                }
            }
            Ok(())
        };

        let camera = match &entry.camera {
            Some(id) => {
                check_slot(&entry.surface, MediaKind::Camera, id)?;
                record(id, &mut within)?;
                Some(MediaDeviceIdentity::new(id.clone()))
            }
            None => None,
        };
        let mut microphones = Vec::with_capacity(entry.microphones.len());
        for id in &entry.microphones {
            check_slot(&entry.surface, MediaKind::Microphone, id)?;
            record(id, &mut within)?;
            microphones.push(MediaDeviceIdentity::new(id.clone()));
        }
        let mut outputs = Vec::with_capacity(entry.outputs.len());
        for id in &entry.outputs {
            check_slot(&entry.surface, MediaKind::Output, id)?;
            record(id, &mut within)?;
            outputs.push(MediaDeviceIdentity::new(id.clone()));
        }

        surfaces.push(ValidatedMediaSurface {
            surface: entry.surface.clone(),
            camera,
            microphones,
            outputs,
            allow_shared: entry.allow_shared.iter().cloned().collect(),
        });
    }

    // The sharing rule: a device used by more than one surface must be consented
    // to by every surface using it, and then warns; otherwise it is refused.
    let consent: HashMap<&str, &HashSet<String>> = surfaces
        .iter()
        .map(|s| (s.surface.as_str(), &s.allow_shared))
        .collect();
    let mut warnings = Vec::new();
    for (id, using) in &users {
        if using.len() < 2 {
            continue;
        }
        let all_consent = using.iter().all(|surface| {
            consent
                .get(surface.as_str())
                .is_some_and(|set| set.contains(id))
        });
        if all_consent {
            warnings.push(MediaWarning::Contention {
                id: id.clone(),
                surfaces: using.clone(),
            });
        } else {
            return Err(MediaError::SharedWithoutConsent {
                id: id.clone(),
                surfaces: using.clone(),
            });
        }
    }

    Ok(ValidatedMedia { surfaces, warnings })
}

/// Check that `id` is a well-formed identity of the kind `expected` its slot
/// takes.
fn check_slot(surface: &str, expected: MediaKind, id: &str) -> Result<(), MediaError> {
    match MediaDeviceIdentity::new(id).kind() {
        Some(found) if found == expected => Ok(()),
        Some(found) => Err(MediaError::WrongKind {
            surface: surface.to_string(),
            expected,
            found,
            id: id.to_string(),
        }),
        None => Err(MediaError::MalformedId {
            surface: surface.to_string(),
            expected,
            id: id.to_string(),
        }),
    }
}

// ── resolution against present devices ──────────────────────────────────────

/// One surface's media, matched against the devices actually present: only the
/// present-and-available devices are carried, so the surface is always usable.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMediaSurface {
    pub surface: String,
    pub camera: Option<MediaDeviceIdentity>,
    pub microphones: Vec<MediaDeviceIdentity>,
    pub outputs: Vec<MediaDeviceIdentity>,
}

/// Something the media profile and the present devices disagree about — surfaced,
/// never fatal. A surface with a problem still resolves to whatever devices it
/// *does* have, so a missing or denied device never makes the Station unusable
/// (acceptance criterion 4).
#[derive(Clone, Debug, PartialEq)]
pub enum MediaProblem {
    /// A surface assigns a device that is not present (never connected, or
    /// removed). Its slot is left unfilled and reported; nothing is substituted.
    DeviceMissing {
        surface: String,
        kind: MediaKind,
        id: String,
    },
    /// A surface assigns a device that is present but access is denied. Reported,
    /// and the surface goes on with its other devices.
    DeviceDenied {
        surface: String,
        kind: MediaKind,
        id: String,
    },
}

impl std::fmt::Display for MediaProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaProblem::DeviceMissing { surface, kind, id } => write!(
                f,
                "surface {surface:?} is assigned {kind} {id:?}, but it is not connected; that slot \
                 is left unfilled — the surface keeps working with its other devices, and nothing \
                 is substituted",
                kind = kind.label(),
            ),
            MediaProblem::DeviceDenied { surface, kind, id } => write!(
                f,
                "surface {surface:?} is assigned {kind} {id:?}, but access to it is denied (a \
                 privacy setting, or another app holds it); that slot is left unfilled — the \
                 surface stays usable",
                kind = kind.label(),
            ),
        }
    }
}

/// The result of matching validated media against the devices actually present.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMedia {
    /// One entry per assigned surface, in profile order, carrying only its
    /// present-and-available devices.
    pub surfaces: Vec<ResolvedMediaSurface>,
    /// Everything that did not line up, named — see [`MediaProblem`].
    pub problems: Vec<MediaProblem>,
}

impl ResolvedMedia {
    /// Whether anything failed to line up. Not fatal — a surface with a problem
    /// is still present and usable.
    pub fn has_problems(&self) -> bool {
        !self.problems.is_empty()
    }
}

/// Match validated media against the devices actually present.
///
/// Each assigned device that is present and available is carried into the
/// resolved surface. A device that is **missing** (never connected, or removed)
/// becomes a [`MediaProblem::DeviceMissing`]; one that is present but **denied**
/// becomes a [`MediaProblem::DeviceDenied`]. In both cases the slot is left
/// unfilled and the surface keeps every device it *does* have — this is the whole
/// of "missing, removed or denied media devices are reported but never make the
/// Station itself unusable". A present device the profile assigns to no surface is
/// not a problem: a bridge need not use every device it can see.
pub fn resolve_media(
    media: &ValidatedMedia,
    discovered: &[DiscoveredMediaDevice],
) -> ResolvedMedia {
    let by_id: HashMap<&str, &DiscoveredMediaDevice> = discovered
        .iter()
        .map(|d| (d.identity.as_str(), d))
        .collect();

    let mut surfaces = Vec::with_capacity(media.surfaces.len());
    let mut problems = Vec::new();

    for surface in &media.surfaces {
        let mut resolve_one =
            |id: &MediaDeviceIdentity, kind: MediaKind| -> Option<MediaDeviceIdentity> {
                match by_id.get(id.as_str()) {
                    Some(d) if d.availability == DeviceAvailability::Available => Some(id.clone()),
                    Some(_) => {
                        problems.push(MediaProblem::DeviceDenied {
                            surface: surface.surface.clone(),
                            kind,
                            id: id.as_str().to_string(),
                        });
                        None
                    }
                    None => {
                        problems.push(MediaProblem::DeviceMissing {
                            surface: surface.surface.clone(),
                            kind,
                            id: id.as_str().to_string(),
                        });
                        None
                    }
                }
            };

        let camera = surface
            .camera
            .as_ref()
            .and_then(|id| resolve_one(id, MediaKind::Camera));
        let microphones = surface
            .microphones
            .iter()
            .filter_map(|id| resolve_one(id, MediaKind::Microphone))
            .collect();
        let outputs = surface
            .outputs
            .iter()
            .filter_map(|id| resolve_one(id, MediaKind::Output))
            .collect();

        surfaces.push(ResolvedMediaSurface {
            surface: surface.surface.clone(),
            camera,
            microphones,
            outputs,
        });
    }

    ResolvedMedia { surfaces, problems }
}

// ── deterministic default assignment ────────────────────────────────────────

/// Build a deterministic default media assignment for `surfaces`, from the
/// devices actually present — the answer to "the operator has not chosen".
///
/// Each surface is given the OS **default** device of each kind, falling back to
/// the first device of that kind in enumeration order; a kind with no device is
/// simply left unassigned. Because a one-device machine has only that device to
/// give every surface, any device that ends up on more than one surface is added
/// to each surface's `allow_shared` — so the generated default validates cleanly,
/// with an explicit-consent [`MediaWarning::Contention`] rather than a
/// [`MediaError::SharedWithoutConsent`]. The two surfaces are still distinct
/// endpoints; they merely share the one physical device the box has.
pub fn default_media_assignment(
    surfaces: &[&str],
    discovered: &[DiscoveredMediaDevice],
) -> Vec<MediaSurfaceEntry> {
    let pick = |kind: MediaKind| -> Option<String> {
        discovered
            .iter()
            .filter(|d| d.kind == kind && d.availability == DeviceAvailability::Available)
            .find(|d| d.default)
            .or_else(|| {
                discovered
                    .iter()
                    .find(|d| d.kind == kind && d.availability == DeviceAvailability::Available)
            })
            .map(|d| d.identity.as_str().to_string())
    };
    let camera = pick(MediaKind::Camera);
    let microphone = pick(MediaKind::Microphone);
    let output = pick(MediaKind::Output);

    // A device is shared iff more than one surface would receive it — which, with
    // one default per kind, is exactly "there is more than one surface and the
    // device exists".
    let shared_when_many: Vec<String> = if surfaces.len() > 1 {
        [&camera, &microphone, &output]
            .into_iter()
            .flatten()
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    surfaces
        .iter()
        .map(|surface| MediaSurfaceEntry {
            surface: (*surface).to_string(),
            camera: camera.clone(),
            microphones: microphone.iter().cloned().collect(),
            outputs: output.iter().cloned().collect(),
            allow_shared: shared_when_many.clone(),
        })
        .collect()
}

// ── setup report ────────────────────────────────────────────────────────────

/// The line `--setup` prints in place of a device list when no OS media backend
/// is compiled into the build (which is every build in this repository today —
/// see [`enumerate_note`]).
const NO_BACKEND_NOTE: &str = "  (no media-device backend is compiled into this build, so no live \
    devices are listed; author media assignments by hand from the device names Windows shows, \
    and see docs/acceptance/1126-media.md)";

/// A one-line note stating where a real OS media enumeration backend would plug
/// in, and why there is not one yet.
///
/// The display profile gets its real enumeration free from Bevy's `Monitor`
/// component (#1123); there is no equivalent already-present source for cameras,
/// microphones and audio outputs, and this repository carries **no** native media
/// device crate. Wiring one — `cpal` for microphones and audio outputs, a camera
/// crate such as `nokhwa` (or Windows Media Foundation directly) for cameras —
/// behind the `host`/`ultralight` feature seam is the winit-adapter analogue of
/// #1126, out of the CI default build exactly as the real-monitor surface is. The
/// pure model in this file is complete and CI-tested without it; live
/// enumeration, camera preview, mic metering and output tone-test are the human
/// acceptance kit's, `docs/acceptance/1126-media.md`.
pub fn enumerate_note() -> &'static str {
    NO_BACKEND_NOTE
}

/// Render the media half of the `--setup` report.
///
/// Pure, so the whole of the setup surface's media *content* is testable without
/// a device backend: a real enumerator (when one exists) hands its devices here.
/// Lists every discovered device by kind; validates the profile's `[[media]]`
/// assignments (surfacing a wrong-kind, duplicate or sharing error, and any
/// consented-share warning); and when devices are present, resolves the
/// assignments against them and reports any missing or denied device. With no
/// devices — no backend — it prints [`enumerate_note`] and still validates the
/// assignments, so an operator authoring by hand still gets the kind/duplicate
/// checks.
pub fn render_media_setup_report(
    discovered: &[DiscoveredMediaDevice],
    profile: Option<&super::bridge_profile::BridgeProfile>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\nMedia devices — discovered {} device(s):\n",
        discovered.len()
    ));
    if discovered.is_empty() {
        out.push_str(enumerate_note());
        out.push('\n');
    } else {
        for kind in [MediaKind::Camera, MediaKind::Microphone, MediaKind::Output] {
            let of_kind: Vec<&DiscoveredMediaDevice> =
                discovered.iter().filter(|d| d.kind == kind).collect();
            out.push_str(&format!("  {}(s):\n", kind.label()));
            if of_kind.is_empty() {
                out.push_str("      (none)\n");
            }
            for d in of_kind {
                out.push_str(&format!(
                    "      [{id}]{default}{denied}\n          name: {name}\n",
                    id = d.identity,
                    default = if d.default { "  (default)" } else { "" },
                    denied = if d.availability == DeviceAvailability::Denied {
                        "  (access denied)"
                    } else {
                        ""
                    },
                    name = d.name.as_deref().unwrap_or("(no name reported)"),
                ));
            }
        }
    }

    let media_entries = profile.map(|p| p.media.as_slice()).unwrap_or(&[]);
    if media_entries.is_empty() {
        out.push_str("\nNo media assignments in the profile.\n");
        return out;
    }

    match validate_media(media_entries) {
        Ok(validated) => {
            out.push_str("\nMedia assignments:\n");
            for surface in &validated.surfaces {
                out.push_str(&format!(
                    "  surface {:?}: {}\n",
                    surface.surface,
                    describe_surface(surface)
                ));
            }
            for warning in &validated.warnings {
                out.push_str(&format!("  - warning: {warning}\n"));
            }
            if !discovered.is_empty() {
                let resolved = resolve_media(&validated, discovered);
                if resolved.problems.is_empty() {
                    out.push_str("Media assignments match the connected devices.\n");
                } else {
                    out.push_str("Media problems:\n");
                    for problem in &resolved.problems {
                        out.push_str(&format!("  - {problem}\n"));
                    }
                }
            }
        }
        Err(e) => {
            out.push_str(&format!("\nMedia assignments are invalid: {e}\n"));
        }
    }

    out
}

/// A one-line human summary of a validated surface's device slots, for the setup
/// report and the boot log.
fn describe_surface(surface: &ValidatedMediaSurface) -> String {
    let camera = surface
        .camera
        .as_ref()
        .map(|c| c.as_str())
        .unwrap_or("(none)");
    let mics = if surface.microphones.is_empty() {
        "(none)".to_string()
    } else {
        surface
            .microphones
            .iter()
            .map(|m| m.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let outs = if surface.outputs.is_empty() {
        "(none)".to_string()
    } else {
        surface
            .outputs
            .iter()
            .map(|o| o.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!("camera {camera}, microphone(s) {mics}, output(s) {outs}")
}

#[cfg(test)]
#[path = "bridge_media_tests.rs"]
mod tests;
