//! Test-only harness for driving a `[script]`-authored world's handlers
//! (issue #984).
//!
//! Every shipped world is script-authored now, so a test that used to read
//! `WorldConfig::triggers` and their `TriggerAction` lists has nothing to read:
//! a scripted trigger fires with an EMPTY action list and its effects come from
//! running the handler. Several tests across the crate need exactly that —
//! `world::config`'s combat_test structure pin, `world::content`'s wave-clock
//! chain, `console::weapons`' raid-cruiser doctrine check, `entities::loader`'s
//! shipped-override merge walk — so the "compile the world's inline `[script]`,
//! then call handlers on a runtime host" dance lives here once instead of four
//! times.
//!
//! It deliberately mirrors production rather than abbreviating it:
//! [`compile_scripts`] is the same compiler `compile_world_scripts` runs, the
//! ASTs are the same retained ones, and [`RuntimeHost::call`] is the same entry
//! point `tick_trigger_pipeline` uses — so what a test reads here is what the
//! sim will do, not a re-implementation of it.

use std::collections::BTreeMap;

use rhai::{Map, AST};
use vellum_script::ScriptSource;

use crate::world::config::TriggerAction;
use crate::world::flags::FlagStore;
use crate::world::script::effects::BufferedEffect;
use crate::world::script::engine::{RuntimeHost, ScriptTrigger};
use crate::world::script::load::compile_scripts;
use crate::world::script::schedule::{CallEffects, SchedClock, TickBudget};

/// A compiled world script plus the host that runs its handlers.
pub struct ScriptedWorld {
    /// The triggers the unit's top level registered, in registration order —
    /// the same order and the same `Trigger` structs `merge_script_triggers`
    /// appends to the runtime's trigger states.
    pub triggers: Vec<ScriptTrigger>,
    asts: BTreeMap<String, AST>,
    host: RuntimeHost,
}

impl ScriptedWorld {
    /// Compile every INLINE `[script.*]` body in a world TOML, under the same
    /// `<world>#script.<key>` virtual paths the loader lifts them to.
    ///
    /// Panics on any finding: a shipped world whose script does not compile is
    /// not a case any caller wants to interpret.
    pub fn compile(world_path: &str, world_toml: &str) -> Self {
        let value: toml::Value = toml::from_str(world_toml).expect("world must be valid TOML");
        let sources: Vec<ScriptSource> = value
            .get("script")
            .and_then(|s| s.as_table())
            .map(|table| {
                table
                    .iter()
                    .filter_map(|(key, v)| {
                        v.as_str().map(|source| ScriptSource {
                            path: format!("{world_path}#script.{key}"),
                            source: source.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let compiled = compile_scripts(&sources);
        assert!(
            compiled.findings.is_empty(),
            "{world_path} script must compile cleanly: {:?}",
            compiled.findings
        );
        Self {
            triggers: compiled.script_triggers,
            asts: compiled.asts,
            host: RuntimeHost::new(),
        }
    }

    /// Call a named fn against `flags` at a zero clock.
    ///
    /// Deferred work is stamped against [`SchedClock::ZERO`], so a
    /// `DelayedAction`'s `fire_at_elapsed` reads back as the authored delay in
    /// seconds. A caller that models a running clock wants [`Self::call_at`].
    pub fn call(&self, fn_name: &str, flags: &FlagStore) -> CallEffects {
        self.call_at(fn_name, flags, &SchedClock::ZERO)
    }

    /// Call a named fn against `flags` on a fresh per-call budget at `clock`,
    /// returning everything it produced.
    ///
    /// A fresh [`TickBudget`] per call because a fixture drives a whole
    /// scenario's worth of handlers in a loop, which the per-TICK call cap is
    /// not meant to bound; the per-CALL operation cap on the engine still
    /// applies, so a runaway handler still fails.
    pub fn call_at(&self, fn_name: &str, flags: &FlagStore, clock: &SchedClock) -> CallEffects {
        let path = self
            .asts
            .keys()
            .next()
            .expect("the world must author a [script] block")
            .clone();
        let ast = &self.asts[&path];
        let mut budget = TickBudget::new();
        self.host
            .call(&mut budget, clock, ast, &path, fn_name, flags, Map::new())
    }

    /// Call the handler of trigger `index`, in registration order.
    pub fn fire(&self, index: usize, flags: &FlagStore, clock: &SchedClock) -> CallEffects {
        self.call_at(&self.triggers[index].handler, flags, clock)
    }

    /// The declarative [`TriggerAction`]s a handler buffers — the name-resolving
    /// effects (`spawn_entity`, `add_objective`, `add_faction_enemy`) the
    /// applier re-dispatches, which is what a structure test wants to read.
    /// Resolved command effects and flag writes are dropped.
    pub fn actions(&self, fn_name: &str, flags: &FlagStore) -> Vec<TriggerAction> {
        buffered_actions(self.call(fn_name, flags).commands)
    }

    /// [`Self::actions`], addressed by trigger index.
    pub fn fired_actions(&self, index: usize, flags: &FlagStore) -> Vec<TriggerAction> {
        self.actions(&self.triggers[index].handler, flags)
    }
}

/// Keep only the unresolved [`TriggerAction`]s out of a drained effect buffer.
pub fn buffered_actions(effects: Vec<BufferedEffect>) -> Vec<TriggerAction> {
    effects
        .into_iter()
        .filter_map(|e| match e {
            BufferedEffect::Action(a) => Some(a),
            BufferedEffect::Cmd(_) => None,
        })
        .collect()
}
