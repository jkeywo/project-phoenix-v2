//! Which observer a disposable Test is drawing for.
//!
//! PRESENTATION ONLY, and that is the whole design. Switching view must not
//! restart the run, move its clock, or change a single thing the fixed tick
//! reads — an author switching to the Game Master view to see why a hull turned
//! must be looking at the same run, at the same tick, that they were watching a
//! moment ago.
//!
//! Two levers, both of which the game already treats as presentation:
//!
//! * `NativeGmPresentation` turns the ordinary GM projections on and off. It is
//!   declared `StateClass::Presentation` and a native host already inserts and
//!   removes it every frame beside a running ship, so a Test doing the same
//!   changes its boot identity no more than that host does.
//! * `LocalShip` says which hull this machine draws. Nothing it gates may reach
//!   the authoritative digest — `tests/local_ship_neutrality.rs` runs the same
//!   seeded world twice, moving only that marker, and compares the digest on
//!   every tick.
//!
//! So a Test observes through the ordinary builders rather than a Workshop-only
//! substitute, and adds no command route, credential or save path to do it.
use super::test_protocol::{TestShip, TestView};
use crate::gm_projection::NativeGmPresentation;
use crate::lockstep::FleetSlotOf;
use crate::server_app::LocalShip;
use bevy::prelude::*;

/// The view a Test is drawing, and the ships it could draw instead.
#[derive(Resource, Default)]
pub struct TestViewState {
    /// What the operator last asked for.
    pub requested: TestView,
    /// Every simulated player ship, for the selector to offer.
    ///
    /// A Test today runs one player ship, because the disposable boot keeps the
    /// default solo roster. This reads whatever the run actually has rather
    /// than assuming that, so a multi-ship Test needs no change here.
    pub ships: Vec<TestShip>,
}

pub(crate) fn install(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<TestViewState>(
        StateClass::Presentation,
        "gm-milestone-integrated-workshop",
    )
    .init_resource::<TestViewState>()
    .add_systems(Update, apply_test_view);
}

/// Bring the world into line with the requested view.
///
/// Runs every frame rather than on change: the ship list is discovered from the
/// world, and a hull that spawns after the view was chosen should appear in the
/// selector without the operator asking again.
fn apply_test_view(
    mut commands: Commands,
    mut state: ResMut<TestViewState>,
    gm: Option<Res<NativeGmPresentation>>,
    ships: Query<
        (
            Entity,
            &crate::entities::spawner::EntityUuid,
            Option<&crate::entities::spawner::EntityName>,
        ),
        With<FleetSlotOf>,
    >,
    local: Query<Entity, With<LocalShip>>,
) {
    let mut listed: Vec<(Entity, TestShip)> = ships
        .iter()
        .map(|(entity, uuid, name)| {
            (
                entity,
                TestShip {
                    entity: uuid.0.clone(),
                    name: name
                        .map(|name| name.0.clone())
                        .unwrap_or_else(|| uuid.0.clone()),
                },
            )
        })
        .collect();
    // Stable order, so the selector does not reshuffle under the operator's
    // cursor when an unrelated entity spawns.
    listed.sort_by(|a, b| a.1.entity.cmp(&b.1.entity));
    let next: Vec<TestShip> = listed.iter().map(|(_, ship)| ship.clone()).collect();
    if state.ships != next {
        state.ships = next;
    }

    match &state.requested {
        TestView::GameMaster => {
            if gm.is_none() {
                commands.insert_resource(NativeGmPresentation);
            }
        }
        TestView::Ship { entity } => {
            if gm.is_some() {
                commands.remove_resource::<NativeGmPresentation>();
            }
            // `None` means the ship the Test launched with, which already
            // carries the marker: leave it exactly where it is rather than
            // moving it to whatever happens to sort first.
            let Some(requested) = entity.as_deref() else {
                return;
            };
            let Some((target, _)) = listed.iter().find(|(_, ship)| ship.entity == requested) else {
                return;
            };
            let current: Vec<Entity> = local.iter().collect();
            if current.len() == 1 && current[0] == *target {
                return;
            }
            for entity in current {
                commands.entity(entity).remove::<LocalShip>();
            }
            commands.entity(*target).insert(LocalShip);
        }
    }
}
