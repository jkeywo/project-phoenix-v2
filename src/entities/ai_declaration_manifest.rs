//! Which AI-capable fine systems a hull actually DECLARES — and which ones a
//! synthesiser silently invents for it (issue #885a).
//!
//! # The gap this makes visible
//!
//! PRD #774 US7 asks that "every AI-capable fine system declares a policy or
//! explicit idle state, so that automation cannot silently be omitted". Today
//! the opposite ships: nineteen synthesisers (fourteen `default_*_ai_config()`
//! plus five `default_*_target_selector_config()`) fill any missing declaration
//! at spawn, and the missing-declaration case is never validated at all — every
//! `validate_fine_system_ai_*` call in [`EntityConfig::from_toml`] sits inside
//! an `if let Some(..)` with no `else`. There is no configuration state that
//! distinguishes "the author forgot this system" from "the author wants the
//! baseline", because the synthesiser fires identically in both cases.
//!
//! This module does not close the gap — the content migration (#885b) does,
//! stage by stage. What it does is make the gap **countable**, and it is the
//! ledger that records each stage:
//!
//! | change | mark | how |
//! |---|---|---|
//! | #885a, the count | 206 | — |
//! | #892 retired two raider hulls | 174 | **deletion** |
//! | #885b stage 5b authored all 50 selector blocks | 124 | **authoring** |
//! | #885b stage 5c authored all 124 policy blocks | **0** | **authoring** |
//!
//! Those two burn-downs are not the same thing and the module says so wherever
//! the number appears. Deletion drops the mark by taking undeclared hulls out
//! of the fleet: nothing was declared, and US7 is no closer to satisfied.
//! Authoring drops it by writing the declaration the synthesiser was standing
//! in for, which is the only kind of progress the PRD is asking for. A reader
//! who sees only "the ratchet fell" cannot tell them apart, so:
//!
//! 1. [`manifest`] enumerates, per entity, every AI-capable fine-system SLOT and
//!    whether the hull declared it. A slot is per-(hull, system) — per weapon
//!    where the system is per-weapon — so the output is the worklist #885b
//!    needs rather than a total.
//! 2. That worklist was tracked as a committed table through the #885b
//!    burn-down; it reached zero once stage 5c authored the last policy block,
//!    and the tracking scaffolding was retired at that point in favour of (3)
//!    below, which enforces the same "nothing undeclared" invariant at load —
//!    on every caller, not just the shipped fleet a worklist test could walk.
//! 3. [`AiDeclarationMode::Strict`] turns a missing declaration into a load
//!    error, and since #885b stage 5d it is the DEFAULT: every caller of
//!    [`EntityConfig::from_toml`] gets it. Stage 5d also deleted the nineteen
//!    synthesisers outright, so an undeclared AI-capable fine system now has no
//!    Rust-side stand-in at all — it would simply never act. Rejecting it at
//!    load is what stops that being a silent outcome.
//!
//! # Why the slot table is not simply hand-maintained
//!
//! Same reason as [`crate::entities::ai_flag_hosts`], whose approach this
//! follows: a hardcoded "these systems, on those hulls" list rots the moment
//! anything moves, and it rots SILENTLY — which is the failure mode being
//! closed. So:
//!
//! * [`FineSystemKey`] is an ENUM and [`slots_of_kind`] matches on it
//!   exhaustively, so a twentieth fine system cannot be added without the
//!   compiler demanding its gating.
//! * Every [`FineSystemKind`] records `spawn_sites`: the functions that attach
//!   its runtime component.
//!   `tests::every_kind_is_attached_at_every_one_of_its_spawn_sites` RE-DERIVES
//!   that by reading the crate's own source, so a declaration wired up on the
//!   NPC path and forgotten on the player one fails here — the omission that
//!   shipped in #785, #786 and #882.
//! * `tests::no_synthesiser_is_defined_or_called_anywhere` scans
//!   `src/entities/config.rs` and every spawn site for `default_*_ai_config` /
//!   `default_*_target_selector_config`, and requires ZERO. Stage 5d deleted
//!   them; this is the ratchet that stops one coming back.
//! * `tests::the_manifest_matches_the_real_spawner` spawns every shipped hull
//!   through the real `spawn_entity` and checks the manifest's slot set against
//!   the components actually attached. The gating in [`slots_of_kind`] mirrors
//!   the spawner by hand; this is what stops the mirror drifting.
//!
//! # The four selectors with no idle lever
//!
//! [`IdleLever`] records, per system, HOW "deliberately does nothing" can be
//! said. Policies say it in-band (`idle = true` inside the authored block —
//! `default_boost_ai_config` is exactly that shape). Tactical says it out of
//! band, with `[weapons_console] selector_idle`. The Sensors, Navigation,
//! Repair and Comms-hail selectors **cannot say it at all**: there is no idle
//! field on `FineSystemAiSelectorToml` and no sibling of `selector_idle` for
//! them.
//!
//! So for those four, US7's "or explicit idle" half is not expressible in
//! today's schema, and this module does not pretend otherwise — the manifest
//! carries [`IdleLever::Absent`] on them, [`strict_error`] quotes that in the
//! message rather than demanding something unwritable, and the demand it does
//! make is the satisfiable one: author the selector block.
//!
//! #885b answered that by authoring rather than by widening the schema: the
//! project owner ruled that US7's operative reading is "nothing is silently
//! synthesised", so `selector_idle` would not have satisfied it even where the
//! schema can express it — the selector is still built and attached from a Rust
//! default. Stage 5b therefore authored all fifty blocks, and no shipped hull
//! now depends on an idle field that does not exist. The gap the
//! [`IdleLever::Absent`] wording covers is still real for anything NEW that
//! wants to opt out, which is why the wording stays and
//! `tests::strict_mode_asks_for_the_block_where_no_idle_field_exists` still
//! exercises it.

use crate::entities::ai_flag_hosts::{self, AiHost, EvalSite};
use crate::entities::config::EntityConfig;

/// How a fine system can declare "I deliberately take no AI action".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleLever {
    /// The policy schema's own `idle = true`, INSIDE the authored block. Saying
    /// it still means authoring the block, so it is not a shortcut past the
    /// declaration — it is what the declaration can say.
    InBandPolicy,
    /// A dedicated field OUTSIDE the block, which declares intent without
    /// authoring anything. The quoted text is the authored key.
    Field(&'static str),
    /// Neither exists. US7's "or explicit idle" half cannot be satisfied for
    /// this system without a schema change.
    Absent,
}

/// The AI-capable fine-system kinds, as a closed enum.
///
/// Exhaustively matched in [`slots_of_kind`] and in the manifest's spawner
/// cross-check, so a new kind cannot be added without both being updated.
///
/// Two groups:
///
/// * The **twenty declaration-tracked kinds** (Captain … CommsSelector). Each
///   has a matching [`FineSystemKind`] in [`FINE_SYSTEM_KINDS`], an owning
///   [`ai_flag_hosts::AiHost`], and a per-hull authored block whose presence the
///   manifest counts. The twentieth is [`FineSystemKey::WeaponsDoctrine`], added
///   by issue #956.
/// * The **operate kinds** (Tractor, Umbilical, Dock, ExternalRepair), added by
///   issue #1162. These are AI-capable — a backfilled crew works them — but
///   their behaviour is NOT authored per hull: it is driven by a per-verb
///   operate directive (`Tow`/`Stabilise`/`Escort`/`Transfer`/`FieldRepair`)
///   and the shared thresholds in `fleet_baseline.toml`, so there is no
///   `[X.ai]` block to omit and nothing for the declaration manifest to
///   enforce. They are therefore deliberately NOT in [`FINE_SYSTEM_KINDS`] and
///   [`slots_of_kind`] returns no slot for them — "a hull that authors no policy
///   for a new system simply does not operate it" (the issue's own AC) is the
///   correct behaviour, not a missing declaration. They still live in the closed
///   enum so the two exhaustive match sites name every AI-capable system in one
///   place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FineSystemKey {
    Captain,
    CommsResponse,
    Engines,
    Steering,
    Lateral,
    Vertical,
    Impulse,
    Boost,
    PhaserBank,
    BlasterBank,
    TorpedoTube,
    WeaponsDoctrine,
    TorpedoMagazine,
    ShieldsFocus,
    Power,
    SensorsSelector,
    TacticalSelector,
    NavigationSelector,
    RepairSelector,
    CommsSelector,
    // ── Operate kinds (issue #1162): directive-driven, no per-hull declaration ──
    Tractor,
    Umbilical,
    Dock,
    ExternalRepair,
}

