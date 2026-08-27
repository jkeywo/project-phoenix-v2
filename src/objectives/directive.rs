//! Shared authored-Directive interpretation (issue #1268).
//!
//! Entity doctrine and World `add_objective` actions intentionally keep their
//! established, different TOML field names. Their adapters reduce those fields
//! to [`AuthoredDirective`]; this module is the only place that knows the kind
//! vocabulary, field ownership, required fields, defaults, and conversion to
//! the runtime [`AiDirective`].

use crate::core::messages::AiDirective;
use std::fmt;

/// Every authored Directive kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectiveKind {
    None,
    Patrol,
    Destroy,
    Reach,
    Retreat,
    Hail,
    Scan,
    Dock,
    Tow,
    Stabilise,
    Escort,
    Transfer,
    FieldRepair,
    Order,
}

impl DirectiveKind {
    pub const ALL: [Self; 14] = [
        Self::None,
        Self::Patrol,
        Self::Destroy,
        Self::Reach,
        Self::Retreat,
        Self::Hail,
        Self::Scan,
        Self::Dock,
        Self::Tow,
        Self::Stabilise,
        Self::Escort,
        Self::Transfer,
        Self::FieldRepair,
        Self::Order,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Patrol => "Patrol",
            Self::Destroy => "Destroy",
            Self::Reach => "Reach",
            Self::Retreat => "Retreat",
            Self::Hail => "Hail",
            Self::Scan => "Scan",
            Self::Dock => "Dock",
            Self::Tow => "Tow",
            Self::Stabilise => "Stabilise",
            Self::Escort => "Escort",
            Self::Transfer => "Transfer",
            Self::FieldRepair => "FieldRepair",
            Self::Order => "Order",
        }
    }

    fn parse(authored: Option<&str>) -> Result<Self, DirectiveError> {
        let Some(authored) = authored else {
            return Ok(Self::None);
        };
        Self::ALL
            .into_iter()
            .find(|kind| kind.name() == authored)
            .ok_or_else(|| DirectiveError::UnknownKind(authored.to_string()))
    }

    const fn requires(self, slot: DirectiveSlot) -> bool {
        match self {
            // An untargeted Destroy is the shipped standing combat doctrine:
            // the target selector resolves any visible hostile. Patrol's empty
            // route is likewise an established resolved-hold default.
            Self::None | Self::Patrol | Self::Destroy => false,
            Self::Reach | Self::Retreat => matches!(slot, DirectiveSlot::Anchor),
            Self::Order => matches!(slot, DirectiveSlot::Target | DirectiveSlot::Route),
            Self::Hail
            | Self::Scan
            | Self::Dock
            | Self::Tow
            | Self::Stabilise
            | Self::Escort
            | Self::Transfer
            | Self::FieldRepair => matches!(slot, DirectiveSlot::Target),
        }
    }
}

impl fmt::Display for DirectiveKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Which unchanged TOML authoring surface supplied a Directive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectiveSurface {
    Doctrine,
    World,
}

/// Canonical value slots consumed by a runtime [`AiDirective`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectiveSlot {
    Anchors,
    Loop,
    Target,
    Anchor,
    Route,
}

impl DirectiveSlot {
    const ALL: [Self; 5] = [
        Self::Anchors,
        Self::Loop,
        Self::Target,
        Self::Anchor,
        Self::Route,
    ];
}

/// One existing TOML field and the semantic role it has on its authoring
/// surface. Doctrine's dedicated target fields remain distinct here, so the
/// common validator—not an adapter—owns their forbidden-kind rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectiveField {
    PatrolAnchors,
    PatrolLoop,
    Anchor,
    WorldTarget,
    DoctrineDestroyTarget,
    DoctrineHailTarget,
    DoctrineScanTarget,
    DoctrineDockTarget,
    DoctrineOperateTarget,
    DoctrineOrderTarget,
    WorldRoute,
    DoctrineOrderRoute,
}

