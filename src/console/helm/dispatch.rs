//! Host helm router — the admitted-command dispatch seam for the per-axis Helm
//! wire targets.
//!
//! This module is the single file named by the PASM entity
//! `host-helm-control-router`'s dependency on the host command-admission seam.
//! It exists as its own module (issue #745) so that dependency is a real,
//! observed code edge (the `use crate::command_admission::…;` below) rather than
//! a claim made only in prose — the same seam `src/console/repair/dispatch.rs`
//! established for the Repair router (issue #736).
//!
//! The router never inspects who sent a command. Every helm command it applies
//! (`process_helm_inputs`) has already been validated
//! by the command-admission layer — human tokens at the network gate, the
//! per-axis helm AI's emissions through the same `validate_and_admit` seam — and
//! placed into each ship's own `AdmittedCommands`. This function declares which
//! per-axis System kinds that router consumes. `process_helm_inputs` resolves
//! each kind to the owning ship's authored System id, so alternate ship-specific
//! ids remain inside the same consumer address domain.

/// Register the six per-axis Helm System kinds as admitted-command consumers
/// (issue #833): `process_helm_inputs` applies all six in one applier — the
/// four per-axis helm ids plus vertical thrust and, since issue #881,
/// `helm-boost` (which the retired LocalShip-only `handle_boost_messages` used
/// to own). Called from `ShipPlugin::build`.
///
/// The applier systems themselves are scheduled in `src/ship_plugin.rs`
/// (in `SimSet::Physics`, downstream of `command_admission`'s `AdmissionSet`),
/// so an admitted command is always fully populated before an applier reads it.
pub fn register_helm_dispatch(app: &mut bevy::prelude::App) {
    use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};

    app.register_admitted_consumer(ConsumerMatcher::kind(
        crate::ship::system_registry::HELM_THRUST_KIND,
    ))
    .register_admitted_consumer(ConsumerMatcher::kind(
        crate::ship::system_registry::HELM_STEERING_KIND,
    ))
    .register_admitted_consumer(ConsumerMatcher::kind(
        crate::ship::system_registry::HELM_IMPULSE_KIND,
    ))
    .register_admitted_consumer(ConsumerMatcher::kind(
        crate::ship::system_registry::LATERAL_THRUST_KIND,
    ))
    .register_admitted_consumer(ConsumerMatcher::kind(
        crate::ship::system_registry::VERTICAL_THRUST_KIND,
    ))
    .register_admitted_consumer(ConsumerMatcher::kind(
        crate::ship::system_registry::HELM_BOOST_KIND,
    ));
}
