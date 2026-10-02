use super::*;

fn table() -> JoinCodeTable {
    JoinCodeTable::read(std::path::Path::new("assets/join/join-codes.toml"))
        .expect("the authored table is checked in")
}

#[test]
fn the_shipped_table_is_the_one_this_build_reads() {
    // The whole justification for a third reader: it must load the file the
    // designer edits, not a copy. A `format_version` bump lands here first.
    let t = table();
    assert_eq!(t.version().len(), 36, "an authored release GUID");
    assert_eq!(t.project_for(NAMESPACE_CLIENT).map(str::len), Some(36));
    assert_eq!(t.project_for(NAMESPACE_SERVER).map(str::len), Some(36));
    assert!(t.limits.max_relay_frame_bytes >= 65536);
}

#[test]
fn a_table_from_another_revision_is_refused_rather_than_half_read() {
    let bad = "format_version = 99\n[suffix]\nlength = 5\nalphabet = \"AB\"\n\
                   [namespaces]\nclient = \"c\"\n[version]\nguid = \"v\"\n";
    assert!(JoinCodeTable::parse(bad).is_err());
}

#[test]
fn canonicalisation_folds_exactly_what_the_client_folds() {
    // The confusables that make a code typeable off a viewscreen: `0`→`O`,
    // `1`→`I`, `L`→`I`, with `J` deliberately untouched, and the punctuation
    // a player adds while reading it aloud dropped.
    let t = table();
    assert_eq!(t.canonicalise("qu-ark"), "QUARK");
    assert_eq!(t.canonicalise("he110"), "HEIIO");
    assert_eq!(t.canonicalise("j o k e r"), "JOKER");
    assert_eq!(t.canonicalise("QU_ARK"), "QUARK");
}

#[test]
fn a_denied_word_is_refused_in_every_confusable_spelling() {
    // Containment, because the suffix is longer than the words on the list:
    // a code is refused for READING as one of them wherever it does.
    let t = table();
    assert_eq!(t.validate_suffix("ADMINXYZ"), Err("denied"));
    assert_eq!(t.validate_suffix("XYZADMIN"), Err("denied"));
    assert_eq!(t.validate_suffix("XYADM1NZ"), Err("denied"));
    assert_eq!(t.validate_suffix(""), Err("empty"));
    assert_eq!(t.validate_suffix("ABC"), Err("length"));
    assert_eq!(t.validate_suffix("ABCDEFG$"), Err("charset"));
    assert_eq!(t.validate_suffix("quarking"), Ok("QUARKING".to_string()));
}

#[test]
fn a_minted_code_is_one_the_clients_own_parser_would_accept() {
    // Composition is `PROJECT_VERSION_SUFFIX` and nothing else; the parts
    // are the authored GUIDs. Asserted here because a code that does not
    // round-trip through `parseJoinCode` is a code nobody can use.
    let t = table();
    let mut n = 0;
    let code = t
        .mint_client_code(|len| {
            n += 1;
            n % len
        })
        .expect("a code is mintable");
    assert_eq!(
        code.suffix.chars().count(),
        8,
        "the authored length, and 25^8 ≈ 1.5e11 is the whole defence"
    );
    assert_eq!(code.namespace, NAMESPACE_CLIENT);
    assert_eq!(
        code.full,
        format!("{}_{}_{}", code.project, code.version, code.suffix)
    );
    assert!(
        !t.reads_as_denied(&code.suffix),
        "never reads as a denied word"
    );
}

#[test]
fn the_mint_never_draws_a_denied_word_even_when_the_draw_insists() {
    // A `draw` pinned to a whole suffix that READS as a deny-listed word
    // must not produce it — `mintSuffix`'s `continue`, and the reason the
    // ceiling exists. The scripted word is exactly one suffix long, so
    // every one of the 64 attempts draws the same denied spelling rather
    // than a rotation of it that happens to be clean.
    let t = table();
    let denied: Vec<char> = "ADMINXYZ".chars().collect();
    assert_eq!(
        denied.len(),
        t.suffix_length,
        "one whole suffix per attempt"
    );
    let mut i = 0;
    let minted = t.mint_suffix(|_| {
        let ch = denied[i % denied.len()];
        i += 1;
        t.alphabet.iter().position(|c| *c == ch).unwrap()
    });
    assert!(
        minted.is_none(),
        "64 denied draws produce no code, not ADMINXYZ"
    );
}

