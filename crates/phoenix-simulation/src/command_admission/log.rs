use crate::entities::spawner::EntityUuid;
pub use phoenix_sim_session::command_admission::log::*;

pub fn ship_key_from_uuid(uuid: Option<&EntityUuid>) -> ShipKey {
    ShipKey(uuid.map(|u| u.0.clone()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A ship with no `EntityUuid` — the bare-`App` fixture shape — still gets
    /// its command recorded, under a key that says it is unresolvable.
    #[test]
    fn a_ship_with_no_uuid_yields_an_unnamed_key() {
        assert!(!crate::command_admission::log::ship_key_from_uuid(None).is_named());
        let uuid = EntityUuid("abc".into());
        assert_eq!(
            crate::command_admission::log::ship_key_from_uuid(Some(&uuid)),
            ShipKey("abc".into())
        );
    }
}
