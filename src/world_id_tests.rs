use super::*;

#[test]
fn namespace_codes_round_trip_and_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for (i, ns) in IdNamespace::ALL.into_iter().enumerate() {
        assert!(seen.insert(ns.code()), "duplicate code {}", ns.code());
        assert_eq!(IdNamespace::from_code(ns.code()), Some(ns));
        // Dense-discriminant invariant: the codes are the `ALL` index, not
        // merely unique. `headless::digest` and the rendering both rely on
        // this staying true.
        assert_eq!(
            ns.code() as usize,
            i,
            "discriminants must be dense and match the ALL index"
        );
    }
    assert_eq!(IdNamespace::from_code(200), None);
}

/// The declared sequence is the fold order. Pinned by value, not by
/// position, so appending a variant is free and inserting one fails here.
#[test]
fn namespace_discriminants_are_pinned() {
    assert_eq!(IdNamespace::Entity.code(), 0);
    assert_eq!(IdNamespace::Asteroid.code(), 1);
    assert_eq!(IdNamespace::Message.code(), 2);
    assert_eq!(IdNamespace::Projectile.code(), 3);
}

/// The rendering is a real, parseable uuid — which is the entire reason it
/// is shaped this way. See the module docs for the two consumers that
/// depend on it.
#[test]
fn a_minted_id_is_a_valid_uuid() {
    let rendered = WorldId::new(IdNamespace::Entity, 12_345, 7).render();
    let parsed = uuid::Uuid::parse_str(&rendered).expect("a mint must be a valid uuid");
    assert_eq!(parsed.get_version_num(), 8, "RFC 9562 version 8, 'custom'");
    assert_eq!(parsed.to_string(), rendered, "canonical, lowercase form");
}

#[test]
fn render_round_trips_through_parse() {
    for id in [
        WorldId::new(IdNamespace::Entity, 0, 0),
        WorldId::new(IdNamespace::Asteroid, 1, 1),
        WorldId::new(IdNamespace::Message, 12_345, 7),
        WorldId::new(IdNamespace::Projectile, u64::MAX, SEQ_LIMIT - 1),
    ] {
        assert_eq!(WorldId::parse(&id.render()), Some(id), "{id:?}");
    }
}

/// The property the fixed-width hex layout exists for: lexicographic order
/// over the rendered strings agrees with numeric order over the tuples —
/// across namespaces as well as within one, because the namespace occupies
/// the leading bits and hex `0`–`9` sort before `a`–`f`.
#[test]
fn padded_render_sorts_like_the_structured_tuple() {
    let ids = [
        WorldId::new(IdNamespace::Entity, 2, 1),
        WorldId::new(IdNamespace::Entity, 10, 1),
        WorldId::new(IdNamespace::Entity, 10, 2),
        WorldId::new(IdNamespace::Entity, 1, SEQ_LIMIT - 1),
        WorldId::new(IdNamespace::Asteroid, 1, 0),
        WorldId::new(IdNamespace::Projectile, 0, 0),
    ];
    let mut by_tuple = ids.to_vec();
    by_tuple.sort();
    let mut by_string: Vec<WorldId> = ids.to_vec();
    by_string.sort_by_key(|id| id.render());
    assert_eq!(by_tuple, by_string);
    // And the naive readable render is what that protects against.
    assert!("10-1" < "2-1");
}

/// Everything that is NOT a mint must fail to parse, especially the two
/// populations that share the world with them.
#[test]
fn parse_rejects_uuids_that_are_not_mints() {
    // A v4 uuid — every asteroid, every session token.
    assert_eq!(
        WorldId::parse("a1b2c3d4-0000-4000-8000-000000000001"),
        None,
        "a v4 uuid must not be read as a mint"
    );
    // Right shape, unknown namespace: guessing would fold it in the wrong
    // group, so it is refused rather than defaulted.
    // (the namespace byte is the top of the payload, i.e. hex digits 4-5)
    assert_eq!(WorldId::parse("0000ff00-0000-8000-8000-000000000000"), None);
    // Not uuids at all.
    assert_eq!(WorldId::parse("ent-000000000001-000001"), None);
    assert_eq!(WorldId::parse("2-1"), None);
    assert_eq!(WorldId::parse("_self"), None);
    assert_eq!(WorldId::parse(""), None);
}

/// The asteroid population, asserted against the real function rather than
/// a hand-written literal.
#[test]
fn a_cell_derived_asteroid_id_is_not_a_mint() {
    // Same construction `deterministic_cell_uuid` uses: v4 from bytes.
    let rock = uuid::Builder::from_random_bytes([7u8; 16])
        .into_uuid()
        .to_string();
    assert_eq!(WorldId::parse(&rock), None);
}

#[test]
fn sequence_is_per_namespace_and_resets_on_a_new_tick() {
    let mint = WorldIdMint::default();
    mint.begin_tick(7);
    assert_eq!(
        mint.mint(IdNamespace::Entity),
        WorldId::new(IdNamespace::Entity, 7, 0)
    );
    assert_eq!(
        mint.mint(IdNamespace::Entity),
        WorldId::new(IdNamespace::Entity, 7, 1)
    );
    // A different namespace counts separately.
    assert_eq!(
        mint.mint(IdNamespace::Message),
        WorldId::new(IdNamespace::Message, 7, 0)
    );
    // Re-syncing the same tick must not rewind.
    mint.begin_tick(7);
    assert_eq!(
        mint.mint(IdNamespace::Entity),
        WorldId::new(IdNamespace::Entity, 7, 2)
    );
    mint.begin_tick(8);
    assert_eq!(
        mint.mint(IdNamespace::Entity),
        WorldId::new(IdNamespace::Entity, 8, 0)
    );
}

