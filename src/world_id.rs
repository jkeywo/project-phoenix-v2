//! Deterministic tick-scoped world-id minting (issue #907, PRD #849).
//!
//! # The hazard this closes
//!
//! Issue #894 mandates that the authoritative-state digest folds in stable
//! world-id order. Phoenix's world ids were not stable across instances. Two
//! generations of them were wrong in two different ways:
//!
//! * `Uuid::new_v4()` — OS entropy. Two instances spawning the same NPC gave it
//!   two different ids, so the digest's folded *contents* differed **and** its
//!   fold *order* differed, since the order is a sort over those ids.
//! * A draw from `SimStream::EntityUuid` (issue #897's fix) — deterministic
//!   within one seeded instance, but a function of RNG **draw order**, not of
//!   the tick. Adding one unrelated draw anywhere upstream reshuffles every
//!   subsequent entity id and every digest recorded with it, and two instances
//!   that interleave draws differently mint different ids for the same spawn.
//!
//! A `(namespace, tick, seq)` counter is a function of the logical tick and of
//! the spawn order *within* that tick — both of which #895 (the fixed tick) and
//! #896 (serial physics) already make deterministic — and it survives an RNG
//! draw being added elsewhere, because it never touches the RNG at all.
//!
//! # The scheme
//!
//! [`WorldIdMint`] is a resource holding the tick it is currently minting for
//! and one sequence counter per [`IdNamespace`]. [`sync_world_id_mint`] runs in
//! `FixedFirst` (registered by `sim_tick::register_sim_tick`, so the mint and
//! the tick it is scoped to are wired in one place) and, whenever the tick has
//! moved, adopts it and resets every sequence to zero. Every mint after that
//! point in the step gets `(namespace, that tick, next seq)`.
//!
//! Minting takes `&self`, not `&mut self`. The interior `Mutex` is the same
//! shape `SimRng` uses and for the same reason: a spawn site that had to hold a
//! `ResMut` would conflict with every other spawn site in the schedule, and the
//! resulting ambiguity in system ordering is precisely the non-determinism this
//! module exists to remove.
//!
//! # Namespace is part of the minted id
//!
//! Asteroids carry `AsteroidUuid`, not `EntityUuid`, deliberately. #894 folds
//! namespaces in a declared sequence rather than merging them, so namespace
//! membership has to come from the same value as the sort key — it is a field
//! of [`WorldId`] and the leading bits of the rendered string, not something a
//! consumer infers from which component the id was found on.
//!
//! # The rendering, and why it is uuid-shaped
//!
//! [`WorldId`]'s derived `Ord` is `(namespace, tick, seq)` compared as numbers.
//! That is the sort key, and it is what `headless::digest::FoldKey` uses. The
//! string is a *rendering* of it.
//!
//! The obvious rendering is something readable like `"ent-11-4"`, and it is
//! what this module was first written to produce. It does not survive contact
//! with the codebase, for a reason worth recording rather than rediscovering:
//! **a world id's uuid SHAPE is load-bearing in two places.**
//!
//! 1. `ai::AiWorldEntity::uuid` is a real `uuid::Uuid`, not a string
//!    (`src/ai/core.rs`), built by `Uuid::parse_str(&uuid.0).unwrap_or_default()`
//!    in `ai::server`. A non-uuid id does not fail loudly there — it parses to
//!    `Uuid::nil()`, so *every* entity in the AI's world view collapses onto one
//!    identity and target selection quietly stops working. `Uuid` is threaded
//!    through the whole pure AI core (`WorldView`, `resolve_objective_target`,
//!    the target-selection results, the sentinel `Uuid::nil()`s).
//! 2. Comms uses "does this parse as a uuid?" as its *entity-vs-synthetic-name*
//!    discriminator — `current_sender_in_range` (`src/comms/server.rs`) treats a
//!    non-uuid sender like `_self` or `"Starcorp Command"` as always in range,
//!    because it has no physical entity to range-check. A readable id would
//!    make every real sender look synthetic and defeat the range gate.
//!
//! So the rendering is a **canonical UUID string whose bits are the tuple**:
//! version 8 (RFC 9562's "custom", which is exactly this case), with the value
//! `namespace << 96 | tick << 32 | seq` laid out big-endian in fixed-width
//! lowercase hex across the 30 hex digits the version and variant markers leave
//! free. That satisfies the record's constraint literally — the rendering is
//! zero-padded fixed-width, and because hex digits `0`–`9` sort before `a`–`f`
//! in ASCII, a lexicographic comparison of two rendered ids agrees with the
//! numeric comparison of their tuples — while leaving every consumer, the wire
//! protocol, the pure-JS client and the save format untouched.
//!
//! What is *not* claimed: readability. `ent-000000000011-000000` would be nicer
//! in a log than `00000000-0000-8000-8000-000b00000000`, and that was the
//! trade. [`WorldId::parse`] is the way back — `FoldKey` uses it, and so should
//! anything else that wants to know which tick an entity was born on.
//!
//! Capacities, since a fixed-width layout has them: the tick is a full `u64`,
//! the sequence is 32 bits (four billion mints in one tick), and the namespace
//! is 8 bits. Overflowing the sequence is not silently possible — [`WorldIdMint`]
//! saturates rather than wrapping, and [`SEQ_LIMIT`] is where that is stated.
//!
//! # What is deliberately NOT minted here
//!
//! * **Asteroids.** `asteroids::lifecycle::deterministic_cell_uuid` derives an
//!   asteroid's id from its `(layer, cell, slot)` coordinates — a pure function
//!   of where the rock is, which is constraint 8's whole design. That is
//!   already cross-instance identical, and it has to stay coordinate-derived:
//!   a rock respawns fresh when the player leaves its cell and returns, and it
//!   must come back with the id it had, which a tick-scoped counter could not
//!   give it. Those ids are v4-shaped, so they do not parse as mints (the
//!   version nibble differs) and key as `(0, 0)` plus the raw string in the
//!   fold — deterministic, just not numeric.
//! * **Session tokens and lobby identity.** `localStorage` session tokens are
//!   per-browser identity, not simulation state; they are not folded, not
//!   replayed and not sim-authoritative.

