//! Complete invocation of a retained World script. Adapters own when to call
//! and how to apply its result; this module owns the shared tick budget, AST
//! lookup and the current layer's read context.

use super::{
    layered_flag_chain, ScriptCallContext, WorldContentRuntime, WorldLayerMap, WorldScriptRuntime,
};
use crate::world::script::{
    comms::{enter_node_scoped, EnterError, ScriptDialogueNode},
    schedule::{CallEffects, TickBudget},
};

#[derive(Debug)]
pub struct MissingScriptUnit;

#[derive(Debug)]
pub enum DialogueInvocationError {
    /// Preflight declined without spending an attempt or tripping the budget.
    BudgetUnavailable,
    MissingUnit,
    Node(EnterError),
}

impl WorldScriptRuntime {
    /// Retain charges within a tick, including calls made by another adapter.
    /// Drains also call this at their existing entry point when no call follows.
    pub(crate) fn prepare_invocation_tick(&mut self, tick: u64) {
        if self.budget_tick != tick {
            self.reset_invocation_budget(tick);
        }
    }

    /// Restore starts with a fresh budget even if bootstrap reached this tick.
    /// Budget usage is transient and is neither captured nor folded.
    pub(crate) fn reset_invocation_budget(&mut self, tick: u64) {
        self.budget = TickBudget::new();
        self.budget_tick = tick;
    }

    pub(crate) fn invoke_effects(
        &mut self,
        context: &ScriptCallContext<'_>,
        content: &WorldContentRuntime,
        layers: Option<&WorldLayerMap>,
    ) -> Result<CallEffects, MissingScriptUnit> {
        self.prepare_invocation_tick(context.clock.tick);
        let ast = self
            .asts
            .get(context.script_path)
            .ok_or(MissingScriptUnit)?;
        let flags = layered_flag_chain(context.origin_layer.as_deref(), &content.flags, layers)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        Ok(self.host.call_scoped(
            &mut self.budget,
            &context.clock,
            ast,
            context.script_path,
            context.function,
            &flags,
            &content.deadlines,
            &content.commitments,
            &content.evidence,
            context.origin_layer.as_deref(),
            rhai::Map::new(),
        ))
    }

    pub(crate) fn invoke_dialogue(
        &mut self,
        context: &ScriptCallContext<'_>,
        content: &WorldContentRuntime,
        layers: Option<&WorldLayerMap>,
    ) -> Result<(CallEffects, Option<ScriptDialogueNode>), DialogueInvocationError> {
        self.prepare_invocation_tick(context.clock.tick);
        // Unlike call_scoped's attempt, the Comms preflight has always been a
        // read: at the call cap it refuses without setting the tripped latch.
        if !self.budget.can_admit() {
            return Err(DialogueInvocationError::BudgetUnavailable);
        }
        let ast = self
            .asts
            .get(context.script_path)
            .ok_or(DialogueInvocationError::MissingUnit)?;
        let flags = layered_flag_chain(context.origin_layer.as_deref(), &content.flags, layers)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        enter_node_scoped(
            &self.host,
            &mut self.budget,
            &context.clock,
            ast,
            context.script_path,
            context.function,
            &flags,
            &content.deadlines,
            &content.commitments,
            &content.evidence,
            context.origin_layer.as_deref(),
        )
        .map_err(DialogueInvocationError::Node)
    }

    #[cfg(test)]
    pub(crate) fn seed_invocation_budget(&mut self, tick: u64, budget: TickBudget) {
        self.budget_tick = tick;
        self.budget = budget;
    }

    #[cfg(test)]
    pub(crate) fn invocation_budget(&self) -> (u64, &TickBudget) {
        (self.budget_tick, &self.budget)
    }
}

#[cfg(test)]
#[path = "script_invocation_tests.rs"]
mod tests;