impl FineSystemKey {
    /// The stable manifest key. Appears in [`EXPECTED_UNDECLARED`], so renaming
    /// one is a visible diff over the whole worklist rather than a quiet
    /// reshuffle.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Captain => "captain",
            Self::CommsResponse => "comms_response",
            Self::Engines => "engines",
            Self::Steering => "steering",
            Self::Lateral => "lateral",
            Self::Vertical => "vertical",
            Self::Impulse => "impulse",
            Self::Boost => "boost",
            Self::PhaserBank => "phaser_bank",
            Self::BlasterBank => "blaster_bank",
            Self::TorpedoTube => "torpedo_tube",
            Self::WeaponsDoctrine => "weapons_doctrine",
            Self::TorpedoMagazine => "torpedo_magazine",
            Self::ShieldsFocus => "shields_focus",
            Self::Power => "power",
            Self::SensorsSelector => "sensors_selector",
            Self::TacticalSelector => "tactical_selector",
            Self::NavigationSelector => "navigation_selector",
            Self::RepairSelector => "repair_selector",
            Self::CommsSelector => "comms_selector",
            // Operate kinds (issue #1162). Named for readability of any future
            // diagnostic; they never reach the declaration worklist because
            // `slots_of_kind` yields no slot for them.
            Self::Tractor => "tractor",
            Self::Umbilical => "umbilical",
            Self::Dock => "dock",
            Self::ExternalRepair => "external_repair",
        }
    }
}

/// One AI-capable fine-system kind: which host owns it, which synthesiser fills
/// it when unauthored, how it could declare idle, and where the synthesis
/// happens.
#[derive(Clone, Copy, Debug)]
pub struct FineSystemKind {
    pub key: FineSystemKey,
    /// The host whose runtime evaluation this declaration feeds. Reused from
    /// [`crate::entities::ai_flag_hosts`] rather than restated, so the authored
    /// block name and the system's human name have one home.
    pub host: &'static AiHost,
    /// The runtime component this declaration decodes into, by type name.
    ///
    /// Every function listed in `spawn_sites` must mention it — re-derived from
    /// the crate's own source by
    /// `tests::every_kind_is_attached_at_every_one_of_its_spawn_sites`. That is
    /// the check that catches a declaration attached on the NPC path and not the
    /// player one, which is how #785, #786 and #882 each shipped broken.
    pub component: &'static str,
    pub idle_lever: IdleLever,
    /// Every function that attaches this kind's `component`. More than one when
    /// the player ship's own attachment path repeats what `spawn_entity` already
    /// does (see the #885 comment on the double attachment), and ALL of them are
    /// checked — wiring a declaration up on one path and not the other is this
    /// area's most likely failure mode.
    pub spawn_sites: &'static [EvalSite],
}

const fn site(file: &'static str, func: &'static str) -> EvalSite {
    EvalSite { file, func }
}

/// The generic per-entity spawn path PLUS the player ship's own attachment
/// pass. The player ship never goes through `spawn_entity` at all, so anything
/// listed here that
/// `server_app` forgets is a declaration the player ship simply does not get.
/// The four per-weapon kinds joined this list in #885b stage 5d, which is when
/// their omission stopped being masked by a read-time synthesised fallback.
const SPAWNER_AND_PLAYER: &[EvalSite] = &[
    site("src/entities/spawner.rs", "spawn_entity"),
    site("src/server_app.rs", "spawn_game_start_entities"),
];
/// Both Comms declarations are resolved by one shared helper that both spawn
/// paths call, so the helper is the single site.
const COMMS_HELPER: &[EvalSite] = &[site(
    "src/console/comms/server.rs",
    "comms_console_ai_components",
)];

/// Roll call, ordered to mirror [`ai_flag_hosts::AI_HOSTS`]: fifteen policy
/// kinds, then five selector kinds.
pub const FINE_SYSTEM_KINDS: &[FineSystemKind] = &[
    FineSystemKind {
        key: FineSystemKey::Captain,
        host: &ai_flag_hosts::CAPTAIN_RED_ALERT,
        component: "CaptainAiPolicy",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::Engines,
        host: &ai_flag_hosts::HELM_ENGINES,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::Steering,
        host: &ai_flag_hosts::HELM_STEERING,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::Lateral,
        host: &ai_flag_hosts::HELM_LATERAL,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::Vertical,
        host: &ai_flag_hosts::HELM_VERTICAL,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::Impulse,
        host: &ai_flag_hosts::HELM_IMPULSE,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::Boost,
        host: &ai_flag_hosts::HELM_BOOST,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::PhaserBank,
        host: &ai_flag_hosts::PHASER_BANK,
        component: "PhaserBankAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::BlasterBank,
        host: &ai_flag_hosts::BLASTER_BANK,
        component: "BlasterBankAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::TorpedoTube,
        host: &ai_flag_hosts::TORPEDO_TUBE,
        component: "TorpedoTubeAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::WeaponsDoctrine,
        host: &ai_flag_hosts::WEAPONS_DOCTRINE,
        component: "WeaponsDoctrineAiPolicy",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::TorpedoMagazine,
        host: &ai_flag_hosts::TORPEDO_MAGAZINE,
        component: "TorpedoMagazineAiPolicy",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::ShieldsFocus,
        host: &ai_flag_hosts::SHIELDS_FOCUS,
        component: "ShieldsFocusAiPolicy",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::Power,
        host: &ai_flag_hosts::POWER_ALLOCATION,
        component: "PowerAiPolicy",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::CommsResponse,
        host: &ai_flag_hosts::COMMS_RESPONSE,
        component: "CommsResponseAiPolicy",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: COMMS_HELPER,
    },
    FineSystemKind {
        key: FineSystemKey::SensorsSelector,
        host: &ai_flag_hosts::SENSORS_SELECTOR,
        component: "SensorsTargetSelector",
        idle_lever: IdleLever::Absent,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::TacticalSelector,
        host: &ai_flag_hosts::TACTICAL_SELECTOR,
        component: "TacticalTargetSelector",
        idle_lever: IdleLever::Field("[weapons_console] selector_idle"),
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::NavigationSelector,
        host: &ai_flag_hosts::NAVIGATION_SELECTOR,
        component: "NavigationTargetSelector",
        idle_lever: IdleLever::Absent,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::RepairSelector,
        host: &ai_flag_hosts::REPAIR_SELECTOR,
        component: "RepairTargetSelector",
        idle_lever: IdleLever::Absent,
        spawn_sites: SPAWNER_AND_PLAYER,
    },
    FineSystemKind {
        key: FineSystemKey::CommsSelector,
        host: &ai_flag_hosts::COMMS_SELECTOR,
        component: "CommsTargetSelector",
        idle_lever: IdleLever::Absent,
        spawn_sites: COMMS_HELPER,
    },
];

/// What the hull said about one slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Declared {
    /// The hull authored the policy/selector block. Whether that block says
    /// "act" or `idle = true` is the author's business — either way the
    /// declaration exists.
    Block,
    /// The hull pulled the out-of-band idle lever instead of authoring a block.
    ///
    /// Note the asymmetry this hides and #885b must face: `selector_idle = true`
    /// declares intent but does NOT stop the synthesiser — the selector is still
    /// built and attached, the host just refuses to use it. No shipped hull is
    /// in this state today, which `tests::no_shipped_hull_declares_idle_out_of_band`
    /// pins.
    IdleLever,
    /// Nothing. A synthesiser invents the declaration at spawn.
    Nothing,
}

