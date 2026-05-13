use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RegionEffectKind {
    DamageZone { dps: f32 },
    SlowZone { multiplier: f32 },
    BlocksImpulse,
    RadarDampening { multiplier: f32 },
    CommsJam,
    SensorBlind,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(effect: RegionEffectKind) {
        let json = serde_json::to_string(&effect).unwrap();
        let decoded: RegionEffectKind = serde_json::from_str(&json).unwrap();
        assert_eq!(effect, decoded);
    }

    #[test]
    fn serde_round_trip_damage_zone() {
        round_trip(RegionEffectKind::DamageZone { dps: 15.0 });
    }

    #[test]
    fn serde_round_trip_slow_zone() {
        round_trip(RegionEffectKind::SlowZone { multiplier: 0.5 });
    }

    #[test]
    fn serde_round_trip_blocks_impulse() {
        round_trip(RegionEffectKind::BlocksImpulse);
    }

    #[test]
    fn serde_round_trip_radar_dampening() {
        round_trip(RegionEffectKind::RadarDampening { multiplier: 0.3 });
    }

    #[test]
    fn serde_round_trip_comms_jam() {
        round_trip(RegionEffectKind::CommsJam);
    }

    #[test]
    fn serde_round_trip_sensor_blind() {
        round_trip(RegionEffectKind::SensorBlind);
    }

    #[test]
    fn serde_round_trip_negative_values() {
        round_trip(RegionEffectKind::DamageZone { dps: -5.0 });
        round_trip(RegionEffectKind::SlowZone { multiplier: -1.0 });
    }

    #[test]
    fn serde_round_trip_zero_values() {
        round_trip(RegionEffectKind::DamageZone { dps: 0.0 });
        round_trip(RegionEffectKind::RadarDampening { multiplier: 0.0 });
    }
}
