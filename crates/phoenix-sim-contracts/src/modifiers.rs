/// Which integer attribute a modifier affects. Server-internal only; not in
/// `messages.rs` because integer modifier values are never sent to clients.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntModifierSlot {
    /// Additional repair teams granted to a ship.
    RepairTeams,
}

impl IntModifierSlot {
    /// Total number of slots; must be updated when new variants are added.
    pub const COUNT: usize = 1;

    /// Maps each slot to a fixed array index.
    pub fn index(&self) -> usize {
        match self {
            IntModifierSlot::RepairTeams => 0,
        }
    }

    pub fn all() -> [IntModifierSlot; Self::COUNT] {
        [IntModifierSlot::RepairTeams]
    }
}