use bevy::prelude::*;
use std::sync::Mutex;

/// The declared namespace sequence. **Append only.**
///
/// The discriminants *are* the sequence `headless::digest` folds in, and they
/// are also the leading bits of every rendered id. Inserting a variant in the
/// middle reorders every id that sorts near it and invalidates every digest
/// ever recorded. Appending one is free: a namespace nothing folds simply never
/// appears in a fold. The discriminants are dense and are the `ALL` index —
/// `IdNamespace::ALL[i].code() as usize == i` for every `i` — so `ALL`'s
/// declared order and the wire-visible codes can never quietly drift apart.
///
/// [`Entity`](IdNamespace::Entity) and [`Asteroid`](IdNamespace::Asteroid) are
/// the two folded namespaces. [`Message`](IdNamespace::Message) and
/// [`Projectile`](IdNamespace::Projectile) are minted but not folded today:
/// message ids are command-addressing surface (a recorded
/// `RespondToMessage { message_id, .. }` has to resolve on the peer that
/// replays it), and projectile ids key the in-flight/hit bookkeeping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum IdNamespace {
    /// `EntityUuid` (`src/entities/spawner.rs`) — all spawner identity.
    Entity = 0,
    /// `AsteroidUuid` (`src/server_app.rs`) — streamed lifecycle identity,
    /// deliberately a different namespace from the above. Minted by
    /// `deterministic_cell_uuid` rather than here; see the module docs.
    Asteroid = 1,
    /// `CommsMessage::id` — addressed by `RespondToMessage`, so authoritative.
    Message = 2,
    /// Torpedo and blaster projectile ids.
    Projectile = 3,
}

impl IdNamespace {
    /// Every namespace, in declared order. Used to size the mint's counters and
    /// to decode a rendered id.
    pub const ALL: [IdNamespace; 4] = [
        IdNamespace::Entity,
        IdNamespace::Asteroid,
        IdNamespace::Message,
        IdNamespace::Projectile,
    ];

    /// The byte this namespace encodes as. Wire-visible: changing one re-labels
    /// every id a recorded run ever produced.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// The inverse of [`code`](IdNamespace::code).
    pub fn from_code(code: u8) -> Option<Self> {
        IdNamespace::ALL.into_iter().find(|ns| ns.code() == code)
    }
}

/// One past the largest sequence a single tick can mint.
///
/// The rendering gives the sequence 32 bits. [`WorldIdMint::mint`] saturates at
/// this value rather than wrapping, so the failure mode of an absurd tick is a
/// repeated id (loud: two entities with one identity) rather than a silently
/// recycled one (quiet: a later entity stealing an earlier one's history).
pub const SEQ_LIMIT: u64 = 1 << 32;

