//! The pure, Bevy-free heart of the rescue transporter (issue #1348, PRD #1337).
//!
//! Two things live here and nothing else: the authored `[transporter]` config
//! ([`TransporterConfig`]) with its validation, and the pure **transport
//! verdict** ([`transport_status`]) that decides, from live scalars the adapter
//! reads off the world, whether the transporter may recover civilians this tick
//! and, if not, the one refusal the console shows.
//!
//! # Why this is a module of its own, Bevy-free
//!
//! AGENTS.md rule 10: the verdict is decided here, in isolation, and unit-tested
//! here; the sibling [`crate::transporter::server`] adapter gathers the real
//! components, calls in, and applies what comes back, deciding nothing itself.
//! This is the exact split [`crate::tractor::coupling`] keeps, and this slice
//! copies it: a first-class engineering-owned `[[system]]` with a pure verdict
//! and a Bevy adapter.
//!
//! # The transporter is NOT the tractor
//!
//! The tractor couples to whatever Tactical currently has locked; the
//! transporter instead names its OWN target (`TransportSelectContact { uuid }`),
//! because a rescue is a deliberate act against a specific discovered contact,
//! not against the combat lock. And a rescue is TIME-EXTENDED — it recovers
//! civilians at an authored rate over many ticks — where a tractor hold is a
//! per-tick geometry. So the config carries a rate and the verdict carries a
//! "nothing left to recover" refusal the tractor has no analogue for.

use serde::{Deserialize, Serialize};

/// The authored transporter terms for a hull — its `[transporter]` table (issue
/// #1348).
///
/// Every field is a designer's number, read from TOML: AGENTS.md rule 11, no
/// hardcoded gameplay values. A hull that authors no `[transporter]` table
/// carries no [`crate::transporter::server::Transporter`] component and is
/// unchanged in every way. The **power group** the transporter draws from is
/// NOT here — it is the `power_group` field of the transporter `[[system]]`
/// block, the one authoritative place a system names its group — and the adapter
/// resolves it at spawn so the two can never drift.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransporterConfig {
    /// The furthest a discovered contact may sit from the operator and still be
    /// transported, in world units, centre to centre — the same measure the
    /// tractor's `range` and a scan band's `max_range` take. Drifting past it
    /// interrupts the transport ([`TransportRefusal::OutOfRange`]).
    pub range: f32,
    /// How long the transporter takes to recover ONE civilian, in seconds, while
    /// eligible and running. The authored RATE, expressed as a duration per
    /// civilian so a designer tunes "a soul a second" directly rather than a
    /// coefficient: a contact carrying `n` civilians is fully recovered after
    /// `n * seconds_per_civilian` seconds of continuous eligible transport, and
    /// the progress the crew watch is the accrual toward the next one.
    pub seconds_per_civilian: f32,
    /// The lowest power-group level at which the transporter runs. Below it the
    /// transport interrupts ([`TransportRefusal::Unpowered`]). Authored, not
    /// derived from the group's nominal rung, so a hull can make its transporter
    /// cheap or dear independent of what else shares the group.
    pub min_power_level: u8,
}

