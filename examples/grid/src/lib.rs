//! A deliberately different game: move one square around an eight-cell grid.
//! This crate cannot name any Phoenix model, content, simulation or presentation type.
use phoenix_runtime::digest::{hash_bytes, DigestLedger};
use phoenix_runtime::transfer::{self, Accepted, SnapshotChunk, SnapshotReceiver};
use phoenix_runtime::{commands::CommandQueue, session::LockstepSession, CommandOrder, HostSlot};
use serde::{Deserialize, Serialize};

pub mod codec;
pub mod protocol;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridState {
    pub tick: u64,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    pub dx: i32,
    pub dy: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedMove {
    pub tick: u64,
    pub order: CommandOrder,
    pub movement: Move,
}

/// The application owns admission and movement; the runtime owns ordering and barriers.
pub struct Grid {
    pub state: GridState,
    pub session: LockstepSession,
    pub ledger: DigestLedger,
    pending: CommandQueue<OrderedMove>,
}
impl Grid {
    pub fn new(local: HostSlot, peers: impl IntoIterator<Item = HostSlot>) -> Self {
        let mut pending = CommandQueue::default();
        pending.set_origin(local);
        Self {
            state: GridState::default(),
            session: LockstepSession::new(local, peers, 1),
            ledger: DigestLedger::new(1),
            pending,
        }
    }
    pub fn submit(&mut self, movement: Move) -> Option<OrderedMove> {
        if !valid_move(&movement) {
            return None;
        }
        let command = OrderedMove {
            tick: self.state.tick.checked_add(self.session.delay() + 1)?,
            order: self.pending.next_order(),
            movement,
        };
        self.receive(command.clone());
        Some(command)
    }
    pub fn receive(&mut self, command: OrderedMove) {
        if command.tick >= self.state.tick
            && valid_move(&command.movement)
            && (command.order.origin == self.session.local()
                || self
                    .session
                    .peers()
                    .any(|peer| peer == command.order.origin))
        {
            self.pending.insert((command.tick, command.order), command);
        }
    }
    pub fn advance(&mut self) -> bool {
        if !self.session.may_simulate(self.state.tick) {
            return false;
        }
        for command in self.pending.drain_due(self.state.tick) {
            self.state.x = (self.state.x + command.movement.dx).clamp(0, 7);
            self.state.y = (self.state.y + command.movement.dy).clamp(0, 7);
        }
        self.state.tick += 1;
        let hash = self.state_hash();
        self.ledger.record(self.state.tick, hash);
        self.ledger.final_digest = hash;
        true
    }
    pub fn digest(&self) -> String {
        format!(
            "grid/1/{}/{}/{}/{:016x}",
            self.state.tick,
            self.state.x,
            self.state.y,
            self.state_hash()
        )
    }
    fn state_hash(&self) -> u64 {
        let mut bytes = Vec::from(b"grid/1".as_slice());
        bytes.extend(self.state.tick.to_le_bytes());
        bytes.extend(self.state.x.to_le_bytes());
        bytes.extend(self.state.y.to_le_bytes());
        hash_bytes(&bytes)
    }
    pub fn checkpoint(&self) -> Result<String, String> {
        codec::encode(&Checkpoint {
            version: 1,
            state: self.state.clone(),
            next_sequence: self.pending.next_sequence(),
            pending: self.pending.values().cloned().collect(),
        })
    }
    pub fn recovery_chunks(&self) -> Result<Vec<SnapshotChunk>, String> {
        Ok(transfer::chunk(
            &self.checkpoint()?,
            self.session.local(),
            self.state.tick,
            self.state.tick,
        ))
    }
    pub fn restore(&mut self, text: &str) -> Result<(), String> {
        let checkpoint: Checkpoint = codec::decode(text)?;
        if checkpoint.version != 1
            || !(0..8).contains(&checkpoint.state.x)
            || !(0..8).contains(&checkpoint.state.y)
        {
            return Err("unsupported or invalid grid checkpoint".into());
        }
        let local = self.session.local();
        let peers: Vec<_> = self.session.peers().collect();
        let session =
            LockstepSession::new_at(local, peers, self.session.delay(), checkpoint.state.tick)
                .ok_or("checkpoint tick exceeds the clock")?;
        let mut pending = CommandQueue::default();
        pending.set_origin(local);
        pending.restore_sequence(checkpoint.next_sequence);
        for command in checkpoint.pending {
            if command.tick < checkpoint.state.tick || !valid_move(&command.movement) {
                return Err("invalid pending grid move".into());
            }
            if command.order.origin == local && command.order.seq >= checkpoint.next_sequence {
                return Err("pending move exceeds issuer checkpoint".into());
            }
            pending.insert((command.tick, command.order), command);
        }
        self.state = checkpoint.state;
        self.session = session;
        self.pending = pending;
        self.ledger = DigestLedger::new(1);
        self.ledger.record(self.state.tick, self.state_hash());
        self.ledger.final_digest = self.state_hash();
        Ok(())
    }
    pub fn recover(&mut self, chunks: &[SnapshotChunk]) -> Result<(), String> {
        let mut receiver = SnapshotReceiver::new();
        let mut complete = None;
        for chunk in chunks {
            if let Accepted::Complete(text) = receiver.accept(chunk).map_err(|e| e.to_string())? {
                complete = Some(text);
            }
        }
        self.restore(&complete.ok_or("incomplete grid checkpoint")?)
    }
}
#[derive(Serialize, Deserialize)]
struct Checkpoint {
    version: u32,
    state: GridState,
    next_sequence: u64,
    pending: Vec<OrderedMove>,
}

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(test)]
#[path = "grid_tests.rs"]
mod tests;

fn valid_move(movement: &Move) -> bool {
    matches!(
        (movement.dx, movement.dy),
        (1, 0) | (-1, 0) | (0, 1) | (0, -1)
    )
}
