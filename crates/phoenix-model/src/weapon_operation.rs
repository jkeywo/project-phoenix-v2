//! Supported operating modes and real finite-round feed bindings.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeamOperation {
    #[default]
    Fixed,
    Sustained,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponRoundFeed {
    pub store: String,
    pub amount: i64,
}
impl WeaponRoundFeed {
    pub fn validate(&self) -> Result<(), String> {
        if self.store.is_empty()
            || self.store.len() > 128
            || !self
                .store
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || self.amount <= 0
        {
            return Err("weapon rounds require a bounded managed store ID and positive whole amount per barrel".into());
        }
        Ok(())
    }
}

/// Authored table inheritance needs an explicit legacy-policy override. Runtime
/// state keeps the usual Option: false clears inherited tags; [] remains Hold.
pub fn deserialize_target_filter<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<crate::tags::EntityTag>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Authored {
        Tags(Vec<crate::tags::EntityTag>),
        Legacy(bool),
    }
    match Authored::deserialize(deserializer)? {
        Authored::Tags(tags) => Ok(Some(tags)),
        Authored::Legacy(false) => Ok(None),
        Authored::Legacy(true) => Err(serde::de::Error::custom(
            "weapon targets accept false or a closed EntityTag array, never true",
        )),
    }
}
