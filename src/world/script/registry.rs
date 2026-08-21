//! One declaration, two outputs: the engine registration AND its editor
//! descriptor (issue #1238 quick win).
//!
//! The author-callable vocabulary used to be written down *twice* — once as the
//! `register_fn`/`register_type` calls the two engines
//! ([`loading_engine`](super::engine::loading_engine) /
//! [`runtime_engine`](super::engine::runtime_engine)) run, and once by hand as a
//! `HOST_FNS: &[HostFn]` slice the editor's autocomplete reads. Rhai's own
//! `gen_fn_signatures` needs the `metadata` feature phoenix deliberately keeps
//! off (it would bloat the wasm), so that second list was a hand-maintained
//! mirror pinned to the first by a drift test.
//!
//! [`HostRegistry`] removes the second copy. It wraps the engine being built and
//! also collects a [`HostFn`] descriptor whenever the [`host_fn!`] macro is used
//! in place of a bare `register_fn`. Because a descriptor is emitted *by* the
//! registration call itself, it cannot name a function the engine never
//! registered (a "phantom"), and an exposed verb cannot be registered without
//! also being described. The editor registry ([`super::authoring::host_fns`]) is
//! then *derived* by running the same registration the engines run and taking
//! the descriptors it collected — see
//! [`collect_host_fn_descriptors`](super::engine::collect_host_fn_descriptors).
//!
//! Not every registration is exposed to the editor: the read/write `deadlines` /
//! `commitments` handles, the `flt(…)` marker, the name-resolving `spawn_entity`
//! / `add_objective` family and the newer civilian/infrastructure levers are
//! registered with a plain `engine.register_fn` (no descriptor), exactly as
//! before. Those sites carry no [`host_fn!`]; only the verbs the autocomplete
//! offers do.

use std::ops::{Deref, DerefMut};

use rhai::Engine;

use super::authoring::HostFn;

/// An engine under construction, plus the editor descriptors emitted alongside
/// its registrations.
///
/// Derefs to the wrapped [`Engine`], so the `register_fn` / `register_type` /
/// `register_indexer_*` calls a registration module already makes work unchanged
/// through the wrapper — a bare `engine.register_fn(name, closure)` registers a
/// verb the editor is NOT told about, exactly as it did before. The [`host_fn!`]
/// macro is the opt-in that *also* records a [`HostFn`] descriptor, so the two
/// halves of one exposed verb — its runtime registration and its autocomplete
/// entry — are declared in one place and cannot drift.
pub(crate) struct HostRegistry {
    engine: Engine,
    descriptors: Vec<HostFn>,
}

impl HostRegistry {
    /// Wrap a fresh engine, collecting no descriptors yet.
    pub(crate) fn new(engine: Engine) -> Self {
        Self {
            engine,
            descriptors: Vec::new(),
        }
    }

    /// Record one editor descriptor. Called by [`host_fn!`] beside the matching
    /// `register_fn`, never on its own.
    pub(crate) fn record(&mut self, host_fn: HostFn) {
        self.descriptors.push(host_fn);
    }

    /// The wrapped engine as a plain `&mut Engine`, for the few sites that call a
    /// helper taking `&mut Engine` directly (`register_real_lit`).
    pub(crate) fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// Consume the wrapper, keeping the built engine and discarding the
    /// descriptors — what the two public engine builders do.
    pub(crate) fn into_engine(self) -> Engine {
        self.engine
    }

    /// Consume the wrapper, keeping the collected descriptors and discarding the
    /// engine — what the descriptor harvest does.
    pub(crate) fn into_descriptors(self) -> Vec<HostFn> {
        self.descriptors
    }
}

impl Deref for HostRegistry {
    type Target = Engine;

    fn deref(&self) -> &Engine {
        &self.engine
    }
}

impl DerefMut for HostRegistry {
    fn deref_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }
}

/// Register an author-callable host fn on the engine AND record its editor
/// descriptor, from one declaration.
///
/// Used in place of `engine.register_fn(name, closure)` at the sites the editor
/// offers for autocomplete. Everything after the closure is the descriptor: the
/// `receiver` the method hangs off (`""` for a top-level call), the completion
/// `category`, the parameter names, and a one-line `summary`. An overloaded verb
/// registers its other overloads with a bare `register_fn` — one descriptor per
/// callable name, matching how the hand-maintained mirror collapsed overloads.
///
/// ```ignore
/// host_fn!(
///     engine, "complete_objective",
///     receiver = "effects", category = "effect",
///     params = ["id"],
///     summary = "Mark the objective complete.",
///     |sink: &mut EffectSink, id: ImmutableString| {
///         sink.push(ActionCmd::CompleteObjective { id: id.to_string() });
///     },
/// );
/// ```
macro_rules! host_fn {
    (
        $reg:expr, $name:literal,
        receiver = $receiver:literal,
        category = $category:literal,
        params = [ $($param:literal),* $(,)? ],
        summary = $summary:expr,
        $closure:expr $(,)?
    ) => {{
        $reg.register_fn($name, $closure);
        $reg.record($crate::world::script::authoring::HostFn {
            name: $name,
            receiver: $receiver,
            params: &[ $($param),* ],
            category: $category,
            summary: $summary,
        });
    }};
}

pub(crate) use host_fn;
