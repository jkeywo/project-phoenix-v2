// Shared sibling script unit for the script-in-layers tests (issue #1045).
//
// Named by BOTH `layer_shared_x.toml` and `layer_shared_y.toml`, which is the
// only way two worlds can arrive carrying the same authored script path: an
// inline `[script]` block lifts to a virtual path prefixed with its own world
// file, so it can never collide.
//
// One authored key means one resident AST, while each naming layer receives its
// own origin-scoped registration. `ast_owners` keeps the unit resident until the
// last owner unloads; trigger/dialogue ownership is never inferred from this
// shared path.
//
// The counter is an `increment` rather than an absolute write precisely so a
// double registration shows up as a 2 instead of hiding behind an idempotent 1.

on_world_loaded("shared_arrived");

fn shared_arrived(ctx) {
    ctx.flags.increment("shared_arrivals", 1);
}

// Used by the unload test: if a dialogue from one owner survived after that
// owner left, the other owner's retained AST would still make this executable.
fn shared_accept(ctx) {
    ctx.effects.complete_objective("stale_shared_dialogue");
}
