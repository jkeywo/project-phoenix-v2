use super::*;

fn world(kind: &str) -> AuthoredDirective {
    AuthoredDirective::new(Some(kind.to_string()))
}

#[test]
fn catalogue_names_are_unique_and_round_trip() {
    let mut names = std::collections::BTreeSet::new();
    for kind in DirectiveKind::ALL {
        assert!(names.insert(kind.name()));
        assert_eq!(DirectiveKind::parse(Some(kind.name())), Ok(kind));
    }
    assert_eq!(DirectiveKind::parse(None), Ok(DirectiveKind::None));
}

#[test]
fn missing_empty_cross_kind_and_unknown_are_typed_rejections() {
    assert_eq!(
        interpret(&world("Reach")),
        Err(DirectiveError::MissingField {
            kind: DirectiveKind::Reach,
            slot: DirectiveSlot::Anchor,
        })
    );

    let mut empty = world("Reach");
    empty.push_text(DirectiveField::Anchor, Some("  ".into()));
    assert_eq!(
        interpret(&empty),
        Err(DirectiveError::EmptyField {
            kind: DirectiveKind::Reach,
            slot: DirectiveSlot::Anchor,
        })
    );

    let mut cross_kind = world("Reach");
    cross_kind.push_texts(DirectiveField::PatrolAnchors, vec!["alpha".into()]);
    assert_eq!(
        interpret(&cross_kind),
        Err(DirectiveError::ForbiddenField {
            kind: DirectiveKind::Reach,
            kind_authored: true,
            field: DirectiveField::PatrolAnchors,
        })
    );

    assert_eq!(
        interpret(&world("Wander")),
        Err(DirectiveError::UnknownKind("Wander".into()))
    );

    let mut unknown = world("Patrol");
    unknown.push_unknown_fields(["directive_waypoints".into()]);
    assert_eq!(
        interpret(&unknown),
        Err(DirectiveError::UnknownField("directive_waypoints".into()))
    );
}

#[test]
fn nonempty_text_lists_reject_blank_elements_without_removing_empty_patrol_hold() {
    let mut blank_anchor = world("Patrol");
    blank_anchor.push_texts(
        DirectiveField::PatrolAnchors,
        vec!["alpha".into(), " \t ".into()],
    );
    assert_eq!(
        interpret(&blank_anchor),
        Err(DirectiveError::EmptyField {
            kind: DirectiveKind::Patrol,
            slot: DirectiveSlot::Anchors,
        })
    );

    let mut empty_hold = world("Patrol");
    empty_hold.push_texts(DirectiveField::PatrolAnchors, Vec::new());
    assert_eq!(
        interpret(&empty_hold),
        Ok(AiDirective::Patrol {
            anchors: Vec::new(),
            loop_path: false,
        })
    );
}

#[test]
fn established_untargeted_destroy_and_empty_patrol_defaults_are_preserved() {
    assert_eq!(
        interpret(&world("Destroy")),
        Ok(AiDirective::Destroy {
            target: String::new()
        })
    );
    assert_eq!(
        interpret(&world("Patrol")),
        Ok(AiDirective::Patrol {
            anchors: vec![],
            loop_path: false,
        })
    );
}

#[test]
fn diagnostics_name_only_fields_available_on_the_originating_surface() {
    let mut hail = world("Hail");
    hail.push_text(DirectiveField::Anchor, Some("wrong".into()));
    let error = interpret(&hail).expect_err("Hail cannot author an anchor");

    let world_message = error.describe(DirectiveSurface::World);
    assert!(world_message.contains("which reads `target`"));
    assert!(!world_message.contains("directive_hail_target"));

    let doctrine_message = error.describe(DirectiveSurface::Doctrine);
    assert!(doctrine_message.contains("which reads `directive_hail_target`"));
    assert!(!doctrine_message.contains("which reads `target`"));
}

#[test]
fn diagnostics_distinguish_absent_kind_from_explicit_none() {
    let mut absent = AuthoredDirective::new(None);
    absent.push_text(DirectiveField::Anchor, Some("wrong".into()));
    let absent_message = interpret(&absent)
        .expect_err("an anchor without a kind is forbidden")
        .describe(DirectiveSurface::World);
    assert!(absent_message.contains("no directive_kind is authored"));

    let mut explicit_none = world("None");
    explicit_none.push_text(DirectiveField::Anchor, Some("wrong".into()));
    let explicit_message = interpret(&explicit_none)
        .expect_err("None reads no directive field")
        .describe(DirectiveSurface::World);
    assert!(explicit_message.contains("directive_kind = \"None\""));
    assert!(explicit_message.contains("which reads no directive field"));
    assert!(!explicit_message.contains("no directive_kind is authored"));
}
