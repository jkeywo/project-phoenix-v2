//! Native landing role selection and its one-way world/join commit boundary.

use bevy::prelude::Resource;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeSessionRole {
    #[default]
    Undecided,
    ShipHost,
    StandaloneGameMaster,
    FleetGameMaster,
}

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeSessionRoleState {
    role: NativeSessionRole,
    committed: bool,
    pub pending_code: Option<String>,
    pub refusal: Option<String>,
    pub join_status: Option<String>,
}

impl NativeSessionRoleState {
    pub fn role(&self) -> NativeSessionRole {
        self.role
    }

    pub fn committed(&self) -> bool {
        self.committed
    }

    pub fn request(&mut self, role: NativeSessionRole) -> bool {
        if self.committed && self.role != role {
            self.refusal = Some("role-already-committed".into());
            return false;
        }
        self.role = role;
        self.refusal = None;
        true
    }

    pub fn back(&mut self) -> bool {
        if self.committed {
            self.refusal = Some("role-already-committed".into());
            return false;
        }
        self.role = NativeSessionRole::Undecided;
        self.pending_code = None;
        self.refusal = None;
        self.join_status = None;
        true
    }

    pub fn commit(&mut self) {
        self.committed = true;
    }
}

#[cfg(test)]
#[path = "session_role_tests.rs"]
mod tests;
