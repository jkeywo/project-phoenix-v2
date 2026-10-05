use bevy::prelude::*;
/// Newtype wrapper so `FactionRegistry` can be inserted as a Bevy Resource.
#[derive(Resource)]
pub struct FactionRegistryResource(pub crate::ai::faction::FactionRegistry);

impl std::ops::Deref for FactionRegistryResource {
    type Target = crate::ai::faction::FactionRegistry;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