/// AC5 in miniature: two mints driven through the same tick/spawn sequence
/// produce identical ids, with no shared state between them.
#[test]
fn two_independent_mints_agree_on_the_same_schedule() {
    let script = [
        (0u64, IdNamespace::Entity),
        (0, IdNamespace::Entity),
        (0, IdNamespace::Message),
        (1, IdNamespace::Entity),
        (1, IdNamespace::Projectile),
        (4, IdNamespace::Entity),
    ];
    let run = || {
        let mint = WorldIdMint::default();
        script
            .iter()
            .map(|(tick, ns)| {
                mint.begin_tick(*tick);
                mint.mint(*ns).render()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
    // Pinned by value: these strings are a recorded run's identities, and
    // changing the layout silently re-labels every one of them.
    assert_eq!(
        run(),
        vec![
            "00000000-0000-8000-8000-000000000000",
            "00000000-0000-8000-8000-000000000001",
            "00000200-0000-8000-8000-000000000000",
            "00000000-0000-8000-8000-000100000000",
            "00000300-0000-8000-8000-000100000000",
            "00000000-0000-8000-8000-000400000000",
        ]
    );
}

/// Ids minted off the fixed schedule (a `Startup` spawn, a frame-driven
/// system) keep the last-synced tick and continue its sequence, so they
/// cannot collide with the ids that tick already issued.
#[test]
fn off_tick_mints_do_not_collide_with_the_tick_they_ride_on() {
    let mint = WorldIdMint::default();
    mint.begin_tick(3);
    let during = mint.mint(IdNamespace::Entity);
    let after = mint.mint(IdNamespace::Entity); // no begin_tick: a frame system
    assert_ne!(during, after);
    assert_eq!(after, WorldId::new(IdNamespace::Entity, 3, 1));
}

#[test]
fn absent_resource_still_mints_unique_valid_ids() {
    let a = mint_id_with(None, IdNamespace::Entity);
    let b = mint_id_with(None, IdNamespace::Entity);
    assert_ne!(a, b);
    assert!(WorldId::parse(&a).is_some());
    assert!(uuid::Uuid::parse_str(&b).is_ok());
}

// --- serde round-trips (issue #862) -------------------------------------
//
// Round-tripped through RON specifically, because that is the text format
// `vellum-save`'s browser backend stores strings as — a value that only
// round-trips through `serde_json` or in-memory would prove nothing about
// the snapshot path these types actually travel.

#[test]
fn world_id_round_trips_through_ron() {
    let id = WorldId::new(IdNamespace::Projectile, 12_345, 7);
    let text = ron::to_string(&id).expect("WorldId should serialise");
    let restored: WorldId = ron::from_str(&text).expect("WorldId should parse");
    assert_eq!(restored, id);
}

/// The property #862 is waiting on: a mint's tick and every namespace's
/// sequence counter survive a trip out of the process and back, restored
/// exactly rather than reset to zero. Each namespace is minted a
/// *different* number of times first, so a restore that merely re-synced
/// the tick (without carrying the per-namespace counters) cannot pass by
/// accident.
#[test]
fn mint_state_round_trips_through_ron_and_restores_counters_exactly() {
    let live = WorldIdMint::default();
    live.begin_tick(9);
    for (i, ns) in IdNamespace::ALL.iter().enumerate() {
        for _ in 0..=i {
            live.mint(*ns);
        }
    }
    // Snapshot the per-namespace counts BEFORE the reference draws below,
    // so `expected_counts` is what a restore should report and
    // `expected_next` is what the live mint goes on to produce *from
    // those captured counters*.
    let expected_counts: Vec<u64> = IdNamespace::ALL
        .iter()
        .map(|ns| live.minted_so_far(*ns))
        .collect();
    let text = ron::to_string(&live.state()).expect("WorldIdMintState should serialise");
    let expected_next: Vec<WorldId> = IdNamespace::ALL.iter().map(|ns| live.mint(*ns)).collect();

    let restored =
        WorldIdMint::from_state(ron::from_str(&text).expect("WorldIdMintState should parse"));

    assert_eq!(restored.tick(), 9, "the tick round-trips");
    for (ns, expected_count) in IdNamespace::ALL.iter().zip(expected_counts.iter()) {
        assert_eq!(
            restored.minted_so_far(*ns),
            *expected_count,
            "restored counters must match the live mint at capture time for {ns:?}"
        );
    }

    // And the restored mint continues each namespace's sequence from
    // exactly where the live one was captured, not from zero.
    let resumed: Vec<WorldId> = IdNamespace::ALL
        .iter()
        .map(|ns| restored.mint(*ns))
        .collect();
    assert_eq!(resumed, expected_next);
}

/// The Bevy wiring: `FixedFirst` sync means a system in the step sees the
/// index of the step it is running in.
#[test]
fn sync_system_adopts_the_current_tick() {
    let mut app = App::new();
    app.init_resource::<crate::sim_tick::SimTick>()
        .init_resource::<WorldIdMint>()
        .add_systems(Update, sync_world_id_mint);
    app.update();
    assert_eq!(app.world().resource::<WorldIdMint>().tick(), 0);

    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 42;
    app.update();
    assert_eq!(app.world().resource::<WorldIdMint>().tick(), 42);
    assert_eq!(
        app.world()
            .resource::<WorldIdMint>()
            .mint(IdNamespace::Entity),
        WorldId::new(IdNamespace::Entity, 42, 0)
    );
}