impl TransporterConfig {
    /// Reject an authored `[transporter]` table that describes a transporter that
    /// could never recover anyone (issue #1348). A non-positive range or
    /// per-civilian duration, or a zero minimum power level (which would let a
    /// wholly unpowered transporter run), are author mistakes whose only other
    /// symptom would be a control the crew can press that quietly never moves a
    /// soul.
    pub fn validate(&self) -> Result<(), String> {
        if !self.range.is_finite() || self.range <= 0.0 {
            return Err(format!(
                "[transporter] range must be a positive distance, got {}",
                self.range
            ));
        }
        if !self.seconds_per_civilian.is_finite() || self.seconds_per_civilian <= 0.0 {
            return Err(format!(
                "[transporter] seconds_per_civilian must be a positive duration, got {}",
                self.seconds_per_civilian
            ));
        }
        if self.min_power_level == 0 {
            return Err(
                "[transporter] min_power_level must be at least 1 — a transporter that runs at \
                 level 0 would never lose its allocation"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// The authored `[civilian_rescue]` table on a scannable contact entity (issue
/// #1348) — the civilians it carries, waiting to be discovered and recovered.
///
/// The mirror of [`TransporterConfig`]: that table says what a hull can do the
/// rescuing WITH, this one says what a contact offers to be rescued. Absent for
/// every entity that authors nothing, which carries no
/// [`crate::transporter::server::CivilianRescue`] component and is unchanged in
/// every way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CivilianRescueConfig {
    /// How many civilians the contact carries. The authored figure the scan
    /// reveals and the transporter recovers.
    pub count: u32,
}

impl CivilianRescueConfig {
    /// Reject a `[civilian_rescue]` table that carries nobody (issue #1348): a
    /// zero count is a contact whose only symptom would be a rescue objective the
    /// crew can never satisfy, or a discovery beat that announces an empty hold.
    pub fn validate(&self) -> Result<(), String> {
        if self.count == 0 {
            return Err(
                "[civilian_rescue] count must be at least 1 — a contact carrying nobody should \
                 omit the table"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// The one reason a rescue transport did not run (or interrupted) this tick
/// (issue #1348), as the console shows it — a `strings.csv` id, never English.
///
/// Mirrors [`crate::tractor::coupling::TractorRefusal`]'s refusal-plus-
/// `string_id` shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportRefusal {
    /// Nothing is selected, so the transporter has no contact to lock onto.
    NoContact,
    /// A contact is selected but Sensors have not yet completed the revealing
    /// scan — the life signs are not confirmed, so there is nothing authoritative
    /// to beam up. The "announce only after the scan" gate, as a refusal.
    NotDiscovered,
    /// The selected contact carries no unrecovered civilians — everyone is
    /// already aboard.
    NoCivilians,
    /// The selected contact sits further than the authored `range`.
    OutOfRange,
    /// The transporter's power group is below the authored `min_power_level`.
    Unpowered,
    /// The transporter system is damaged to `Disabled` (or `Destroyed`).
    Disabled,
}

impl TransportRefusal {
    /// Every refusal, in the order a console legend would read them.
    pub const ALL: [Self; 6] = [
        Self::NoContact,
        Self::NotDiscovered,
        Self::NoCivilians,
        Self::OutOfRange,
        Self::Unpowered,
        Self::Disabled,
    ];

    /// The `strings.csv` id the console resolves through `t()`. A `match`, not a
    /// composed `format!("transporter.refused.{...}")`, so `check-strings.mjs`
    /// can see every id a new variant needs a row for.
    pub fn string_id(self) -> &'static str {
        match self {
            TransportRefusal::NoContact => "transporter.refused.no_contact",
            TransportRefusal::NotDiscovered => "transporter.refused.not_discovered",
            TransportRefusal::NoCivilians => "transporter.refused.no_civilians",
            TransportRefusal::OutOfRange => "transporter.refused.out_of_range",
            TransportRefusal::Unpowered => "transporter.refused.unpowered",
            TransportRefusal::Disabled => "transporter.refused.disabled",
        }
    }
}

/// The live inputs the transport verdict reads, gathered by the adapter off the
/// world.
///
/// A struct rather than a long argument list so the verdict reads as a table of
/// gates, and so a new gate is a field with a name rather than another
/// positional `bool`.
#[derive(Clone, Debug, PartialEq)]
pub struct TransportInputs<'a> {
    /// The uuid the operator has selected to rescue, or `None` when nothing is
    /// selected.
    pub selected: Option<&'a str>,
    /// Whether Sensors have completed the revealing scan of the selected contact
    /// — the discovery gate. `false` for an undiscovered or unselected contact.
    pub discovered: bool,
    /// How many unrecovered civilians the selected contact still carries.
    pub remaining: u32,
    /// Centre-to-centre distance from the operator to the selected contact, or
    /// `None` when nothing is selected or the contact cannot be found — either
    /// way there is nothing in range.
    pub separation: Option<f32>,
    /// The transporter's authored reach.
    pub range: f32,
    /// The allocation level the transporter's power group is holding.
    pub power_level: u8,
    /// The transporter's authored minimum power level.
    pub min_power_level: u8,
    /// Whether the transporter system is damaged out.
    pub disabled: bool,
}

/// **The transport verdict.** `Ok(())` when the transporter may recover a
/// civilian this tick, else the one refusal the console shows (issue #1348).
///
/// Pure: the adapter reads the live world into these scalars and applies the
/// answer. Used at start time (so "starting with no contact / undiscovered / out
/// of range / unpowered is refused") and re-run every tick a transport is live
/// (so each interruption ends it).
///
/// # Check order is the console's "most actionable first"
///
/// A knocked-out or unpowered transporter cannot recover whoever it is pointed
/// at, so those are reported before the acquisition checks; among the latter,
/// there is no range to a contact that was never selected, so `NoContact`
/// precedes `NotDiscovered` precedes `OutOfRange`. `NoCivilians` sits last
/// because it is not a fault to fix by flying or powering — it is the SUCCESS
/// condition wearing a refusal's clothes, and the adapter reads it as "this
/// rescue is complete" rather than as an error.
pub fn transport_status(inputs: &TransportInputs) -> Result<(), TransportRefusal> {
    if inputs.disabled {
        return Err(TransportRefusal::Disabled);
    }
    if inputs.power_level < inputs.min_power_level {
        return Err(TransportRefusal::Unpowered);
    }
    if inputs.selected.is_none() {
        return Err(TransportRefusal::NoContact);
    }
    if !inputs.discovered {
        return Err(TransportRefusal::NotDiscovered);
    }
    match inputs.separation {
        Some(sep) if sep <= inputs.range => {}
        _ => return Err(TransportRefusal::OutOfRange),
    }
    if inputs.remaining == 0 {
        return Err(TransportRefusal::NoCivilians);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> TransporterConfig {
        TransporterConfig {
            range: 600.0,
            seconds_per_civilian: 2.0,
            min_power_level: 2,
        }
    }

    fn inputs() -> TransportInputs<'static> {
        TransportInputs {
            selected: Some("derelict"),
            discovered: true,
            remaining: 3,
            separation: Some(300.0),
            range: 600.0,
            power_level: 3,
            min_power_level: 2,
            disabled: false,
        }
    }

    #[test]
    fn a_well_formed_config_validates() {
        assert!(cfg().validate().is_ok());
    }

    #[test]
    fn a_zero_range_or_duration_or_power_is_rejected() {
        let base = cfg();
        assert!(TransporterConfig {
            range: 0.0,
            ..base.clone()
        }
        .validate()
        .is_err());
        assert!(TransporterConfig {
            seconds_per_civilian: 0.0,
            ..base.clone()
        }
        .validate()
        .is_err());
        assert!(TransporterConfig {
            seconds_per_civilian: -1.0,
            ..base.clone()
        }
        .validate()
        .is_err());
        assert!(TransporterConfig {
            min_power_level: 0,
            ..base
        }
        .validate()
        .is_err());
    }

    #[test]
    fn a_discovered_powered_in_range_contact_with_civilians_transports() {
        assert_eq!(transport_status(&inputs()), Ok(()));
        // Exactly at the range boundary still runs.
        assert_eq!(
            transport_status(&TransportInputs {
                separation: Some(600.0),
                ..inputs()
            }),
            Ok(())
        );
    }

    #[test]
    fn nothing_selected_refuses_no_contact() {
        assert_eq!(
            transport_status(&TransportInputs {
                selected: None,
                separation: None,
                ..inputs()
            }),
            Err(TransportRefusal::NoContact)
        );
    }

    #[test]
    fn an_unscanned_contact_refuses_not_discovered() {
        // The whole "announce only after the scan" gate: a selected contact that
        // has not been revealed cannot be transported.
        assert_eq!(
            transport_status(&TransportInputs {
                discovered: false,
                ..inputs()
            }),
            Err(TransportRefusal::NotDiscovered)
        );
    }

    #[test]
    fn a_contact_past_the_authored_range_refuses_out_of_range() {
        assert_eq!(
            transport_status(&TransportInputs {
                separation: Some(600.1),
                ..inputs()
            }),
            Err(TransportRefusal::OutOfRange)
        );
        // A present selection whose entity cannot be found (no separation) is
        // also "nothing in range".
        assert_eq!(
            transport_status(&TransportInputs {
                separation: None,
                ..inputs()
            }),
            Err(TransportRefusal::OutOfRange)
        );
    }

    #[test]
    fn a_fully_recovered_contact_refuses_no_civilians() {
        assert_eq!(
            transport_status(&TransportInputs {
                remaining: 0,
                ..inputs()
            }),
            Err(TransportRefusal::NoCivilians)
        );
    }

    #[test]
    fn power_below_the_minimum_refuses_unpowered_before_acquisition() {
        // Even with no contact, the more-actionable power refusal wins.
        assert_eq!(
            transport_status(&TransportInputs {
                selected: None,
                separation: None,
                power_level: 1,
                ..inputs()
            }),
            Err(TransportRefusal::Unpowered)
        );
    }

    #[test]
    fn a_disabled_transporter_refuses_first_of_all() {
        assert_eq!(
            transport_status(&TransportInputs {
                disabled: true,
                power_level: 0,
                selected: None,
                separation: None,
                ..inputs()
            }),
            Err(TransportRefusal::Disabled)
        );
    }

    #[test]
    fn every_refusal_has_its_own_string_id() {
        let mut ids: Vec<&str> = TransportRefusal::ALL
            .iter()
            .map(|r| r.string_id())
            .collect();
        assert_eq!(ids.len(), 6);
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 6, "the six ids are distinct");
        for refusal in TransportRefusal::ALL {
            assert!(refusal.string_id().starts_with("transporter.refused."));
        }
    }
}
