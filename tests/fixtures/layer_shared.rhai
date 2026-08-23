// Shared sibling script unit for the script-in-layers tests (issue #1045).
//
// Named by BOTH `layer_shared_x.toml` and `layer_shared_y.toml`, which is the
// only way two worlds can arrive carrying the same authored script path: an
// inline `[script]` block lifts to a virtual path prefixed with its own world
// file, so it can never collide.
//
// One authored key means one resident unit, and running a unit's top level is
// what BUILDS its registrations — so the second world to name this file arrives
// carrying a duplicate of the registration below. `merge_layer_scripts` keeps
// the resident unit and registers only what the load actually inserted; without
// that filter this `on_world_loaded` would exist twice, fire twice on the tick
// the second layer landed, and never be retracted (the retraction keys on the
// units a layer OWNS, which the sharer has none of), so a load/unload cycle
// would grow the table without bound.
//
// The counter is an `increment` rather than an absolute write precisely so a
// double registration shows up as a 2 instead of hiding behind an idempotent 1.

on_world_loaded("shared_arrived");

fn shared_arrived(ctx) {
    ctx.flags.increment("shared_arrivals", 1);
}
