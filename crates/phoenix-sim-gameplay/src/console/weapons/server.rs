/// The firing posture every weapons host seeds the `red_alert` fact from
/// (issue #1041): the ship's Red Alert, and whether the firing system can shoot
/// at all — since issue #1396, whether its authored power group is COLD.
///
/// # Why a posture rather than a second fact
///
/// Issue #1041's acceptance criteria require the restraint lever to compose with
/// the authored fire gate **with no new doctrine vocabulary**. Every armed hull
/// in the fleet writes exactly one gate —
/// `when = "fact(red_alert) >= param(min_alert_to_fire)"` — and differs only in
/// the threshold: an Alliance hull with a captain's console authors `1`, the
/// always-armed Harrow gun line authors `0` (issue #872). A second fact would
/// mean editing every one of those predicates to AND it in, which is precisely
/// the vocabulary change the AC forbids, and a hull whose author forgot would
/// silently ignore the order.
///
/// So restraint is composed into the VALUE of the fact the gate already reads.
/// [`Self::alert_fact_value`] is the whole mechanism:
///
/// | posture | fact | `>= 0` (Harrow) | `>= 1` (Alliance) |
/// |---|---|---|---|
/// | stood down | `0.0` | fires | holds |
/// | red alert | `1.0` | fires | fires |
/// | **weapons cold** | [`WEAPONS_COLD_ALERT_FACT`] | **holds** | **holds** |
///
/// Read the numbers as a LADDER rather than as a boolean with a sentinel bolted
/// on: `min_alert_to_fire` is a floor on how hot the ship must be before a bank
/// will open up, and a switched-off gun is a rung BELOW stood-down — colder than
/// cold, because a stood-down ship is merely not expecting trouble while a cold
/// group has no power to shoot with. Sitting under every authored floor is what
/// makes the lever work on the always-armed hulls too, which a `0.0` could not
/// do: `0 >= 0` is true, and a Harrow with dead guns would have kept firing.
/// That is the AI/human symmetry half of the AC — the NPC fire hosts respect it
/// identically because they read the same seeded fact through the same authored
/// predicate, not because anything checks who is flying.
///
/// # Per SYSTEM, not per ship (issue #1396)
///
/// [`Self::weapons_cold`] is a reading of ONE firing system's own
/// `[[system]].power_group`, resolved through
/// [`system_power_group_is_cold`], so the posture is built inside each host's
/// per-bank / per-tube loop rather than once per ship. A hull is free to author
/// its tubes onto a different group than its beams; the gate then closes on the
/// half that is switched off and leaves the other half shooting. Every shipped
/// hull puts all of it on `weapons` today, so the fleet behaves as if the
/// reading were ship-wide — but the authoring granularity is the system's, and
/// so is the gate's.
///
/// # Power is the ONLY restraint lever (issue #1398)
///
/// There was a second occupant on this rung until #1398: the captain's
/// `ShipWeaponsHold`, a hidden per-ship boolean set by a Captain-console button
/// and by the Rhai `hold_fire()` / `release_fire()` verbs. It is gone. The verbs
/// kept their names and became POWER orders on the named ship's `weapons` group
/// (`crate::ship::power::drain_scripted_power_orders`), so a scenario silences a
/// hull the same way an Engineering officer does and the fleet has one lever
/// instead of two that had to be read together.
///
/// # Why Red Alert's own behaviour cannot have moved
///
/// With nothing cold this returns exactly `1.0` / `0.0` — bit for bit the
/// expression each host inlined before #1041. Nothing else in the fire path
/// changed, so a run in which no group goes cold folds the same digest it always
/// did; the committed world anchors are the standing proof.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponsAlertPosture {
    /// This ship's own [`crate::ship::state::ShipRedAlert`].
    pub red_alert: bool,
    /// True when the power group this firing system authors is at level 0 —
    /// switched off, not turned down (issue #1396). Read through
    /// [`system_power_group_is_cold`], so a hull that authors no group for the
    /// system, or a fixture that spawns no reactor, reads `false`.
    pub weapons_cold: bool,
    /// A Command stance directing this weapons Station's alert posture (issue
    /// #1107), overriding the ship's own Red Alert for the fire gate. `None`
    /// means no Command stance is in force → the posture tracks `red_alert`
    /// exactly as before this issue, so an undirected hull stays byte-identical.
    pub stance_high_alert: Option<bool>,
}

