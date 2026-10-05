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
//!   `crates/phoenix-simulation/src/entities/config.rs` and every spawn site for `default_*_ai_config` /
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
    /// The stable manifest key: the identifier a slot is reported under in
    /// [`manifest_lines`] and [`strict_error`], so renaming one is a visible
    /// diff over the whole report rather than a quiet reshuffle.
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
    /// Every attachment site for this kind's `component`. Generic and GameStart
    /// ships share one installer; Comms conversion remains in its owning helper.
    /// Each declared site is checked against the source.
    pub spawn_sites: &'static [EvalSite],
}

const fn site(file: &'static str, func: &'static str) -> EvalSite {
    EvalSite { file, func }
}

/// The single shared capability installer used by generic and GameStart spawns.
const SHIP_INSTALLER: &[EvalSite] = &[site(
    "crates/phoenix-simulation/src/entities/ship_spawn.rs",
    "install",
)];
/// Comms declarations are resolved by the owning helper called by the installer.
const COMMS_HELPER: &[EvalSite] = &[site(
    "crates/phoenix-simulation/src/console/comms/server.rs",
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
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::Engines,
        host: &ai_flag_hosts::HELM_ENGINES,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::Steering,
        host: &ai_flag_hosts::HELM_STEERING,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::Lateral,
        host: &ai_flag_hosts::HELM_LATERAL,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::Vertical,
        host: &ai_flag_hosts::HELM_VERTICAL,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::Impulse,
        host: &ai_flag_hosts::HELM_IMPULSE,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::Boost,
        host: &ai_flag_hosts::HELM_BOOST,
        component: "FineSystemAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::PhaserBank,
        host: &ai_flag_hosts::PHASER_BANK,
        component: "PhaserBankAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::BlasterBank,
        host: &ai_flag_hosts::BLASTER_BANK,
        component: "BlasterBankAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::TorpedoTube,
        host: &ai_flag_hosts::TORPEDO_TUBE,
        component: "TorpedoTubeAiPolicies",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::WeaponsDoctrine,
        host: &ai_flag_hosts::WEAPONS_DOCTRINE,
        component: "WeaponsDoctrineAiPolicy",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::TorpedoMagazine,
        host: &ai_flag_hosts::TORPEDO_MAGAZINE,
        component: "TorpedoMagazineAiPolicy",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::ShieldsFocus,
        host: &ai_flag_hosts::SHIELDS_FOCUS,
        component: "ShieldsFocusAiPolicy",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::Power,
        host: &ai_flag_hosts::POWER_ALLOCATION,
        component: "PowerAiPolicy",
        idle_lever: IdleLever::InBandPolicy,
        spawn_sites: SHIP_INSTALLER,
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
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::TacticalSelector,
        host: &ai_flag_hosts::TACTICAL_SELECTOR,
        component: "TacticalTargetSelector",
        idle_lever: IdleLever::Field("[weapons_console] selector_idle"),
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::NavigationSelector,
        host: &ai_flag_hosts::NAVIGATION_SELECTOR,
        component: "NavigationTargetSelector",
        idle_lever: IdleLever::Absent,
        spawn_sites: SHIP_INSTALLER,
    },
    FineSystemKind {
        key: FineSystemKey::RepairSelector,
        host: &ai_flag_hosts::REPAIR_SELECTOR,
        component: "RepairTargetSelector",
        idle_lever: IdleLever::Absent,
        spawn_sites: SHIP_INSTALLER,
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
#[path = "ai_declaration_manifest_source_scan_tests.rs"]
pub(crate) mod source_scan;

#[cfg(test)]
#[path = "ai_declaration_manifest_tests.rs"]
mod tests;