/// The version nibble every minted id carries: RFC 9562's version 8, "custom".
///
/// It is also what distinguishes a mint from every other uuid in the world — a
/// v4 uuid (`Uuid::new_v4`, `Builder::from_random_bytes`, and so every
/// `deterministic_cell_uuid` asteroid) carries `'4'` here and correctly fails
/// [`WorldId::parse`].
const VERSION_MARKER: char = '8';

/// The variant digit. `0x8` is `0b1000`: the two leading bits are RFC 9562's
/// variant, and the two trailing bits are spare and deliberately left zero so
/// the digit is a constant and cannot perturb the lexicographic order.
const VARIANT_MARKER: char = '8';

/// Index of the version nibble within the 32 hex digits of a uuid.
const VERSION_INDEX: usize = 12;

/// Index of the variant nibble within the 32 hex digits of a uuid.
const VARIANT_INDEX: usize = 16;

/// Hex digits left for the payload once the two markers are removed.
const PAYLOAD_DIGITS: usize = 30;

/// A minted world id: the structured `(namespace, tick, seq)` tuple.
///
/// `Ord` is the derived field order, which *is* the fold policy #894 records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorldId {
    pub namespace: IdNamespace,
    pub tick: u64,
    pub seq: u64,
}

impl WorldId {
    pub fn new(namespace: IdNamespace, tick: u64, seq: u64) -> Self {
        Self {
            namespace,
            tick,
            seq,
        }
    }

    /// The 120-bit payload this id packs into a uuid, most significant field
    /// first so numeric order over the payload is tuple order.
    fn payload(&self) -> u128 {
        ((self.namespace.code() as u128) << 96)
            | ((self.tick as u128) << 32)
            | (self.seq.min(SEQ_LIMIT - 1) as u128)
    }

    /// Render as a canonical, lowercase, hyphenated UUID string.
    ///
    /// See the module docs for why the rendering is uuid-shaped at all.
    pub fn render(&self) -> String {
        let hex = format!("{:0width$x}", self.payload(), width = PAYLOAD_DIGITS);
        debug_assert_eq!(
            hex.len(),
            PAYLOAD_DIGITS,
            "payload must not exceed 120 bits"
        );

        let mut digits = String::with_capacity(32);
        digits.push_str(&hex[0..VERSION_INDEX]);
        digits.push(VERSION_MARKER);
        digits.push_str(&hex[VERSION_INDEX..VARIANT_INDEX - 1]);
        digits.push(VARIANT_MARKER);
        digits.push_str(&hex[VARIANT_INDEX - 1..]);

        format!(
            "{}-{}-{}-{}-{}",
            &digits[0..8],
            &digits[8..12],
            &digits[12..16],
            &digits[16..20],
            &digits[20..32],
        )
    }

    /// Parse a rendered id back into its structured form.
    ///
    /// `None` for anything that is not one of ours — a v4 uuid (every asteroid,
    /// every session token), an authored name, a test literal. Callers key
    /// those as `(0, 0)` and fall back to comparing the raw string, which keeps
    /// a mixed world's sort total.
    ///
    /// Deliberately strict about the version and variant markers rather than
    /// just reading bits out of any uuid: the markers are the whole of what
    /// says "this id came from the mint", and a lenient parse would invent a
    /// tick and a sequence for an asteroid.
    pub fn parse(id: &str) -> Option<Self> {
        let bytes = id.as_bytes();
        if bytes.len() != 36 {
            return None;
        }
        for pos in [8, 13, 18, 23] {
            if bytes[pos] != b'-' {
                return None;
            }
        }
        let digits: String = id.chars().filter(|c| *c != '-').collect();
        if digits.len() != 32 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let digits: Vec<char> = digits.chars().collect();
        if digits[VERSION_INDEX] != VERSION_MARKER || digits[VARIANT_INDEX] != VARIANT_MARKER {
            return None;
        }

        let mut hex = String::with_capacity(PAYLOAD_DIGITS);
        for (i, c) in digits.iter().enumerate() {
            if i != VERSION_INDEX && i != VARIANT_INDEX {
                hex.push(*c);
            }
        }
        let payload = u128::from_str_radix(&hex, 16).ok()?;
        // Anything in the top 24 bits is not a namespace this build knows how
        // to place, and guessing would put an id in the wrong fold group.
        let namespace = IdNamespace::from_code(u8::try_from(payload >> 96).ok()?)?;
        Some(Self {
            namespace,
            tick: ((payload >> 32) & u64::MAX as u128) as u64,
            seq: (payload & (SEQ_LIMIT - 1) as u128) as u64,
        })
    }
}

