use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FlagKind {
    CommsJammed,
    SensorBlind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comms_jammed_variant_exists() {
        match FlagKind::CommsJammed {
            FlagKind::CommsJammed => {}
            _ => panic!("expected CommsJammed"),
        }
    }

    #[test]
    fn sensor_blind_variant_exists() {
        match FlagKind::SensorBlind {
            FlagKind::SensorBlind => {}
            _ => panic!("expected SensorBlind"),
        }
    }
}