/// One AI-capable fine system on one entity.
#[derive(Clone, Debug)]
pub struct Slot {
    pub kind: &'static FineSystemKind,
    /// The weapon id for per-weapon systems; `None` for ship-level ones.
    pub instance: Option<String>,
    pub declared: Declared,
}

impl Slot {
    /// The manifest key: `"captain"`, or `"torpedo_tube[fore-1]"`.
    pub fn key(&self) -> String {
        match &self.instance {
            Some(id) => format!("{}[{id}]", self.kind.key.as_str()),
            None => self.kind.key.as_str().to_string(),
        }
    }
}

fn declared_from(authored: bool) -> Declared {
    if authored {
        Declared::Block
    } else {
        Declared::Nothing
    }
}

/// Every slot of one kind on one entity, with the gating that decides whether
/// the kind applies at all.
///
/// This mirrors the spawn path by hand — there is no way to read a `match`
/// expression out of `spawner.rs` and get its meaning — so
/// `tests::the_manifest_matches_the_real_spawner` runs the real spawner over
/// every shipped hull and compares.
///
/// Two gates matter and are easy to get backwards:
///
/// * **Ship-level systems gate on `[behaviour]` ALONE.** Not on the console
///   section they belong to. A hull that declares no `[sensors_console]`,
///   `[navigation_console]`, `[repair]` or `[comms_console]` still receives all
///   five selectors. Computing this from "which sections does the hull
///   declare?" would under-report the gap by four slots per bare hull.
/// * **Weapons systems sit OUTSIDE the `[behaviour]` gate.** The per-weapon
///   kinds — and the ship-level `weapons_doctrine` with them — gate on
///   `[weapons_console]` / `[torpedoes]` instead, so an entity with weapons and
///   no `[behaviour]` gets bank policies, tube policies and a doctrine, and
///   nothing else. `weapons_doctrine` is the one ship-level kind on this side of
///   the line: it is a weapons-console decision, and the host that resolves it
///   (`tick_weapons_arc_request`) iterates every `Ship` without asking whether
///   the hull carries a `[behaviour]`.
pub fn slots_of_kind(kind: &'static FineSystemKind, c: &EntityConfig) -> Vec<Slot> {
    let ship_level = |authored: bool| -> Vec<Slot> {
        if c.behaviour.is_none() {
            return Vec::new();
        }
        vec![Slot {
            kind,
            instance: None,
            declared: declared_from(authored),
        }]
    };
    let helm = |f: fn(&crate::entities::config::HelmConsoleConfig) -> bool| -> bool {
        c.helm_console.as_ref().is_some_and(f)
    };

    match kind.key {
        FineSystemKey::Captain => {
            ship_level(c.captain_console.as_ref().is_some_and(|x| x.ai.is_some()))
        }
        FineSystemKey::CommsResponse => {
            ship_level(c.comms_console.as_ref().is_some_and(|x| x.ai.is_some()))
        }
        FineSystemKey::Engines => ship_level(helm(|h| h.engines_ai.is_some())),
        FineSystemKey::Steering => ship_level(helm(|h| h.steering_ai.is_some())),
        FineSystemKey::Lateral => ship_level(helm(|h| h.lateral_ai.is_some())),
        FineSystemKey::Vertical => ship_level(helm(|h| h.vertical_ai.is_some())),
        FineSystemKey::Impulse => ship_level(helm(|h| h.impulse_ai.is_some())),
        FineSystemKey::Boost => ship_level(helm(|h| h.boost_ai.is_some())),
        FineSystemKey::ShieldsFocus => ship_level(
            c.shields_console
                .as_ref()
                .is_some_and(|x| x.ai_policy.is_some()),
        ),
        FineSystemKey::Power => ship_level(c.power.as_ref().is_some_and(|x| x.ai_policy.is_some())),
        // Ship-level in SHAPE (one slot, no instance id) but NOT gated like the
        // five selectors it sits beside: it gates on `[weapons_console]`, the
        // way the per-weapon kinds below do, and deliberately not on
        // `[behaviour]`.
        //
        // The reason is that `tick_weapons_arc_request` iterates `With<Ship>`
        // unconditionally and `spawner.rs` attaches the policy inside its own
        // `if let Some(wc) = config.weapons_console` arm — neither asks about
        // `[behaviour]`. A hull with a weapons console and no `[behaviour]`
        // already owes its bank and tube declarations for exactly that reason;
        // gating THIS kind on `[behaviour]` would have let the same hull ship
        // with no doctrine, no load error, and no arc-bearing request at all,
        // silently losing an advisory a HUMAN helmsman reads off channel 3.
        //
        // The `[behaviour]`-gated reading would also under-report against the
        // real spawn path, which `tests::the_manifest_matches_the_real_spawner`
        // exists to catch.
        FineSystemKey::WeaponsDoctrine => match c.weapons_console.as_ref() {
            Some(w) => vec![Slot {
                kind,
                instance: None,
                declared: declared_from(w.ai.is_some()),
            }],
            None => Vec::new(),
        },
        FineSystemKey::SensorsSelector => ship_level(
            c.sensors_console
                .as_ref()
                .is_some_and(|x| x.selector.is_some()),
        ),
        FineSystemKey::NavigationSelector => ship_level(
            c.navigation_console
                .as_ref()
                .is_some_and(|x| x.selector.is_some()),
        ),
        FineSystemKey::RepairSelector => {
            ship_level(c.repair.as_ref().is_some_and(|x| x.selector.is_some()))
        }
        FineSystemKey::CommsSelector => ship_level(
            c.comms_console
                .as_ref()
                .is_some_and(|x| x.selector.is_some()),
        ),
        // The one kind with an out-of-band idle lever, so the three states are
        // genuinely distinguishable here.
        FineSystemKey::TacticalSelector => {
            if c.behaviour.is_none() && !c.is_static_point_defence() {
                return Vec::new();
            }
            let wc = c.weapons_console.as_ref();
            let declared = if wc.is_some_and(|w| w.selector.is_some()) {
                Declared::Block
            } else if wc.is_some_and(|w| w.selector_idle) {
                Declared::IdleLever
            } else {
                Declared::Nothing
            };
            vec![Slot {
                kind,
                instance: None,
                declared,
            }]
        }
        FineSystemKey::PhaserBank => c
            .weapons_console
            .iter()
            .flat_map(|w| w.phaser_banks.iter())
            .map(|b| Slot {
                kind,
                instance: Some(b.id.clone()),
                declared: declared_from(b.ai.is_some()),
            })
            .collect(),
        // Blasters differ from phasers: the spawner only attaches the policy map
        // when the bank list is NON-EMPTY, so an empty list is zero slots rather
        // than an empty map.
        FineSystemKey::BlasterBank => match c.weapons_console.as_ref() {
            Some(w) if !w.blaster_banks.is_empty() => w
                .blaster_banks
                .iter()
                .map(|b| Slot {
                    kind,
                    instance: Some(b.id.clone()),
                    declared: declared_from(b.ai.is_some()),
                })
                .collect(),
            _ => Vec::new(),
        },
        FineSystemKey::TorpedoTube => c
            .torpedoes
            .iter()
            .flat_map(|t| t.tubes.iter())
            .map(|t| Slot {
                kind,
                instance: Some(t.id.clone()),
                declared: declared_from(t.ai.is_some()),
            })
            .collect(),
        FineSystemKey::TorpedoMagazine => match c.torpedoes.as_ref() {
            Some(t) => vec![Slot {
                kind,
                instance: None,
                declared: declared_from(t.ai.is_some()),
            }],
            None => Vec::new(),
        },
        // The operate kinds (issue #1162) carry NO per-hull AI declaration: a
        // backfilled crew works the tractor/umbilical/dock/external-repair from
        // a per-verb operate directive and the shared `fleet_baseline` thresholds,
        // not from an authored `[X.ai]` block. There is nothing to omit, so the
        // manifest tracks no slot for them — and a hull that carries the system
        // but sees no directive simply does not operate it, unchanged. (These
        // arms are never reached in practice: the operate kinds are deliberately
        // absent from `FINE_SYSTEM_KINDS`, which is all `manifest` iterates. They
        // exist only to keep this match exhaustive over the closed enum.)
        FineSystemKey::Tractor
        | FineSystemKey::Umbilical
        | FineSystemKey::Dock
        | FineSystemKey::ExternalRepair => Vec::new(),
    }
}

