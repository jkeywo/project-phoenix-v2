//! Stable identity and hotplug rematching of OS monitor descriptions.
use serde::{Deserialize, Serialize};
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
pub use crate::geometry::MonitorGeometry;

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
///
/// **This answer is context-dependent, and a live reader must not compare it
/// against a stored one.** Both exceptions above are decided by *this frame's*
/// set: a display gains its `#x,y` suffix only while its twin is present, and a
/// base key changes outright with a renegotiated resolution. A caller that has
/// to recognise the same screen frame after frame wants
/// [`identify_stable`] (a whole roster) or [`present_assigned_identities`] (a
/// profile's assignments), both of which match by *form* instead.
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

/// The position-suffixed form of an identity: `name@WxH#x,y`.
fn positioned_key(raw: &RawMonitor) -> String {
    format!("{}#{},{}", base_key(raw), raw.position_x, raw.position_y)
}

/// **Both** identity forms a present monitor answers to — its position-free
/// base key and its position-suffixed key.
///
/// The one place that pair is spelled, because two readers of a live monitor
/// set that disagree about which forms a display satisfies is exactly the class
/// of bug this whole scheme exists to prevent. [`present_assigned_identities`]
/// asks "is this assignment still on screen"; [`identify_stable`] asks "which
/// display is the one I already knew" — the same question about the same
/// context-dependence of [`identify`], asked by the two halves of the runtime
/// watcher.
pub fn identity_forms(raw: &RawMonitor) -> [String; 2] {
    [positioned_key(raw), base_key(raw)]
}

// ── identity that survives a roster change (issue #1330) ────────────────────

/// How hard one pass of [`identify_stable`] is willing to look for a monitor the
/// caller already knew.
///
/// Ordered strongest first, and every pass but the last is anchored to the
/// position the display was last seen at, so a weaker match can never take the
/// screen a stronger one names.
#[derive(Clone, Copy)]
enum Rematch {
    /// The known identity **is** this monitor's position-suffixed form. At most
    /// one *present* monitor can satisfy it — no two displays share a top-left
    /// corner — so this pass never has two candidates to choose between.
    ///
    /// That is all it promises. It does not promise the one candidate is the
    /// same physical display: a twin dragged into the corner the known display
    /// used to occupy answers to that key too, and this pass would hand it the
    /// role. Anchoring to the position is the strongest evidence a monitor list
    /// can offer, not proof of identity.
    Suffixed,
    /// The known identity is this monitor's base key and it has not moved. The
    /// twin case: the display an operator has been looking at keeps the short
    /// key when its identical twin arrives beside it and forces `identify` to
    /// start suffixing.
    BaseHere,
    /// Same OS name, same position, different mode — a display that
    /// renegotiated its resolution (a television waking, an EDID handshake
    /// settling). Its base key changed with `WxH`, so no form match can find
    /// it, but it is plainly the same screen in the same place.
    RenegotiatedHere,
    /// The known identity is this monitor's base key, wherever it now sits —
    /// the OS-settings rearrange [`identify`]'s whole scheme is built to
    /// survive.
    BaseAnywhere,
}

impl Rematch {
    /// Every pass, strongest first.
    const ALL: [Rematch; 4] = [
        Rematch::Suffixed,
        Rematch::BaseHere,
        Rematch::RenegotiatedHere,
        Rematch::BaseAnywhere,
    ];

    /// Whether `raw` is, under this pass, the display `known` names.
    fn matches(self, raw: &RawMonitor, known: &DiscoveredMonitor) -> bool {
        let here = raw.position_x == known.geometry.position_x
            && raw.position_y == known.geometry.position_y;
        let id = known.identity.as_str();
        match self {
            Rematch::Suffixed => id == positioned_key(raw),
            Rematch::BaseHere => here && id == base_key(raw),
            Rematch::RenegotiatedHere => here && raw.name == known.name,
            Rematch::BaseAnywhere => id == base_key(raw),
        }
    }
}