impl DirectiveField {
    const ALL: [Self; 12] = [
        Self::PatrolAnchors,
        Self::PatrolLoop,
        Self::Anchor,
        Self::WorldTarget,
        Self::DoctrineDestroyTarget,
        Self::DoctrineHailTarget,
        Self::DoctrineScanTarget,
        Self::DoctrineDockTarget,
        Self::DoctrineOperateTarget,
        Self::DoctrineOrderTarget,
        Self::WorldRoute,
        Self::DoctrineOrderRoute,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::PatrolAnchors => "directive_anchors",
            Self::PatrolLoop => "directive_loop",
            Self::Anchor => "directive_anchor",
            Self::WorldTarget => "target",
            Self::DoctrineDestroyTarget => "directive_target",
            Self::DoctrineHailTarget => "directive_hail_target",
            Self::DoctrineScanTarget => "directive_scan_target",
            Self::DoctrineDockTarget => "directive_dock_target",
            Self::DoctrineOperateTarget => "directive_operate_target",
            Self::DoctrineOrderTarget => "directive_order_target",
            Self::WorldRoute => "route",
            Self::DoctrineOrderRoute => "directive_order_route",
        }
    }

    const fn slot(self) -> DirectiveSlot {
        match self {
            Self::PatrolAnchors => DirectiveSlot::Anchors,
            Self::PatrolLoop => DirectiveSlot::Loop,
            Self::Anchor => DirectiveSlot::Anchor,
            Self::WorldTarget
            | Self::DoctrineDestroyTarget
            | Self::DoctrineHailTarget
            | Self::DoctrineScanTarget
            | Self::DoctrineDockTarget
            | Self::DoctrineOperateTarget
            | Self::DoctrineOrderTarget => DirectiveSlot::Target,
            Self::WorldRoute | Self::DoctrineOrderRoute => DirectiveSlot::Route,
        }
    }

    const fn allows(self, kind: DirectiveKind) -> bool {
        match self {
            Self::PatrolAnchors | Self::PatrolLoop => matches!(kind, DirectiveKind::Patrol),
            Self::Anchor => matches!(kind, DirectiveKind::Reach | DirectiveKind::Retreat),
            Self::WorldTarget => matches!(
                kind,
                DirectiveKind::Destroy
                    | DirectiveKind::Hail
                    | DirectiveKind::Scan
                    | DirectiveKind::Dock
                    | DirectiveKind::Tow
                    | DirectiveKind::Stabilise
                    | DirectiveKind::Escort
                    | DirectiveKind::Transfer
                    | DirectiveKind::FieldRepair
                    | DirectiveKind::Order
            ),
            Self::DoctrineDestroyTarget => matches!(kind, DirectiveKind::Destroy),
            Self::DoctrineHailTarget => matches!(kind, DirectiveKind::Hail),
            Self::DoctrineScanTarget => matches!(kind, DirectiveKind::Scan),
            Self::DoctrineDockTarget => matches!(kind, DirectiveKind::Dock),
            Self::DoctrineOperateTarget => matches!(
                kind,
                DirectiveKind::Tow
                    | DirectiveKind::Stabilise
                    | DirectiveKind::Escort
                    | DirectiveKind::Transfer
                    | DirectiveKind::FieldRepair
            ),
            Self::DoctrineOrderTarget | Self::WorldRoute | Self::DoctrineOrderRoute => {
                matches!(kind, DirectiveKind::Order)
            }
        }
    }

    const fn appears_on(self, surface: DirectiveSurface) -> bool {
        match self {
            Self::PatrolAnchors | Self::PatrolLoop | Self::Anchor => true,
            Self::WorldTarget | Self::WorldRoute => matches!(surface, DirectiveSurface::World),
            Self::DoctrineDestroyTarget
            | Self::DoctrineHailTarget
            | Self::DoctrineScanTarget
            | Self::DoctrineDockTarget
            | Self::DoctrineOperateTarget
            | Self::DoctrineOrderTarget
            | Self::DoctrineOrderRoute => matches!(surface, DirectiveSurface::Doctrine),
        }
    }

    fn owners(self) -> String {
        DirectiveKind::ALL
            .into_iter()
            .filter(|kind| self.allows(*kind))
            .map(DirectiveKind::name)
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DirectiveValue {
    Text(String),
    Texts(Vec<String>),
    Bool(bool),
}

impl DirectiveValue {
    fn contains_blank_text(&self) -> bool {
        match self {
            Self::Text(value) => value.trim().is_empty(),
            Self::Texts(values) => values.iter().any(|value| value.trim().is_empty()),
            Self::Bool(_) => false,
        }
    }
}

/// One field that was materially authored. Default/empty collection values are
/// omitted by adapters because doctrine's serde-defaulted representation cannot
/// distinguish them from omission; authored strings and non-empty string lists
/// remain present so the shared interpreter can reject blank values and blank
/// list elements consistently.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredDirectiveField {
    pub field: DirectiveField,
    pub value: DirectiveValue,
}