impl std::fmt::Display for WorldId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render())
    }
}

/// The single chokepoint every simulation id is minted at.
///
/// Holds the tick it is minting for and one sequence per namespace. See the
/// module docs for why minting takes `&self`.
#[derive(Resource, Debug)]
pub struct WorldIdMint {
    inner: Mutex<MintState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MintState {
    tick: u64,
    next_seq: [u64; IdNamespace::ALL.len()],
}

impl Default for WorldIdMint {
    fn default() -> Self {
        Self {
            inner: Mutex::new(MintState {
                tick: 0,
                next_seq: [0; IdNamespace::ALL.len()],
            }),
        }
    }
}

impl WorldIdMint {
    /// Adopt `tick` and reset every sequence, if the tick has moved.
    ///
    /// Idempotent within a tick: calling it twice in the same step does not
    /// re-issue an already-minted sequence number. That matters because ids
    /// minted outside the fixed schedules (a `Startup` spawn, a frame-driven
    /// system) carry the last-synced tick and continue its sequence rather than
    /// restarting it — which is what keeps them unique without needing a
    /// separate "off-tick" namespace.
    pub fn begin_tick(&self, tick: u64) {
        let mut state = self.lock();
        if state.tick != tick {
            state.tick = tick;
            state.next_seq = [0; IdNamespace::ALL.len()];
        }
    }

    /// Mint the next id in `namespace` for the tick currently being minted.
    pub fn mint(&self, namespace: IdNamespace) -> WorldId {
        let mut state = self.lock();
        let slot = &mut state.next_seq[namespace.code() as usize];
        let seq = *slot;
        // Saturate rather than wrap — see `SEQ_LIMIT`.
        *slot = (*slot + 1).min(SEQ_LIMIT - 1);
        WorldId {
            namespace,
            tick: state.tick,
            seq,
        }
    }

    /// The tick this mint is currently issuing ids for.
    pub fn tick(&self) -> u64 {
        self.lock().tick
    }

    /// How many ids `namespace` has minted for the current tick.
    ///
    /// Read by the digest (`headless::digest::fold_run_scope`), which folds
    /// these counters for the same reason it folds `SimRng`'s stream positions:
    /// a divergent spawn count is then caught on the tick it happens rather
    /// than on the tick the next id is minted.
    pub fn minted_so_far(&self, namespace: IdNamespace) -> u64 {
        self.lock().next_seq[namespace.code() as usize]
    }