/// Every AI-capable fine-system slot on one entity, declared or not.
///
/// Empty for scenery: an entity with no `[behaviour]`, no `[weapons_console]`
/// and no `[torpedoes]` has no AI-capable fine system, and content validation
/// for missing intent must not start demanding declarations from it.
pub fn manifest(c: &EntityConfig) -> Vec<Slot> {
    FINE_SYSTEM_KINDS
        .iter()
        .flat_map(|kind| slots_of_kind(kind, c))
        .collect()
}

/// The slots on one entity that nobody declared — the #885b worklist, sorted by
/// manifest key.
pub fn undeclared_keys(c: &EntityConfig) -> Vec<String> {
    let mut keys: Vec<String> = manifest(c)
        .into_iter()
        .filter(|s| s.declared == Declared::Nothing)
        .map(|s| s.key())
        .collect();
    keys.sort();
    keys
}

/// One human-readable line per slot, for the load-time surface and for anyone
/// reading the worklist by eye. Developer-facing tooling: never player-visible,
/// so it carries no string id (AGENTS.md rule #11's display-text exception does
/// not apply).
pub fn manifest_lines(label: &str, c: &EntityConfig) -> Vec<String> {
    manifest(c)
        .into_iter()
        .map(|slot| {
            let state = match slot.declared {
                Declared::Block => format!("declared     {}", slot.kind.host.block),
                Declared::IdleLever => match slot.kind.idle_lever {
                    IdleLever::Field(f) => format!("idle         {f}"),
                    _ => "idle".to_string(),
                },
                Declared::Nothing => {
                    format!("UNDECLARED   author {}", slot.kind.host.block)
                }
            };
            format!("{label}  {:<28}  {state}", slot.key())
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Strict mode
// ─────────────────────────────────────────────────────────────────────────────

/// Whether a missing fine-system declaration is a load error.
///
/// **Default ON since #885b stage 5d.** [`EntityConfig::from_toml`] uses
/// [`Self::DEFAULT`], so every load path — shipped hulls, world entities,
/// scenarios, the editor's validator — rejects an AI-capable fine system that
/// declares neither a policy nor an explicit idle state. That is PRD #774 US7's
/// actual requirement: automation cannot silently be omitted.
///
/// [`Self::Lenient`] survives for the one thing that still needs it: a test
/// fixture that deliberately declares nothing, so that the strict path itself
/// can be exercised against it. Nothing in production passes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiDeclarationMode {
    /// Missing declarations are accepted and simply attach no policy. Kept for
    /// fixtures that need to build an undeclared entity in order to test the
    /// strict path.
    Lenient,
    /// A missing declaration on an AI-capable fine system fails the entity load.
    #[default]
    Strict,
}

impl AiDeclarationMode {
    /// The mode [`EntityConfig::from_toml`] runs in. **This is the switch.**
    pub const DEFAULT: Self = Self::Strict;
}

/// The strict-mode load error for an entity, or `None` when every AI-capable
/// fine system on it is declared.
///
/// The message names each undeclared slot, the block to author, and the runtime
/// component that will simply not be attached — so the error IS the worklist for
/// that hull. For the four selectors with no idle lever it says so, rather than
/// telling an author to write something the schema has no field for.
pub fn strict_error(c: &EntityConfig) -> Option<String> {
    let missing: Vec<Slot> = manifest(c)
        .into_iter()
        .filter(|s| s.declared == Declared::Nothing)
        .collect();
    if missing.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = missing
        .iter()
        .map(|slot| {
            let idle = match slot.kind.idle_lever {
                IdleLever::InBandPolicy => "or `idle = true` inside it".to_string(),
                IdleLever::Field(f) => format!("or set `{f}`"),
                IdleLever::Absent => {
                    "— this selector has NO idle field, so an explicit idle is not \
                     expressible in today's schema and the block is the only way to \
                     declare it"
                        .to_string()
                }
            };
            format!(
                "  {} — author {} {idle} (without it no {} is attached and the system \
                 never acts)",
                slot.key(),
                slot.kind.host.block,
                slot.kind.component
            )
        })
        .collect();
    lines.sort();
    Some(format!(
        "strict AI-declaration mode: {} AI-capable fine system(s) declare neither a \
         policy nor an explicit idle state (PRD #774 US7), so their automation would \
         be neither authored nor run:\n{}",
        missing.len(),
        lines.join("\n")
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// The committed worklist
// ─────────────────────────────────────────────────────────────────────────────
//
// #885b's worklist and burn-down ledger (206 → 174 → 124 → 0 undeclared
// AI-capable fine-system slots) tracked here as a committed table through
// every stage. It reached zero once stage 5c authored the last policy block:
// every slot on every shipped hull carries an authored block, transcribed
// verbatim from the synthesiser that used to invent it. The tracking table and
// its exact-equality test were retired at that point — stage 5d then deleted
// the synthesisers outright and made [`AiDeclarationMode::Strict`] the
// default, so a hull that ships an undeclared slot fails to load on every
// path, not just under a test that walks the shipped fleet. See
// `tests::strict_mode_is_on_by_default_and_every_shipped_hull_still_loads`.

#[cfg(test)]
pub(crate) mod source_scan {
    //! Reading the crate's own source, so the declared tables are re-derived
    //! rather than trusted. Same technique as
    //! `crate::entities::ai_flag_hosts::tests`.

    use std::path::PathBuf;

    pub fn crate_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// Drop everything from the file's `#[cfg(test)] mod ...` marker onwards, so
    /// fixtures in unit tests never masquerade as production call sites.
    pub fn strip_test_module(src: &str) -> String {
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim_start() != "#[cfg(test)]" {
                continue;
            }
            if lines
                .get(i + 1)
                .is_some_and(|next| next.trim_start().starts_with("mod "))
            {
                return lines[..i].join("\n");
            }
        }
        src.to_string()
    }

    pub fn read_non_test_source(rel: &str) -> String {
        let path = crate_root().join(rel);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("scanned file {rel} must be readable: {e}"));
        strip_test_module(&src)
    }

    /// The body of `fn <name>`, by brace counting from the signature's `{`.
    pub fn function_body<'a>(src: &'a str, func: &str) -> &'a str {
        let needle = format!("fn {func}");
        let start = src
            .find(&needle)
            .unwrap_or_else(|| panic!("no `{needle}` in the scanned source"));
        let open = start
            + src[start..]
                .find('{')
                .unwrap_or_else(|| panic!("`{needle}` has no body"));
        let mut depth = 0usize;
        for (offset, ch) in src[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &src[open..open + offset + 1];
                    }
                }
                _ => {}
            }
        }
        panic!("`{needle}` body is unbalanced");
    }

    /// Every `default_*_ai_config` / `default_*_target_selector_config` name
    /// mentioned in `src`, deduplicated.
    ///
    /// Deliberately name-shaped rather than call-shaped: it catches a definition
    /// as readily as a call, which is what
    /// `every_synthesiser_definition_belongs_to_a_kind` needs.
    pub fn synthesiser_names(src: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for suffix in ["_ai_config", "_target_selector_config"] {
            let mut from = 0usize;
            while let Some(hit) = src[from..].find(suffix) {
                let end = from + hit + suffix.len();
                // Walk back to the start of the identifier.
                let start = src[..from + hit]
                    .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let name = &src[start..end];
                if name.starts_with("default_") {
                    out.push(name.to_string());
                }
                from = end;
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::source_scan::*;
    use super::*;
    use crate::ai::policy::AiPolicy;
    use crate::ai::selector::TargetSelector;
    use bevy::prelude::*;
    use std::collections::BTreeSet;

    // ── The table is complete and internally consistent ──────────────────────

    #[test]
    fn every_ai_host_owns_exactly_one_fine_system_kind() {
        let hosted: Vec<&str> = FINE_SYSTEM_KINDS.iter().map(|k| k.host.block).collect();
        let unique: BTreeSet<&str> = hosted.iter().copied().collect();
        assert_eq!(
            hosted.len(),
            unique.len(),
            "two kinds claim the same authored block, so the worklist would name the \
             wrong system"
        );
        let all_hosts: BTreeSet<&str> = ai_flag_hosts::AI_HOSTS.iter().map(|h| h.block).collect();
        assert_eq!(
            unique, all_hosts,
            "the manifest's kinds and ai_flag_hosts::AI_HOSTS must be the same twenty \
             hosts. A host with no kind is a fine system whose missing declaration \
             nothing counts — which is exactly the invisibility #885a closes."
        );
    }

    #[test]
    fn every_kind_key_is_unique() {
        let keys: BTreeSet<&str> = FINE_SYSTEM_KINDS.iter().map(|k| k.key.as_str()).collect();
        assert_eq!(
            keys.len(),
            FINE_SYSTEM_KINDS.len(),
            "manifest keys are the committed worklist's identifiers; a duplicate would \
             merge two systems into one line"
        );
    }

    /// AC: the declared attachment site is not hand-maintained trivia — the
    /// spawn path is read and checked, on EVERY path the kind claims.
    ///
    /// This is the test that would have caught #785, #786 and #882: each of them
    /// wired a declaration into `spawn_entity` and forgot
    /// `spawn_game_start_entities`, so the player ship silently ran on a
    /// Rust-side fallback instead of its own authored block.
    #[test]
    fn every_kind_is_attached_at_every_one_of_its_spawn_sites() {
        for kind in FINE_SYSTEM_KINDS {
            for site in kind.spawn_sites {
                let src = read_non_test_source(site.file);
                let body = function_body(&src, site.func);
                assert!(
                    body.contains(kind.component),
                    "{}: {}::{} is declared an attachment site for `{}` but never                      mentions it. Either the attachment moved (point the site at where                      it went) or this path never got it — which for                      `spawn_game_start_entities` means the PLAYER ship runs without                      the declaration its own TOML authors.",
                    kind.key.as_str(),
                    site.file,
                    site.func,
                    kind.component
                );
            }
        }
    }

    /// AC: the nineteen synthesisers are GONE and cannot come back unnoticed.
    ///
    /// #885b stage 5d deleted every `default_*_ai_config()` /
    /// `default_*_target_selector_config()`. A re-introduced one would restore
    /// exactly what PRD #774 US7 forbids — automation supplied by Rust for a
    /// system nobody declared — and would do it silently, because strict mode
    /// only rejects what is *missing*, not what is quietly filled in. So the
    /// scan is over both halves: the definitions in `config.rs`, and any call on
    /// a spawn path.
    #[test]
    fn no_synthesiser_is_defined_or_called_anywhere() {
        let src = read_non_test_source("src/entities/config.rs");
        let defined: Vec<String> = src
            .lines()
            .filter_map(|line| {
                let t = line.trim_start();
                let rest = t
                    .strip_prefix("pub fn ")
                    .or_else(|| t.strip_prefix("fn "))?;
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                let is_synth = name.starts_with("default_")
                    && (name.ends_with("_ai_config") || name.ends_with("_target_selector_config"));
                is_synth.then_some(name)
            })
            .collect();
        assert!(
            defined.is_empty(),
            "src/entities/config.rs defines AI synthesiser(s) again: {defined:?}.              Stage 5d deleted all nineteen; a hull that wants a baseline authors it              in TOML, and strict AI-declaration mode is what makes omitting it an              error rather than a silent Rust default."
        );

        let sites: BTreeSet<(&str, &str)> = FINE_SYSTEM_KINDS
            .iter()
            .flat_map(|k| k.spawn_sites.iter().map(|s| (s.file, s.func)))
            .collect();
        assert!(!sites.is_empty(), "the scan has no spawn sites to walk");
        let mut called: Vec<String> = Vec::new();
        for (file, func) in sites {
            let src = read_non_test_source(file);
            let body = function_body(&src, func);
            for name in synthesiser_names(body) {
                called.push(format!("{file}::{func} calls {name}"));
            }
        }
        called.sort();
        assert!(
            called.is_empty(),
            "these spawn paths call an AI synthesiser again: {called:?}"
        );
    }

    // ── Gating ───────────────────────────────────────────────────────────────

    /// One shipped template through the real load path — include resolution
    /// included (issue #906), so a composed hull is judged on its resolved
    /// document rather than on the text of its own file.
    fn parse(rel: &str) -> EntityConfig {
        let path = crate_root().join(rel);
        let key = path.to_string_lossy().replace('\\', "/");
        crate::entity_includes::load_entity_config(&key)
            .unwrap_or_else(|e| panic!("{rel} must parse: {e}"))
    }

    /// The RESOLVED text of one shipped template (issue #906).
    ///
    /// For the handful of assertions that need the TOML itself rather than a
    /// parsed config — `from_toml_in_mode`, which has no resolver-aware wrapper,
    /// and `spawn`, which parses the text it is handed. Reading the file
    /// directly would hand them the UNRESOLVED text the day a hull declares
    /// `includes`, which is exactly the silent coverage loss this issue exists
    /// to prevent.
    fn shipped_toml(rel: &str) -> String {
        let path = crate_root().join(rel);
        let key = path.to_string_lossy().replace('\\', "/");
        crate::entity_includes::resolve_from_disk(&key)
            .unwrap_or_else(|e| panic!("{rel} must compose: {e}"))
            .toml
    }

    fn hull_files() -> Vec<String> {
        let dir = crate_root().join("assets/entities");
        let mut out: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&dir).expect("assets/entities must be readable") {
            let path = entry.expect("readable dir entry").path();
            if path.extension().is_some_and(|e| e == "toml") {
                out.push(
                    path.file_stem()
                        .expect("toml file has a stem")
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
        out.sort();
        out
    }

    /// Scenery is not an AI actor, and strict mode must never demand
    /// declarations from it.
    #[test]
    fn an_unarmed_static_entity_has_no_slots() {
        let station = parse("assets/entities/station_outpost.toml");
        assert!(station.behaviour.is_none(), "precondition");
        assert!(
            manifest(&station).is_empty(),
            "an unarmed station is hailable and damageable but not an AI actor: no \
             `[behaviour]`, no weapons ⇒ no AI-capable fine system at all."
        );
        assert!(strict_error(&station).is_none());
    }

    /// A minimal AI-bearing entity: `[behaviour]` and NOT ONE console section.
    ///
    /// This was the shape of the three-weapon escort now at
    /// `assets/entities/test/rng_coverage_lancer.toml` until #885b stage 5b, and
    /// it is the shape the gating rule below is about. It is a fixture rather
    /// than a shipped hull because every shipped hull now authors all five
    /// selectors, and authoring `[sensors_console.selector]` necessarily brings
    /// `[sensors_console]` into existence — so no shipped file omits them any
    /// more. The RULE is unchanged, and a fixture is the only thing left that
    /// can still exercise it.
    ///
    /// Since stage 5d it only loads under [`AiDeclarationMode::Lenient`]: strict
    /// mode is the default and rejects it, which is the whole point of it.
    const BARE_BEHAVIOUR_HULL: &str = r#"
name = "test.bare_behaviour_hull"
tags = ["ship"]

[hull]
hull_integrity = 100.0

[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
directive_kind = "Destroy"
base_priority = 40.0
"#;

    /// The gate that a per-hull migration is most likely to get wrong.
    #[test]
    fn ship_level_slots_gate_on_behaviour_alone_not_on_the_console_section() {
        let bare = EntityConfig::from_toml_in_mode(BARE_BEHAVIOUR_HULL, AiDeclarationMode::Lenient)
            .expect("the bare-behaviour fixture must parse in lenient mode");
        assert!(
            bare.sensors_console.is_none()
                && bare.navigation_console.is_none()
                && bare.repair.is_none()
                && bare.comms_console.is_none()
                && bare.weapons_console.is_none(),
            "precondition: the fixture declares no console section at all."
        );
        let keys = undeclared_keys(&bare);
        for expected in [
            "sensors_selector",
            "tactical_selector",
            "navigation_selector",
            "repair_selector",
            "comms_selector",
            "comms_response",
        ] {
            assert!(
                keys.contains(&expected.to_string()),
                "{expected} must be counted for an entity that declares no matching \
                 console section — `[behaviour]` is the only gate, so a worklist \
                 computed from 'which sections does this file have?' would \
                 under-report by five selectors per bare hull. Got: {keys:?}"
            );
        }
    }

    /// No shipped hull is in the state where a declaration exists but synthesis
    /// still fires. When one appears, #885b has to reckon with it.
    #[test]
    fn no_shipped_hull_declares_idle_out_of_band() {
        for stem in hull_files() {
            let c = parse(&format!("assets/entities/{stem}.toml"));
            for slot in manifest(&c) {
                assert_ne!(
                    slot.declared,
                    Declared::IdleLever,
                    "{stem}/{}: the out-of-band idle lever declares intent but does NOT \
                     stop the synthesiser — the selector is still built and attached. \
                     The first hull to pull it needs that asymmetry resolved.",
                    slot.key()
                );
            }
        }
    }

    /// Four of the five selectors cannot express an explicit idle at all, and
    /// the manifest records that distinctly rather than papering over it.
    #[test]
    fn only_tactical_of_the_five_selectors_has_an_idle_lever() {
        let with_lever: Vec<&str> = FINE_SYSTEM_KINDS
            .iter()
            .filter(|k| matches!(k.idle_lever, IdleLever::Field(_)))
            .map(|k| k.key.as_str())
            .collect();
        assert_eq!(
            with_lever,
            vec!["tactical_selector"],
            "Tactical's `selector_idle` is the only out-of-band idle field in the \
             schema. Adding a sibling for Sensors/Navigation/Repair/Comms-hail is the \
             schema change PRD #774 US7's 'or explicit idle' half needs for them — \
             update this when it lands."
        );
        let absent: Vec<&str> = FINE_SYSTEM_KINDS
            .iter()
            .filter(|k| k.idle_lever == IdleLever::Absent)
            .map(|k| k.key.as_str())
            .collect();
        assert_eq!(
            absent,
            vec![
                "sensors_selector",
                "navigation_selector",
                "repair_selector",
                "comms_selector"
            ],
            "these four can declare a policy but cannot declare idle. Strict mode must \
             not demand something the schema has no field for."
        );
    }

    // ── Strict mode ──────────────────────────────────────────────────────────

    /// **PRD #774 US7, as a switch.** The default load mode is strict, so a
    /// missing declaration is a load error on every path — and every shipped
    /// hull still loads.
    #[test]
    fn strict_mode_is_on_by_default_and_every_shipped_hull_still_loads() {
        assert_eq!(
            AiDeclarationMode::DEFAULT,
            AiDeclarationMode::Strict,
            "strict AI-declaration mode is the default since #885b stage 5d: with the \
             synthesisers deleted, an undeclared AI-capable fine system would simply \
             never act, and US7 requires that to be an error rather than a silence"
        );
        for stem in hull_files() {
            // Resolved first (issue #906): the default mode applies to the
            // COMPOSED document, which is the only thing that ever reaches
            // `EntityConfig` on a real load path.
            let src = shipped_toml(&format!("assets/entities/{stem}.toml"));
            EntityConfig::from_toml(&src)
                .unwrap_or_else(|e| panic!("{stem} must still load in the default mode: {e}"));
        }
    }

    /// **The completion gate for #885b, asserted directly.**
    ///
    /// Every shipped hull declares every one of its 172 AI-capable fine-system
    /// slots, so the strict default cannot stop a hull loading.
    ///
    /// This is deliberately stated as "strict mode ACCEPTS them" rather than as
    /// "the worklist is empty": the worklist and the load path are two different
    /// pieces of code, and a hull could in principle satisfy the manifest's
    /// gating while failing [`strict_error`]. Running the real strict load over
    /// every shipped file is what makes the claim about the switch rather than
    /// about the ledger.
    #[test]
    fn strict_mode_accepts_every_shipped_hull() {
        let mut checked = 0usize;
        for stem in hull_files() {
            // Resolved first (issue #906) — strict mode judges the COMPOSED
            // document, so a composed hull must not be read raw here.
            let src = shipped_toml(&format!("assets/entities/{stem}.toml"));
            let config = EntityConfig::from_toml(&src).expect("shipped entity parses");
            assert_eq!(
                strict_error(&config),
                None,
                "{stem}: this hull still owes a declaration, and with strict mode the \
                 default it will not load at all. Author the block named in the message."
            );
            EntityConfig::from_toml_in_mode(&src, AiDeclarationMode::Strict)
                .unwrap_or_else(|e| panic!("{stem} must load in STRICT mode: {e}"));
            checked += 1;
        }
        assert!(
            checked > 0,
            "no entity was checked — the scan is looking in the wrong place"
        );
    }

    /// …and strict mode still rejects, and still names the worklist, for
    /// anything that declares nothing.
    ///
    /// This was pointed at the three-weapon escort (then
    /// `assets/entities/ship_harrow_lancer.toml`, and since #954 a test fixture
    /// under `assets/entities/test/`) until #885b stage 5c authored its last
    /// fourteen policies. No shipped hull can play the part any more — that is
    /// the whole achievement — so the fixture takes over, which keeps the RULE
    /// under test rather than loosening the assertion to whatever the fleet
    /// happens to still owe.
    #[test]
    fn strict_mode_rejects_an_undeclared_policy_and_names_the_worklist() {
        let err = EntityConfig::from_toml_in_mode(BARE_BEHAVIOUR_HULL, AiDeclarationMode::Strict)
            .expect_err("the fixture declares nothing, so strict mode must reject it")
            .to_string();
        for (block, component) in [
            ("[captain_console.ai]", "CaptainAiPolicy"),
            // Since #1209 every helm axis decodes into the one keyed
            // `FineSystemAiPolicies` map, so the strict-mode error names that
            // component for the lateral slot rather than a per-axis newtype.
            ("[helm_console.lateral_ai]", "FineSystemAiPolicies"),
            ("[shields_console.ai_policy]", "ShieldsFocusAiPolicy"),
            ("[power.ai_policy]", "PowerAiPolicy"),
            ("[comms_console.ai]", "CommsResponseAiPolicy"),
        ] {
            assert!(
                err.contains(block) && err.contains(component),
                "the error must name the block to author and the runtime component that \
                 will otherwise never be attached ({block} / {component}): {err}"
            );
        }
        assert!(
            err.contains("or `idle = true` inside it"),
            "a POLICY can declare idle in band, and the message must say so — unlike \
             the four selectors with no idle field at all: {err}"
        );
    }

    /// …and the idle-less-selector wording is still produced, for anything that
    /// has NOT authored one.
    ///
    /// Four of the five selectors have no idle field, so strict mode must ask
    /// for the block rather than for an explicit idle it has no way to write.
    /// Exercised on the bare fixture now that no shipped hull is in that state.
    #[test]
    fn strict_mode_asks_for_the_block_where_no_idle_field_exists() {
        let err = EntityConfig::from_toml_in_mode(BARE_BEHAVIOUR_HULL, AiDeclarationMode::Strict)
            .expect_err("the fixture declares nothing, so strict mode must reject it")
            .to_string();
        assert!(
            err.contains("sensors_selector") && err.contains("[sensors_console.selector]"),
            "the error must name the selector block to author: {err}"
        );
        assert!(
            err.contains("NO idle field"),
            "the four idle-less selectors must say so rather than demanding an \
             unwritable explicit idle: {err}"
        );
    }

    #[test]
    fn strict_mode_accepts_scenery() {
        let src = shipped_toml("assets/entities/station_outpost.toml");
        EntityConfig::from_toml_in_mode(&src, AiDeclarationMode::Strict)
            .expect("scenery has no AI-capable fine system, so strict mode has nothing to demand");
    }

    // ── The manifest against the real spawner ────────────────────────────────

    fn spawn(toml: &str, what: &str) -> (App, Entity) {
        let config = EntityConfig::from_toml(toml)
            .unwrap_or_else(|e| panic!("{what} template must parse: {e}"));
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        let entity = {
            let mut commands = app.world_mut().commands();
            crate::entity_spawner::spawn_entity(
                &mut commands,
                &config,
                Vec3::ZERO,
                format!("manifest-{what}"),
                None,
            )
        };
        app.update();
        (app, entity)
    }

    /// The fine-system id each helm axis's policy is keyed under in the one
    /// [`crate::ship::helm_ai::FineSystemAiPolicies`] map (issue #1209); `None`
    /// for every non-helm kind. The six axes share one component now, so the
    /// cross-check resolves the entry by id rather than reading six newtypes.
    fn helm_axis_system_id(key: FineSystemKey) -> Option<crate::messages::SystemId> {
        use crate::system_registry as sr;
        Some(match key {
            FineSystemKey::Engines => sr::helm_thrust_system_id(),
            FineSystemKey::Steering => sr::helm_steering_system_id(),
            FineSystemKey::Lateral => sr::lateral_thrust_system_id(),
            FineSystemKey::Vertical => sr::vertical_thrust_system_id(),
            FineSystemKey::Impulse => sr::helm_impulse_system_id(),
            FineSystemKey::Boost => sr::helm_boost_system_id(),
            _ => return None,
        })
    }

    /// The policy the real spawner attached for one slot, if any.
    ///
    /// Exhaustive over [`FineSystemKey`], so a twentieth fine system cannot be
    /// added without the compiler demanding a cross-check for it.
    fn attached(w: &World, e: Entity, slot: &Slot) -> Option<Attached> {
        let map = |m: Option<&std::collections::HashMap<String, AiPolicy>>| {
            let id = slot.instance.as_ref()?;
            m?.get(id).cloned().map(Attached::Policy)
        };
        match slot.kind.key {
            FineSystemKey::Captain => w
                .get::<crate::captain_plugin::CaptainAiPolicy>(e)
                .map(|c| Attached::Policy(c.0.clone())),
            FineSystemKey::CommsResponse => w
                .get::<crate::console::comms::server::CommsResponseAiPolicy>(e)
                .map(|c| Attached::Policy(c.0.clone())),
            // The six helm axes now decode into ONE keyed `FineSystemAiPolicies`
            // map (issue #1209); each is looked up by its fine-system id, the
            // ship-level analogue of the per-bank `map(..)` closure above.
            FineSystemKey::Engines
            | FineSystemKey::Steering
            | FineSystemKey::Lateral
            | FineSystemKey::Vertical
            | FineSystemKey::Impulse
            | FineSystemKey::Boost => {
                let id = helm_axis_system_id(slot.kind.key)
                    .expect("the six helm axes each map to a fine-system id");
                w.get::<crate::ship::helm_ai::FineSystemAiPolicies>(e)
                    .and_then(|c| c.0.get(&id).cloned())
                    .map(Attached::Policy)
            }
            FineSystemKey::ShieldsFocus => w
                .get::<crate::ship::shields::ShieldsFocusAiPolicy>(e)
                .map(|c| Attached::Policy(c.0.clone())),
            FineSystemKey::Power => w
                .get::<crate::power_plugin::PowerAiPolicy>(e)
                .map(|c| Attached::Policy(c.0.clone())),
            FineSystemKey::TorpedoMagazine => w
                .get::<crate::weapons_plugin::TorpedoMagazineAiPolicy>(e)
                .map(|c| Attached::Policy(c.0.clone())),
            FineSystemKey::WeaponsDoctrine => w
                .get::<crate::weapons_plugin::WeaponsDoctrineAiPolicy>(e)
                .map(|c| Attached::Policy(c.0.clone())),
            FineSystemKey::PhaserBank => map(w
                .get::<crate::weapons_plugin::PhaserBankAiPolicies>(e)
                .map(|c| &c.0)),
            FineSystemKey::BlasterBank => map(w
                .get::<crate::weapons_plugin::BlasterBankAiPolicies>(e)
                .map(|c| &c.0)),
            FineSystemKey::TorpedoTube => map(w
                .get::<crate::weapons_plugin::TorpedoTubeAiPolicies>(e)
                .map(|c| &c.0)),
            FineSystemKey::SensorsSelector => w
                .get::<crate::ship::sensors::SensorsTargetSelector>(e)
                .map(|c| Attached::Selector(c.selector.clone())),
            FineSystemKey::TacticalSelector => w
                .get::<crate::weapons_plugin::TacticalTargetSelector>(e)
                .map(|c| Attached::Selector(c.selector.clone())),
            FineSystemKey::NavigationSelector => w
                .get::<crate::console::navigation::NavigationTargetSelector>(e)
                .map(|c| Attached::Selector(c.selector.clone())),
            FineSystemKey::RepairSelector => w
                .get::<crate::console::repair::server::RepairTargetSelector>(e)
                .map(|c| Attached::Selector(c.selector.clone())),
            FineSystemKey::CommsSelector => w
                .get::<crate::console::comms::server::CommsTargetSelector>(e)
                .map(|c| Attached::Selector(c.selector.clone())),
            // The operate kinds (issue #1162) attach no per-hull AI policy /
            // selector component: their behaviour is directive-driven, so there
            // is nothing for this cross-check to find. Never reached in practice
            // (they are absent from `FINE_SYSTEM_KINDS`, which is all the callers
            // iterate); present only to keep the match exhaustive.
            FineSystemKey::Tractor
            | FineSystemKey::Umbilical
            | FineSystemKey::Dock
            | FineSystemKey::ExternalRepair => None,
        }
    }

    #[derive(Debug, PartialEq)]
    enum Attached {
        Policy(AiPolicy),
        Selector(TargetSelector),
    }

    /// How many declarations of one kind the real spawner attached: 1/0 for a
    /// ship-level system, the policy map's length for a per-weapon one.
    ///
    /// The counterpart to [`attached`]: that one catches the manifest claiming a
    /// slot the spawner never fills, this one catches the spawner filling a slot
    /// the manifest never claims — the under-report that would let a whole
    /// system drop off #885b's worklist unnoticed.
    fn attached_count(w: &World, e: Entity, key: FineSystemKey) -> usize {
        let one = |present: bool| usize::from(present);
        match key {
            FineSystemKey::Captain => {
                one(w.get::<crate::captain_plugin::CaptainAiPolicy>(e).is_some())
            }
            FineSystemKey::CommsResponse => one(w
                .get::<crate::console::comms::server::CommsResponseAiPolicy>(e)
                .is_some()),
            // Each helm axis's entry in the one keyed `FineSystemAiPolicies` map
            // (issue #1209): present ⇒ 1, exactly as a standalone newtype's
            // presence used to read, so the manifest count still mirrors the
            // spawner one axis at a time.
            FineSystemKey::Engines
            | FineSystemKey::Steering
            | FineSystemKey::Lateral
            | FineSystemKey::Vertical
            | FineSystemKey::Impulse
            | FineSystemKey::Boost => {
                let id = helm_axis_system_id(key)
                    .expect("the six helm axes each map to a fine-system id");
                w.get::<crate::ship::helm_ai::FineSystemAiPolicies>(e)
                    .map_or(0, |c| usize::from(c.0.contains_key(&id)))
            }
            FineSystemKey::ShieldsFocus => one(w
                .get::<crate::ship::shields::ShieldsFocusAiPolicy>(e)
                .is_some()),
            FineSystemKey::Power => one(w.get::<crate::power_plugin::PowerAiPolicy>(e).is_some()),
            FineSystemKey::TorpedoMagazine => one(w
                .get::<crate::weapons_plugin::TorpedoMagazineAiPolicy>(e)
                .is_some()),
            FineSystemKey::WeaponsDoctrine => one(w
                .get::<crate::weapons_plugin::WeaponsDoctrineAiPolicy>(e)
                .is_some()),
            FineSystemKey::SensorsSelector => one(w
                .get::<crate::ship::sensors::SensorsTargetSelector>(e)
                .is_some()),
            FineSystemKey::TacticalSelector => one(w
                .get::<crate::weapons_plugin::TacticalTargetSelector>(e)
                .is_some()),
            FineSystemKey::NavigationSelector => one(w
                .get::<crate::console::navigation::NavigationTargetSelector>(e)
                .is_some()),
            FineSystemKey::RepairSelector => one(w
                .get::<crate::console::repair::server::RepairTargetSelector>(e)
                .is_some()),
            FineSystemKey::CommsSelector => one(w
                .get::<crate::console::comms::server::CommsTargetSelector>(e)
                .is_some()),
            FineSystemKey::PhaserBank => w
                .get::<crate::weapons_plugin::PhaserBankAiPolicies>(e)
                .map_or(0, |c| c.0.len()),
            FineSystemKey::BlasterBank => w
                .get::<crate::weapons_plugin::BlasterBankAiPolicies>(e)
                .map_or(0, |c| c.0.len()),
            FineSystemKey::TorpedoTube => w
                .get::<crate::weapons_plugin::TorpedoTubeAiPolicies>(e)
                .map_or(0, |c| c.0.len()),
            // Operate kinds (issue #1162): no per-hull policy component, so zero
            // attached. Never reached (absent from `FINE_SYSTEM_KINDS`).
            FineSystemKey::Tractor
            | FineSystemKey::Umbilical
            | FineSystemKey::Dock
            | FineSystemKey::ExternalRepair => 0,
        }
    }

    /// AC: the manifest's gating is what the real spawner does.
    ///
    /// Every slot the manifest claims must actually be filled at spawn — and,
    /// since #885b stage 5d, filled from the hull's OWN authored block, because
    /// there is nothing else left to fill it from. Change the spawn gate without
    /// changing [`slots_of_kind`] and this fails naming the hull and the system.
    #[test]
    fn the_manifest_matches_the_real_spawner() {
        let mut checked = 0usize;
        for stem in hull_files() {
            // Resolved first (issue #906): `spawn` below is handed this same
            // text, so a composed hull would otherwise be spawned from its
            // unresolved file and the manifest compared against the wrong ship.
            let src = shipped_toml(&format!("assets/entities/{stem}.toml"));
            let config = EntityConfig::from_toml(&src).expect("shipped entity parses");
            let slots = manifest(&config);
            if slots.is_empty() {
                continue;
            }
            let (app, e) = spawn(&src, &stem);
            let w = app.world();
            for slot in &slots {
                let got = attached(w, e, slot).unwrap_or_else(|| {
                    panic!(
                        "{stem}/{}: the manifest claims this AI-capable fine system \
                         exists on this hull, but the real spawn path attached nothing \
                         for it. The gating in slots_of_kind has drifted from \
                         spawner.rs — over-reporting inflates the worklist with systems \
                         that do not exist.",
                        slot.key()
                    )
                });
                assert_ne!(
                    slot.declared,
                    Declared::Nothing,
                    "{stem}/{}: an undeclared slot on a hull that LOADED. Since #885b \
                     stage 5d strict mode is the default, so this is unreachable \
                     through `from_toml` — reaching it means the load path stopped \
                     checking.",
                    slot.key()
                );
                assert!(
                    matches!(got, Attached::Policy(_) | Attached::Selector(_)),
                    "{stem}/{}: the attached declaration must decode",
                    slot.key()
                );
                checked += 1;
            }
            // …and the other direction: no kind may be attached more times than
            // the manifest claims slots for it. Gating the Repair selector on
            // `[repair]` (the plausible-but-wrong "which sections does this hull
            // declare?" rule) would pass every assertion above and silently drop
            // that system off the worklist for every hull without the section.
            for kind in FINE_SYSTEM_KINDS {
                let claimed = slots_of_kind(kind, &config).len();
                assert_eq!(
                    attached_count(w, e, kind.key),
                    claimed,
                    "{stem}/{}: the manifest claims {claimed} slot(s) of this kind but \
                     the real spawn path attached a different number. Under-reporting \
                     drops a whole fine system out of #885b's worklist without \
                     changing anything a reader would notice.",
                    kind.key.as_str()
                );
            }
        }
        assert!(
            checked > 0,
            "no slots were checked — the scan is looking in the wrong place"
        );
    }

    /// The negative half: scenery gets nothing attached, so strict mode's
    /// silence about it is real rather than a gap in the manifest.
    #[test]
    fn an_entity_with_no_slots_gets_no_ai_declaration_attached() {
        // Resolved first (issue #906) — `spawn` below is handed this same text.
        let src = shipped_toml("assets/entities/station_outpost.toml");
        let config = EntityConfig::from_toml(&src).expect("station parses");
        assert!(manifest(&config).is_empty(), "precondition");

        let (app, e) = spawn(&src, "station");
        let w = app.world();
        for kind in FINE_SYSTEM_KINDS {
            let probe = Slot {
                kind,
                instance: Some("any".to_string()),
                declared: Declared::Nothing,
            };
            assert!(
                attached(w, e, &probe).is_none(),
                "station/{}: an entity with no `[behaviour]` and no weapons must get no \
                 AI declaration attached at all.",
                kind.key.as_str()
            );
        }
    }

    // ── The rendered surface ─────────────────────────────────────────────────

    /// The UNDECLARED rendering, on the fixture — no shipped hull produces one
    /// any more, which is stage 5c's result rather than a reason to stop
    /// checking the rendering.
    #[test]
    fn manifest_lines_name_the_block_to_author() {
        let bare = EntityConfig::from_toml_in_mode(BARE_BEHAVIOUR_HULL, AiDeclarationMode::Lenient)
            .expect("the bare-behaviour fixture must parse in lenient mode");
        let lines = manifest_lines("bare_behaviour_hull", &bare);
        assert_eq!(lines.len(), manifest(&bare).len(), "one line per slot");
        let captain = lines
            .iter()
            .find(|l| l.contains(" captain "))
            .expect("the fixture's captain slot is rendered");
        assert!(
            captain.contains("UNDECLARED") && captain.contains("[captain_console.ai]"),
            "a rendered line must be actionable on its own: {captain}"
        );
    }

    /// …and every slot on every shipped hull renders as DECLARED.
    ///
    /// The other half of the same surface: the load-time report a developer
    /// actually reads must show no `SYNTHESISED` line for shipped content, or
    /// the ledger and the report disagree about whether #885b is done.
    #[test]
    fn no_shipped_hull_renders_an_undeclared_line() {
        for stem in hull_files() {
            let c = parse(&format!("assets/entities/{stem}.toml"));
            for line in manifest_lines(&stem, &c) {
                assert!(
                    !line.contains("UNDECLARED"),
                    "{stem}: this slot is undeclared, and with the synthesisers deleted \
                     it would never act at all: {line}"
                );
            }
        }
    }

    #[test]
    fn a_declared_slot_renders_as_declared() {
        let cruiser = parse("assets/entities/alliance_cruiser.toml");
        assert!(
            cruiser
                .captain_console
                .as_ref()
                .is_some_and(|c| c.ai.is_some()),
            "precondition: the Alliance Cruiser hand-authors `[captain_console.ai]`"
        );
        let lines = manifest_lines("alliance_cruiser", &cruiser);
        let captain = lines
            .iter()
            .find(|l| l.contains(" captain "))
            .expect("captain slot rendered");
        assert!(
            captain.contains("declared") && !captain.contains("UNDECLARED"),
            "an authored block must not be reported as undeclared: {captain}"
        );
    }
}