impl AuthoredDirectiveField {
    pub fn text(field: DirectiveField, value: String) -> Self {
        Self {
            field,
            value: DirectiveValue::Text(value),
        }
    }

    pub fn texts(field: DirectiveField, value: Vec<String>) -> Self {
        Self {
            field,
            value: DirectiveValue::Texts(value),
        }
    }

    pub fn boolean(field: DirectiveField, value: bool) -> Self {
        Self {
            field,
            value: DirectiveValue::Bool(value),
        }
    }
}

/// Canonical raw Directive assembled by an authoring-surface adapter.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredDirective {
    pub kind: Option<String>,
    pub fields: Vec<AuthoredDirectiveField>,
    /// Field names the originating surface did not recognise. Values are
    /// deliberately discarded: an unknown key has no typed semantic slot, so
    /// its spelling is the complete diagnostic payload.
    pub unknown_fields: Vec<String>,
}

impl AuthoredDirective {
    pub fn new(kind: Option<String>) -> Self {
        Self {
            kind,
            fields: Vec::new(),
            unknown_fields: Vec::new(),
        }
    }

    pub fn push_text(&mut self, field: DirectiveField, value: Option<String>) {
        if let Some(value) = value {
            self.fields.push(AuthoredDirectiveField::text(field, value));
        }
    }

    pub fn push_texts(&mut self, field: DirectiveField, value: Vec<String>) {
        if !value.is_empty() {
            self.fields
                .push(AuthoredDirectiveField::texts(field, value));
        }
    }

    pub fn push_bool(&mut self, field: DirectiveField, value: bool) {
        if value {
            self.fields
                .push(AuthoredDirectiveField::boolean(field, value));
        }
    }

    pub fn push_unknown_fields(&mut self, fields: impl IntoIterator<Item = String>) {
        self.unknown_fields.extend(fields);
        self.unknown_fields.sort();
        self.unknown_fields.dedup();
    }
}

/// Stable rejection categories shared by doctrine and World adapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectiveError {
    UnknownKind(String),
    UnknownField(String),
    ForbiddenField {
        kind: DirectiveKind,
        kind_authored: bool,
        field: DirectiveField,
    },
    MissingField {
        kind: DirectiveKind,
        slot: DirectiveSlot,
    },
    EmptyField {
        kind: DirectiveKind,
        slot: DirectiveSlot,
    },
    DuplicateField {
        kind: DirectiveKind,
        slot: DirectiveSlot,
    },
}