    /// Poisoning is recovered from rather than propagated, for the same reason
    /// `SimRng::stream` does it: a panic elsewhere has already failed the run,
    /// and turning it into a second panic inside a spawn path buries the first.
    fn lock(&self) -> std::sync::MutexGuard<'_, MintState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Adopt the current [`SimTick`](crate::sim_tick::SimTick) at the top of every
/// fixed step.
///
/// `FixedFirst`, so every sim system in the step below it mints against the
/// index of the step it is actually running in. `Res`, not `ResMut`, because
/// the mint's state lives behind its own lock — this system therefore conflicts
/// with nothing and never constrains the schedule.
pub fn sync_world_id_mint(tick: Res<crate::sim_tick::SimTick>, mint: Res<WorldIdMint>) {
    mint.begin_tick(tick.0);
}

/// The fallback mint for apps that never inserted the resource.
///
/// Every bare-`App` unit test in this crate is such an app — the same reason
/// every determinism system takes `Option<Res<_>>` rather than a bare `Res`.
/// Deliberately a real mint at tick 0 rather than OS entropy: a fixture has no
/// run to reproduce, but it does need ids that are unique within the process
/// and shaped like the real thing, and reaching for `Uuid::new_v4()` here would
/// reopen exactly the hole issue #907's clippy ban closes. This mint is
/// process-global and shared across every fixture that hits the fallback path,
/// so uniqueness holds across those fixtures too — but nothing about it is
/// reproducible: it carries no seed, and its ids are a function of how many
/// fixtures minted before it in this process, not of any recorded run.
fn fallback_mint() -> &'static WorldIdMint {
    static FALLBACK: std::sync::OnceLock<WorldIdMint> = std::sync::OnceLock::new();
    FALLBACK.get_or_init(WorldIdMint::default)
}

/// Mint one id, in string form, from an optional mint resource.
///
/// This is what call sites use. It is the twin of `sim_rng::with_stream`: take
/// `Option<Res<WorldIdMint>>`, pass `.as_deref()`, get a deterministic id.
pub fn mint_id_with(mint: Option<&WorldIdMint>, namespace: IdNamespace) -> String {
    match mint {
        Some(m) => m.mint(namespace).render(),
        None => fallback_mint().mint(namespace).render(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_codes_round_trip_and_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (i, ns) in IdNamespace::ALL.into_iter().enumerate() {
            assert!(seen.insert(ns.code()), "duplicate code {}", ns.code());
            assert_eq!(IdNamespace::from_code(ns.code()), Some(ns));
            // Dense-discriminant invariant: the codes are the `ALL` index, not
            // merely unique. `headless::digest` and the rendering both rely on
            // this staying true.
            assert_eq!(
                ns.code() as usize,
                i,
                "discriminants must be dense and match the ALL index"
            );
        }
        assert_eq!(IdNamespace::from_code(200), None);
    }

    /// The declared sequence is the fold order. Pinned by value, not by
    /// position, so appending a variant is free and inserting one fails here.
    #[test]
    fn namespace_discriminants_are_pinned() {
        assert_eq!(IdNamespace::Entity.code(), 0);
        assert_eq!(IdNamespace::Asteroid.code(), 1);
        assert_eq!(IdNamespace::Message.code(), 2);
        assert_eq!(IdNamespace::Projectile.code(), 3);
    }

    /// The rendering is a real, parseable uuid — which is the entire reason it
    /// is shaped this way. See the module docs for the two consumers that
    /// depend on it.
    #[test]
    fn a_minted_id_is_a_valid_uuid() {
        let rendered = WorldId::new(IdNamespace::Entity, 12_345, 7).render();
        let parsed = uuid::Uuid::parse_str(&rendered).expect("a mint must be a valid uuid");
        assert_eq!(parsed.get_version_num(), 8, "RFC 9562 version 8, 'custom'");
        assert_eq!(parsed.to_string(), rendered, "canonical, lowercase form");
    }

    #[test]
    fn render_round_trips_through_parse() {
        for id in [
            WorldId::new(IdNamespace::Entity, 0, 0),
            WorldId::new(IdNamespace::Asteroid, 1, 1),
            WorldId::new(IdNamespace::Message, 12_345, 7),
            WorldId::new(IdNamespace::Projectile, u64::MAX, SEQ_LIMIT - 1),
        ] {
            assert_eq!(WorldId::parse(&id.render()), Some(id), "{id:?}");
        }
    }

    /// The property the fixed-width hex layout exists for: lexicographic order
    /// over the rendered strings agrees with numeric order over the tuples —
    /// across namespaces as well as within one, because the namespace occupies
    /// the leading bits and hex `0`–`9` sort before `a`–`f`.
    #[test]
    fn padded_render_sorts_like_the_structured_tuple() {
        let ids = [
            WorldId::new(IdNamespace::Entity, 2, 1),
            WorldId::new(IdNamespace::Entity, 10, 1),
            WorldId::new(IdNamespace::Entity, 10, 2),
            WorldId::new(IdNamespace::Entity, 1, SEQ_LIMIT - 1),
            WorldId::new(IdNamespace::Asteroid, 1, 0),
            WorldId::new(IdNamespace::Projectile, 0, 0),
        ];
        let mut by_tuple = ids.to_vec();
        by_tuple.sort();
        let mut by_string: Vec<WorldId> = ids.to_vec();
        by_string.sort_by_key(|id| id.render());
        assert_eq!(by_tuple, by_string);
        // And the naive readable render is what that protects against.
        assert!("10-1" < "2-1");
    }

    /// Everything that is NOT a mint must fail to parse, especially the two
    /// populations that share the world with them.
    #[test]
    fn parse_rejects_uuids_that_are_not_mints() {
        // A v4 uuid — every asteroid, every session token.
        assert_eq!(
            WorldId::parse("a1b2c3d4-0000-4000-8000-000000000001"),
            None,
            "a v4 uuid must not be read as a mint"
        );
        // Right shape, unknown namespace: guessing would fold it in the wrong
        // group, so it is refused rather than defaulted.
        // (the namespace byte is the top of the payload, i.e. hex digits 4-5)
        assert_eq!(WorldId::parse("0000ff00-0000-8000-8000-000000000000"), None);
        // Not uuids at all.
        assert_eq!(WorldId::parse("ent-000000000001-000001"), None);
        assert_eq!(WorldId::parse("2-1"), None);
        assert_eq!(WorldId::parse("_self"), None);
        assert_eq!(WorldId::parse(""), None);
    }

    /// The asteroid population, asserted against the real function rather than
    /// a hand-written literal.
    #[test]
    fn a_cell_derived_asteroid_id_is_not_a_mint() {
        // Same construction `deterministic_cell_uuid` uses: v4 from bytes.
        let rock = uuid::Builder::from_random_bytes([7u8; 16])
            .into_uuid()
            .to_string();
        assert_eq!(WorldId::parse(&rock), None);
    }

    #[test]
    fn sequence_is_per_namespace_and_resets_on_a_new_tick() {
        let mint = WorldIdMint::default();
        mint.begin_tick(7);
        assert_eq!(
            mint.mint(IdNamespace::Entity),
            WorldId::new(IdNamespace::Entity, 7, 0)
        );
        assert_eq!(
            mint.mint(IdNamespace::Entity),
            WorldId::new(IdNamespace::Entity, 7, 1)
        );
        // A different namespace counts separately.
        assert_eq!(
            mint.mint(IdNamespace::Message),
            WorldId::new(IdNamespace::Message, 7, 0)
        );
        // Re-syncing the same tick must not rewind.
        mint.begin_tick(7);
        assert_eq!(
            mint.mint(IdNamespace::Entity),
            WorldId::new(IdNamespace::Entity, 7, 2)
        );
        mint.begin_tick(8);
        assert_eq!(
            mint.mint(IdNamespace::Entity),
            WorldId::new(IdNamespace::Entity, 8, 0)
        );
    }

    /// AC5 in miniature: two mints driven through the same tick/spawn sequence
    /// produce identical ids, with no shared state between them.
    #[test]
    fn two_independent_mints_agree_on_the_same_schedule() {
        let script = [
            (0u64, IdNamespace::Entity),
            (0, IdNamespace::Entity),
            (0, IdNamespace::Message),
            (1, IdNamespace::Entity),
            (1, IdNamespace::Projectile),
            (4, IdNamespace::Entity),
        ];
        let run = || {
            let mint = WorldIdMint::default();
            script
                .iter()
                .map(|(tick, ns)| {
                    mint.begin_tick(*tick);
                    mint.mint(*ns).render()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
        // Pinned by value: these strings are a recorded run's identities, and
        // changing the layout silently re-labels every one of them.
        assert_eq!(
            run(),
            vec![
                "00000000-0000-8000-8000-000000000000",
                "00000000-0000-8000-8000-000000000001",
                "00000200-0000-8000-8000-000000000000",
                "00000000-0000-8000-8000-000100000000",
                "00000300-0000-8000-8000-000100000000",
                "00000000-0000-8000-8000-000400000000",
            ]
        );
    }

    /// Ids minted off the fixed schedule (a `Startup` spawn, a frame-driven
    /// system) keep the last-synced tick and continue its sequence, so they
    /// cannot collide with the ids that tick already issued.
    #[test]
    fn off_tick_mints_do_not_collide_with_the_tick_they_ride_on() {
        let mint = WorldIdMint::default();
        mint.begin_tick(3);
        let during = mint.mint(IdNamespace::Entity);
        let after = mint.mint(IdNamespace::Entity); // no begin_tick: a frame system
        assert_ne!(during, after);
        assert_eq!(after, WorldId::new(IdNamespace::Entity, 3, 1));
    }

    #[test]
    fn absent_resource_still_mints_unique_valid_ids() {
        let a = mint_id_with(None, IdNamespace::Entity);
        let b = mint_id_with(None, IdNamespace::Entity);
        assert_ne!(a, b);
        assert!(WorldId::parse(&a).is_some());
        assert!(uuid::Uuid::parse_str(&b).is_ok());
    }

    /// The Bevy wiring: `FixedFirst` sync means a system in the step sees the
    /// index of the step it is running in.
    #[test]
    fn sync_system_adopts_the_current_tick() {
        let mut app = App::new();
        app.init_resource::<crate::sim_tick::SimTick>()
            .init_resource::<WorldIdMint>()
            .add_systems(Update, sync_world_id_mint);
        app.update();
        assert_eq!(app.world().resource::<WorldIdMint>().tick(), 0);

        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 42;
        app.update();
        assert_eq!(app.world().resource::<WorldIdMint>().tick(), 42);
        assert_eq!(
            app.world()
                .resource::<WorldIdMint>()
                .mint(IdNamespace::Entity),
            WorldId::new(IdNamespace::Entity, 42, 0)
        );
    }
}
