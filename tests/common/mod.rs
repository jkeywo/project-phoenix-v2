//! Shared test harness for the headless determinism guards.
//!
//! [`SimFixture`] builds a headless simulation app from a [`HeadlessArgs`] plus
//! the three registration-order test seams — `physics_last`,
//! `registration_order` and `extra_registration_probes` — that used to sit on
//! `HeadlessArgs` itself, each documented there as "not a command-line flag and
//! never parsed from one". They are genuinely test-only, so they now live off
//! the CLI-shaped session type: folded into
//! [`SimRegistrationOverrides`](project_phoenix::headless::SimRegistrationOverrides)
//! and threaded through
//! [`build_headless_app_with`](project_phoenix::headless::build_headless_app_with),
//! leaving `HeadlessArgs` modelling only real CLI flags. See `SimPluginOptions`
//! (`src/server_app.rs`) for what each knob proves.
//!
//! Included with `mod common;` by the test binaries that need it. It is a
//! subdirectory `mod.rs` on purpose: cargo compiles every top-level `tests/*.rs`
//! as its own integration test, but not a file nested under `tests/common/`.
#![allow(dead_code)]

use bevy::prelude::App;
use project_phoenix::headless::{
    build_headless_app_with, run, HeadlessArgs, SimRegistrationOverrides,
};
use project_phoenix::server_app::{RegistrationOrder, RegistrationProbes};

/// Builds — and optionally pumps — a headless simulation app for tests,
/// carrying the registration-order knobs that are not real CLI flags.
///
/// A thin builder over [`build_headless_app_with`]: `SimFixture::new(args)`
/// starts at the production configuration (physics first, canonical order, no
/// probes), and the chainable setters below turn the individual test seams.
pub struct SimFixture {
    args: HeadlessArgs,
    overrides: SimRegistrationOverrides,
}

impl SimFixture {
    /// A fixture over `args`, with every registration-order knob at its
    /// production default, so `build()` alone reproduces `build_headless_app`.
    pub fn new(args: HeadlessArgs) -> Self {
        Self {
            args,
            overrides: SimRegistrationOverrides::default(),
        }
    }

    /// Register the physics plugin last instead of first (issue #896).
    pub fn physics_last(mut self, physics_last: bool) -> Self {
        self.overrides.physics_last = physics_last;
        self
    }

    /// Permute the `SimSet`-chain plugin registration order (issue #899).
    pub fn registration_order(mut self, order: RegistrationOrder) -> Self {
        self.overrides.registration_order = order;
        self
    }

    /// Fold a mutation-proof probe pair into the shuffled group (issue #899).
    pub fn extra_registration_probes(mut self, probes: Option<RegistrationProbes>) -> Self {
        self.overrides.extra_registration_probes = probes;
        self
    }

    /// The args this fixture builds from — e.g. to read `args.max_ticks`.
    pub fn args(&self) -> &HeadlessArgs {
        &self.args
    }

    /// Build the app without running it, for callers that drive the frame loop
    /// by hand.
    pub fn build(&self) -> App {
        build_headless_app_with(&self.args, self.overrides).expect("app should build")
    }

    /// Build the app and pump it for `args.max_ticks` frames, returning it.
    pub fn build_and_run(&self) -> App {
        let mut app = self.build();
        run(&mut app, self.args.max_ticks);
        app
    }
}
