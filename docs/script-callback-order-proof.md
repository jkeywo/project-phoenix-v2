# Script callback order across a resume (#1400)

`tests/snapshot_resume.rs` runs the existing pending-callback guard and
`same_tick_ordered_script_callbacks_survive_a_resume` under both the ordinary
pinned test configuration and the fresh-process default-pool parent. The shared
`default_pool` observer requires two actual Apps for each guard, more than one
compute worker and the ordinary `MultiThreaded` executor in each default child.
It checks the actual mission seed, completed tick and authoritative digest.

The new test-only world, `tests/fixtures/worlds/scripted_order_resume.toml`, uses
the shipped cruiser unchanged and the ordinary 60 Hz simulation. Two timer
handlers schedule distinct callbacks for the same future tick. The first writes
an order counter of 1; the second multiplies it by 10, adds 2 and declares victory
only for 12. Reversing their effects produces defeat and a final counter of 1.
This observes order through a real mission outcome.

The capture follows both timers but precedes their callbacks. The guard requires
two distinct saved identities, a shared future due tick and no callbacks in the
fresh bootstrap. It compares the complete restored queue, including identity,
order, owner and due tick, then the full authoritative digest at restore and on
every one of 220 subsequent ticks. Both callbacks must fire at the saved tick,
each once, and both timer latches must remain spent. Every observed tick checks
the order counter, call counts, queue size and terminal outcome; observation
continues after firing to catch a replay.

Targeted validation commands, run sequentially by the integration owner:

```powershell
cargo test --features headless --test snapshot_resume -- a_pending_script_callback_survives_a_resume_and_fires_on_its_own_tick --exact --nocapture --test-threads=1
cargo test --features headless --test snapshot_resume -- same_tick_ordered_script_callbacks_survive_a_resume --exact --nocapture --test-threads=1
cargo test --features headless --test snapshot_resume -- default_pool_preserves_snapshot_resume_guards --exact --nocapture --test-threads=1
```

The default parent retains its duel seed sweep and Combat Test round-trip arms.
The new script arms preserve the existing capture and continuation bounds and
change no production scheduling, digest, identity format or authored game assets.
Source formatting and inspection are complete; native execution on the final
integrated scheduler is pending. This bounded fixture does not claim coverage
of every script effect or of live-event acceptance.
