//! Closed equipment and public intercepted evidence. Private payloads are never
//! represented by the public view until the actual work owner completes decode.
use serde::{Deserialize, Serialize};

pub const MAX_DECODE_TICKS: u32 = 36_000;
pub const MAX_CAPTURE_ROWS: usize = 256;
pub const WORK_PER_TICK: u64 = 1_000;
/// Browser clients must transmit local opaque numeric handles without rounding.
pub const MAX_CAPTURE_SEQUENCE: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsCaptureConfig {
    /// Highest level supported by paid codebreaking, without a secret key.
    pub max_level: u8,
    pub ticks_per_level: u32,
    /// Optional installed route for a genuinely held matching protection key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_key_ticks: Option<u32>,
}

impl CommsCaptureConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.max_level > 3
            || !(1..=MAX_DECODE_TICKS).contains(&self.ticks_per_level)
            || self
                .held_key_ticks
                .is_some_and(|value| !(1..=MAX_DECODE_TICKS).contains(&value))
        {
            return Err(
                "capture requires closed protection levels and positive bounded decode ticks",
            );
        }
        Ok(())
    }

    /// Own equipment feasibility only. Callers must prove real held-key evidence.
    pub fn required_work(&self, level: u8, held_key: bool) -> Option<u64> {
        if self.validate().is_err() || !(1..=3).contains(&level) {
            return None;
        }
        let breaking =
            (level <= self.max_level).then_some(u64::from(self.ticks_per_level) * u64::from(level));
        let keyed = held_key
            .then_some(self.held_key_ticks)
            .flatten()
            .map(u64::from);
        breaking
            .into_iter()
            .chain(keyed)
            .min()
            .map(|ticks| ticks * WORK_PER_TICK)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturedContentKind {
    Text,
    Request,
    Outcome,
}

/// Literal attributed claim only: deliberately no actor UUID, recipient,
/// request reference, accepted action, key or private transmission identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodedCommsEvidence {
    pub sender_name: String,
    pub body: String,
    pub literal_body: bool,
    pub kind: CapturedContentKind,
    pub emitted_tick: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsCaptureView {
    /// Positive receiver-local sequence; cannot address any machinery/contact.
    pub capture: u64,
    pub channel: String,
    pub level: u8,
    pub captured_tick: u64,
    pub feasible: bool,
    pub selected: bool,
    pub progress_per_mille: u16,
    pub decoded_tick: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<DecodedCommsEvidence>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsCaptureControlsView {
    pub rows: Vec<CommsCaptureView>,
    pub controllable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_codebreaking_and_optional_real_key_routes_keep_a_hard_ceiling() {
        let config = CommsCaptureConfig {
            max_level: 1,
            ticks_per_level: 20,
            held_key_ticks: Some(2),
        };
        assert_eq!(config.required_work(1, false), Some(20_000));
        assert_eq!(config.required_work(2, false), None);
        assert_eq!(config.required_work(3, true), Some(2_000));
        assert_eq!(config.required_work(0, true), None);
        assert_eq!(config.required_work(4, true), None);
        let no_key_route = CommsCaptureConfig {
            held_key_ticks: None,
            ..config
        };
        assert_eq!(no_key_route.required_work(3, true), None);
        for config in [
            CommsCaptureConfig {
                max_level: 4,
                ..no_key_route.clone()
            },
            CommsCaptureConfig {
                ticks_per_level: 0,
                ..no_key_route.clone()
            },
            CommsCaptureConfig {
                held_key_ticks: Some(0),
                ..no_key_route.clone()
            },
            CommsCaptureConfig {
                ticks_per_level: MAX_DECODE_TICKS + 1,
                ..no_key_route
            },
        ] {
            assert!(config.validate().is_err());
        }
    }
    #[test]
    fn undecoded_wire_has_no_payload_and_refuses_injected_private_fields() {
        let view = CommsCaptureView {
            capture: 1,
            channel: "fleet".into(),
            level: 2,
            captured_tick: 10,
            feasible: false,
            selected: false,
            progress_per_mille: 0,
            decoded_tick: None,
            evidence: None,
        };
        let mut value = serde_json::to_value(&view).unwrap();
        for field in [
            "evidence",
            "sender_uuid",
            "origin_id",
            "audience",
            "key",
            "body",
        ] {
            assert!(value.get(field).is_none());
        }
        value["sender_uuid"] = "private".into();
        assert!(serde_json::from_value::<CommsCaptureView>(value).is_err());
    }
}