/// The `red_alert` fact value seeded at a weapons host while the firing system
/// cannot shoot — one rung BELOW stood-down, so it fails every `>= threshold`
/// gate a hull can author, the always-armed `0` included.
///
/// Named for the cold-power reading since issue #1396 (it was
/// `WEAPONS_HOLD_ALERT_FACT`), and **the value is unchanged**: power at 0 is the
/// same rung the captain's hold occupied, which is what lets the restraint lever
/// move from a hidden toggle to the reactor without re-authoring a single
/// `min_alert_to_fire` in the fleet.
///
/// Not authored TOML on purpose: it is not a tunable but the bottom of the
/// ladder the authored `min_alert_to_fire` floors sit on, and a designer moving
/// it could only ever break the lever. `authored_ai_pins` pins every shipped
/// bank's threshold at or above zero so this stays true of the shipped fleet.
pub const WEAPONS_COLD_ALERT_FACT: f64 = -1.0;

/// Is the power group authored on `system_id` switched off on this ship?
///
/// The one place the fire path turns `[[system]].power_group` into a yes/no
/// (issue #1396). Two readings compose here and neither is hardcoded: the hull's
/// TOML says which group a gun draws from, and the ship's own reactor says what
/// that group is at. No Rust constant names a group — a hull that puts its tubes
/// on `ordnance` gets exactly the gate its author asked for.
///
/// **Fails open on absence**, in both arguments, exactly as the `Option<&_>`
/// posture components beside it do: a bare-`App` fixture with no reactor and no
/// ship config is not a ship with cold guns, it is a ship the question cannot be
/// asked of, and a fire path that refused there would silently break ~2500 unit
/// fixtures rather than gate anything. A system whose hull authors no
/// `power_group` is likewise never cold: it is not on the reactor's books at
/// all. [`crate::modifiers::power_system::PowerSystem::is_group_cold`] makes the
/// same choice one level down for a group the reactor does not track.
pub fn system_power_group_is_cold(
    ship_config: Option<&crate::ship::config::ShipConfig>,
    power: Option<&crate::modifiers::power_system::PowerSystem>,
    system_id: &crate::core::messages::SystemId,
) -> bool {
    let (Some(ship_config), Some(power)) = (ship_config, power) else {
        return false;
    };
    ship_config
        .power_group_for(system_id)
        .is_some_and(|group| power.is_group_cold(group))
}

impl WeaponsAlertPosture {
    /// Read the posture off one ship's alert and one system's cold reading.
    /// An absent alert reads as "not at alert" — the same fail-open reading the
    /// hosts used before #1041, so a bare-`App` fixture that spawns none behaves
    /// exactly as it did.
    pub fn from_components(
        red_alert: Option<&crate::ship::state::ShipRedAlert>,
        weapons_cold: bool,
    ) -> Self {
        Self::from_parts(red_alert, weapons_cold, None)
    }

    /// As [`from_components`](Self::from_components) but with a Command stance
    /// override (issue #1107). `stance_high_alert: None` is exactly
    /// `from_components`, so every pre-#1107 call site keeps its behaviour.
    pub fn from_parts(
        red_alert: Option<&crate::ship::state::ShipRedAlert>,
        weapons_cold: bool,
        stance_high_alert: Option<bool>,
    ) -> Self {
        Self {
            red_alert: red_alert.is_some_and(|r| r.0),
            weapons_cold,
            stance_high_alert,
        }
    }

    /// A posture with live guns — the shorthand every test written before #1041
    /// keeps working through, and the one the byte-identical claim rests on.
    pub fn alert(red_alert: bool) -> Self {
        Self {
            red_alert,
            weapons_cold: false,
            stance_high_alert: None,
        }
    }

    /// The value seeded for the `red_alert` fact. See the type docs for the
    /// ladder this implements.
    ///
    /// A gun that cannot shoot wins over everything: a cold power group (issue
    /// #1396) sits under every authored floor, so **Red Alert cannot override
    /// it** — raising the alert on
    /// a ship whose weapons are switched off changes the fact from
    /// [`WEAPONS_COLD_ALERT_FACT`] to nothing at all. Otherwise a Command stance
    /// override, when present, decides the posture in place of the ship's own Red
    /// Alert — the seam the migrated Red Alert fire branch travels through (issue
    /// #1107). With no override the reading is the pre-#1107 `red_alert` value,
    /// bit for bit.
    pub fn alert_fact_value(self) -> f64 {
        if self.weapons_cold {
            WEAPONS_COLD_ALERT_FACT
        } else if let Some(high) = self.stance_high_alert {
            if high {
                1.0
            } else {
                0.0
            }
        } else if self.red_alert {
            1.0
        } else {
            0.0
        }
    }
}
