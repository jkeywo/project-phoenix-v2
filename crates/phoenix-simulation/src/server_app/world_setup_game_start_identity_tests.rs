use super::{
    resume_game_start_row_inclusion, select_game_start_entity_uuid, ResumeGameStartEntityUuids,
};

#[test]
fn non_resume_keeps_the_normal_mint_value_unchanged() {
    let minted = "00000000-0000-8000-8000-000000000009".to_string();
    assert_eq!(
        select_game_start_entity_uuid(None, 7, None, minted.clone()),
        minted
    );
}

#[test]
fn a_named_game_start_row_uses_its_registered_world_identity() {
    let registered = "00000000-0000-8000-8000-000000000007".to_string();
    assert_eq!(
        select_game_start_entity_uuid(None, 7, Some(&registered), "unused-mint".into()),
        registered,
        "the name-to-UUID map and spawned entity must describe the same contact"
    );
}

#[test]
fn authored_indexes_restore_player_and_npc_rows_without_predicate_shifting() {
    let saved = ResumeGameStartEntityUuids(vec![
        crate::snapshot::GameStartEntityUuid {
            authored_index: 3,
            entity_uuid: "00000000-0000-8000-8000-000000000003".into(),
        },
        crate::snapshot::GameStartEntityUuid {
            authored_index: 8,
            entity_uuid: "00000000-0000-8000-8000-000000000008".into(),
        },
    ]);
    let registered_player = "registered-player".to_string();

    assert_eq!(
        select_game_start_entity_uuid(
            Some(&saved),
            3,
            Some(&registered_player),
            "fresh-player".into(),
        ),
        "00000000-0000-8000-8000-000000000003"
    );
    assert_eq!(
        select_game_start_entity_uuid(Some(&saved), 5, None, "fresh-skipped".into()),
        "fresh-skipped",
        "a predicate-skipped authored row cannot shift the later mapping"
    );
    assert_eq!(
        select_game_start_entity_uuid(Some(&saved), 8, None, "fresh-npc".into()),
        "00000000-0000-8000-8000-000000000008",
        "GameStart NPCs take their own saved identity too"
    );
    assert_eq!(resume_game_start_row_inclusion(Some(&saved), 3), Some(true));
    assert_eq!(
        resume_game_start_row_inclusion(Some(&saved), 5),
        Some(false)
    );
    assert_eq!(resume_game_start_row_inclusion(Some(&saved), 8), Some(true));
    assert_eq!(
        resume_game_start_row_inclusion(None, 5),
        None,
        "a normal boot still decides row inclusion from its authored predicate"
    );
}