impl DirectiveError {
    /// Render an error using the unchanged spelling on the originating surface.
    pub fn describe(&self, surface: DirectiveSurface) -> String {
        match self {
            Self::UnknownKind(kind) => format!(
                "unknown directive_kind '{kind}'; valid: {}",
                DirectiveKind::ALL
                    .into_iter()
                    .map(DirectiveKind::name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::UnknownField(field) => format!(
                "unknown Directive field `{field}`; remove it or use one of the fields owned by the selected directive_kind"
            ),
            Self::ForbiddenField {
                kind,
                kind_authored,
                field,
            } => {
                let reads = DirectiveField::ALL
                    .into_iter()
                    .filter(|candidate| candidate.appears_on(surface) && candidate.allows(*kind))
                    .map(|candidate| format!("`{}`", candidate.name()))
                    .collect::<Vec<_>>();
                if *kind == DirectiveKind::None && !*kind_authored {
                    format!(
                        "`{}` is read only for a {} directive, but no directive_kind is \
                         authored, so nothing reads it.",
                        field.name(),
                        field.owners(),
                    )
                } else {
                    format!(
                        "`{}` is read only for a {} directive, but directive_kind = \"{kind}\", \
                         which reads {}. A field belonging to another directive kind is silently \
                         ignored, so it is rejected here instead.",
                        field.name(),
                        field.owners(),
                        if reads.is_empty() {
                            "no directive field".to_string()
                        } else {
                            reads.join(" / ")
                        }
                    )
                }
            }
            Self::MissingField { kind, slot } => format!(
                "directive_kind = \"{kind}\" requires a non-empty `{}`",
                expected_field(surface, *kind, *slot).name()
            ),
            Self::EmptyField { kind, slot } => format!(
                "directive_kind = \"{kind}\" requires a non-empty `{}`",
                expected_field(surface, *kind, *slot).name()
            ),
            Self::DuplicateField { kind, slot } => format!(
                "directive_kind = \"{kind}\" supplies more than one field for its {slot:?} value"
            ),
        }
    }
}

#[derive(Default)]
struct Values {
    anchors: Option<Vec<String>>,
    loop_path: Option<bool>,
    target: Option<String>,
    anchor: Option<String>,
    route: Option<String>,
}

impl Values {
    fn contains(&self, slot: DirectiveSlot) -> bool {
        match slot {
            DirectiveSlot::Anchors => self.anchors.is_some(),
            DirectiveSlot::Loop => self.loop_path.is_some(),
            DirectiveSlot::Target => self.target.is_some(),
            DirectiveSlot::Anchor => self.anchor.is_some(),
            DirectiveSlot::Route => self.route.is_some(),
        }
    }

    fn insert(
        &mut self,
        kind: DirectiveKind,
        authored: &AuthoredDirectiveField,
    ) -> Result<(), DirectiveError> {
        let slot = authored.field.slot();
        if self.contains(slot) {
            return Err(DirectiveError::DuplicateField { kind, slot });
        }
        match (slot, &authored.value) {
            (DirectiveSlot::Anchors, DirectiveValue::Texts(value)) => {
                self.anchors = Some(value.clone())
            }
            (DirectiveSlot::Loop, DirectiveValue::Bool(value)) => self.loop_path = Some(*value),
            (DirectiveSlot::Target, DirectiveValue::Text(value)) => {
                self.target = Some(value.clone())
            }
            (DirectiveSlot::Anchor, DirectiveValue::Text(value)) => {
                self.anchor = Some(value.clone())
            }
            (DirectiveSlot::Route, DirectiveValue::Text(value)) => self.route = Some(value.clone()),
            _ => unreachable!("authoring adapter supplied the wrong value type for {slot:?}"),
        }
        Ok(())
    }

    fn empty(&self, slot: DirectiveSlot) -> bool {
        match slot {
            DirectiveSlot::Anchors => self.anchors.as_ref().is_none_or(Vec::is_empty),
            DirectiveSlot::Loop => false,
            DirectiveSlot::Target => self.target.as_deref().is_none_or(|v| v.trim().is_empty()),
            DirectiveSlot::Anchor => self.anchor.as_deref().is_none_or(|v| v.trim().is_empty()),
            DirectiveSlot::Route => self.route.as_deref().is_none_or(|v| v.trim().is_empty()),
        }
    }
}

fn expected_field(
    surface: DirectiveSurface,
    kind: DirectiveKind,
    slot: DirectiveSlot,
) -> DirectiveField {
    match (surface, slot) {
        (_, DirectiveSlot::Anchors) => DirectiveField::PatrolAnchors,
        (_, DirectiveSlot::Loop) => DirectiveField::PatrolLoop,
        (_, DirectiveSlot::Anchor) => DirectiveField::Anchor,
        (DirectiveSurface::World, DirectiveSlot::Target) => DirectiveField::WorldTarget,
        (DirectiveSurface::World, DirectiveSlot::Route) => DirectiveField::WorldRoute,
        (DirectiveSurface::Doctrine, DirectiveSlot::Route) => DirectiveField::DoctrineOrderRoute,
        (DirectiveSurface::Doctrine, DirectiveSlot::Target) => match kind {
            DirectiveKind::Destroy => DirectiveField::DoctrineDestroyTarget,
            DirectiveKind::Hail => DirectiveField::DoctrineHailTarget,
            DirectiveKind::Scan => DirectiveField::DoctrineScanTarget,
            DirectiveKind::Dock => DirectiveField::DoctrineDockTarget,
            DirectiveKind::Tow
            | DirectiveKind::Stabilise
            | DirectiveKind::Escort
            | DirectiveKind::Transfer
            | DirectiveKind::FieldRepair => DirectiveField::DoctrineOperateTarget,
            DirectiveKind::Order => DirectiveField::DoctrineOrderTarget,
            _ => DirectiveField::DoctrineDestroyTarget,
        },
    }
}

/// Validate and convert one canonical authored Directive.
pub fn interpret(raw: &AuthoredDirective) -> Result<AiDirective, DirectiveError> {
    let kind = DirectiveKind::parse(raw.kind.as_deref())?;
    let mut values = Values::default();

    if let Some(field) = raw.unknown_fields.first() {
        return Err(DirectiveError::UnknownField(field.clone()));
    }

    // Name the cross-kind mistake before a consequent missing-field error.
    for authored in &raw.fields {
        if !authored.field.allows(kind) {
            return Err(DirectiveError::ForbiddenField {
                kind,
                kind_authored: raw.kind.is_some(),
                field: authored.field,
            });
        }
        if authored.value.contains_blank_text() {
            return Err(DirectiveError::EmptyField {
                kind,
                slot: authored.field.slot(),
            });
        }
        values.insert(kind, authored)?;
    }

    for slot in DirectiveSlot::ALL {
        if !kind.requires(slot) {
            continue;
        }
        if !values.contains(slot) {
            return Err(DirectiveError::MissingField { kind, slot });
        }
        if values.empty(slot) {
            return Err(DirectiveError::EmptyField { kind, slot });
        }
    }

    Ok(match kind {
        DirectiveKind::None => AiDirective::None,
        DirectiveKind::Patrol => AiDirective::Patrol {
            anchors: values.anchors.unwrap_or_default(),
            loop_path: values.loop_path.unwrap_or(false),
        },
        DirectiveKind::Destroy => AiDirective::Destroy {
            target: values.target.unwrap_or_default(),
        },
        DirectiveKind::Reach => AiDirective::Reach {
            anchor: values.anchor.expect("required above"),
        },
        DirectiveKind::Retreat => AiDirective::Retreat {
            anchor: values.anchor.expect("required above"),
        },
        DirectiveKind::Hail => AiDirective::Hail {
            target: values.target.expect("required above"),
        },
        DirectiveKind::Scan => AiDirective::Scan {
            target: values.target.expect("required above"),
        },
        DirectiveKind::Dock => AiDirective::Dock {
            target: values.target.expect("required above"),
        },
        DirectiveKind::Tow => AiDirective::Tow {
            target: values.target.expect("required above"),
        },
        DirectiveKind::Stabilise => AiDirective::Stabilise {
            target: values.target.expect("required above"),
        },
        DirectiveKind::Escort => AiDirective::Escort {
            target: values.target.expect("required above"),
        },
        DirectiveKind::Transfer => AiDirective::Transfer {
            target: values.target.expect("required above"),
        },
        DirectiveKind::FieldRepair => AiDirective::FieldRepair {
            target: values.target.expect("required above"),
        },
        DirectiveKind::Order => AiDirective::Order {
            target: values.target.expect("required above"),
            route: values.route.expect("required above"),
        },
    })
}

#[cfg(test)]
mod tests {
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
}