/// Identify the monitors present now, keeping the identity a display the caller
/// **already knew** was known by (issue #1330).
///
/// [`identify`] answers a *context-dependent* key: a display's identity gains
/// its `#x,y` suffix only while an identical twin is present, and loses it again
/// the moment that twin leaves — and a base key changes outright when the
/// display renegotiates its resolution. That is harmless for a one-shot
/// enumeration and actively wrong for anything that has to recognise the *same*
/// screen frame after frame, because a re-derived string compared against a
/// stored one reads an unchanged display as one that went away and a new one
/// that arrived. [`present_assigned_identities`] solves that for a profile's
/// assignments; this solves it for a whole live roster, which is what the
/// lobby's monitor row and the bridge layout are rebuilt from.
///
/// Each display in `known` claims at most one present monitor, in four passes
/// (see [`Rematch`]) so a weak, position-free match can never take the screen a
/// stronger, position-anchored one names. A monitor no known display claimed
/// gets the identity [`identify`] would have given it — that is a display that
/// genuinely just arrived — and a known display that claimed nothing is
/// genuinely absent, which is what makes an unplug still read as an unplug.
///
/// Two changes at once — a display that moved **and** changed its mode — are
/// deliberately not chased: nothing anchors the match, and guessing would
/// re-home the viewscreen onto a screen the operator did not choose. That reads
/// as one display leaving and another arriving, which is the conservative
/// answer and the one #1123's doctrine already gives.
///
/// `known` empty makes this exactly [`identify`], which is what the boot path
/// (with nothing yet known) passes.
pub fn identify_stable(raws: &[RawMonitor], known: &[DiscoveredMonitor]) -> Vec<DiscoveredMonitor> {
    let fresh = identify(raws);
    let mut carried: Vec<Option<MonitorIdentity>> = vec![None; raws.len()];
    let mut taken: std::collections::HashSet<MonitorIdentity> = std::collections::HashSet::new();

    for pass in Rematch::ALL {
        for k in known {
            if taken.contains(&k.identity) {
                continue;
            }
            let Some(index) = raws
                .iter()
                .enumerate()
                .find(|(i, raw)| carried[*i].is_none() && pass.matches(raw, k))
                .map(|(i, _)| i)
            else {
                continue;
            };
            carried[index] = Some(k.identity.clone());
            taken.insert(k.identity.clone());
        }
    }

    fresh
        .into_iter()
        .enumerate()
        .map(|(i, mut found)| {
            found.identity = match carried[i].take() {
                Some(carried) => carried,
                // Every carried identity is already in `taken`, so a newcomer
                // can only collide with one of those — never with another
                // newcomer, which `identify` has already told apart.
                None => {
                    let unique = free_identity(&raws[i], found.identity, &taken);
                    taken.insert(unique.clone());
                    unique
                }
            };
            found
        })
        .collect()
}

/// `preferred`, or a key close to it that nothing has claimed.
///
/// Only reachable when a display carrying an identity forward has kept the key a
/// newly-arrived one would otherwise derive — a monitor that renegotiated its
/// mode, and then a second monitor arriving in the mode the first one left. The
/// position suffix settles it, since no two displays share a top-left corner;
/// the counted tail after that is unreachable in practice and exists so this
/// cannot return a duplicate, which the layout would silently deduplicate into a
/// display with no button.
fn free_identity(
    raw: &RawMonitor,
    preferred: MonitorIdentity,
    taken: &std::collections::HashSet<MonitorIdentity>,
) -> MonitorIdentity {
    if !taken.contains(&preferred) {
        return preferred;
    }
    let positioned = MonitorIdentity(positioned_key(raw));
    if !taken.contains(&positioned) {
        return positioned;
    }
    for n in 2.. {
        let candidate = MonitorIdentity(format!("{}#{n}", positioned.as_str()));
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("the counted tail is unbounded")
}