fn record(t: &JoinCodeTable) -> crate::core::rendezvous::JoinCode {
    crate::core::rendezvous::JoinCode {
        full: t.compose(
            t.project_for(NAMESPACE_CLIENT).unwrap(),
            t.version(),
            "QUARKING",
        ),
        suffix: "QUARKING".to_string(),
        project: t.project_for(NAMESPACE_CLIENT).unwrap().to_string(),
        version: t.version().to_string(),
        namespace: NAMESPACE_CLIENT.to_string(),
    }
}

#[test]
fn the_full_code_a_qr_carries_resolves_and_so_does_the_bare_suffix() {
    // The two routes into the same record: a scanned QR sends the whole
    // structured code, and a guest reading the viewscreen types the
    // letters. Both must land on the same answer or half the room cannot
    // join.
    let t = table();
    let rec = record(&t);
    assert_eq!(t.resolve(&rec.full, NAMESPACE_CLIENT, &rec), Ok(()));
    assert_eq!(t.resolve("QUARKING", NAMESPACE_CLIENT, &rec), Ok(()));
    assert_eq!(t.resolve("qu-ark ing", NAMESPACE_CLIENT, &rec), Ok(()));
    assert_eq!(
        t.resolve(
            &format!("http://host/client/index.html#{}", rec.full),
            NAMESPACE_CLIENT,
            &rec
        ),
        Ok(()),
        "a whole scanned URL carries its code in the fragment"
    );
}

#[test]
fn the_three_typed_refusals_stay_three_answers() {
    // What the registry's `lookup` keeps apart, kept apart here: they are
    // three different sentences on a phone, and collapsing them sends a
    // guest back to re-type a code that was already right.
    let t = table();
    let rec = record(&t);
    assert_eq!(
        t.resolve("XYZABCDE", NAMESPACE_CLIENT, &rec),
        Err("unknown")
    );
    let other_release = t.compose(
        &rec.project,
        "00000000-0000-4000-8000-000000000000",
        "QUARKING",
    );
    assert_eq!(
        t.resolve(&other_release, NAMESPACE_CLIENT, &rec),
        Err("version-mismatch")
    );
    let fleet = t.compose(
        t.project_for(NAMESPACE_SERVER).unwrap(),
        t.version(),
        "QUARKING",
    );
    assert_eq!(t.resolve(&fleet, NAMESPACE_CLIENT, &rec), Err("wrong-type"));
    // A phone asking in the fleet namespace for this crew record is the
    // same refusal from the other direction (issue #1114's parameter).
    assert_eq!(
        t.resolve(&rec.full, NAMESPACE_SERVER, &rec),
        Err("wrong-type")
    );
}

#[test]
fn a_code_that_is_not_a_code_is_refused_before_any_lookup() {
    let t = table();
    let rec = record(&t);
    assert_eq!(t.resolve("", NAMESPACE_CLIENT, &rec), Err("empty"));
    assert_eq!(t.resolve("ABC", NAMESPACE_CLIENT, &rec), Err("length"));
    assert_eq!(
        t.resolve(
            &"A".repeat(t.limits.max_code_length + 1),
            NAMESPACE_CLIENT,
            &rec
        ),
        Err("malformed"),
        "a bounded identifier, bounded before it is parsed"
    );
    // A stale `#<32 hex peer id>` bookmark from the PeerJS era.
    assert_eq!(
        t.resolve("0123456789abcdef0123456789abcdef", NAMESPACE_CLIENT, &rec),
        Err("length")
    );
    // Two GUID-shaped parts and no third is a paste that went wrong, not a
    // spaced-out suffix.
    assert_eq!(
        t.resolve(
            &format!("{}_{}", rec.project, rec.version),
            NAMESPACE_CLIENT,
            &rec
        ),
        Err("malformed")
    );
}

#[test]
fn a_head_is_told_apart_from_a_typed_suffix() {
    assert!(is_code_head("2f6b0a11-9c4e-4d7a-8f31-5b90c2d47e18"));
    assert!(is_code_head("release-1"));
    assert!(!is_code_head("QU"));
    assert!(!is_code_head("ARK"));
    assert!(!is_code_head(""));
    assert!(!is_code_head("has space"));
    // The coupling to the authored length: a WHOLE suffix must never read
    // as an identifier head, or `QUARKING_` would be reported malformed
    // instead of resolving. Raising `suffix.length` past this is what this
    // assertion is here to catch.
    let t = table();
    assert!(!is_code_head(&"A".repeat(t.suffix_length)));
}
