use super::*;

fn station(id: &str) -> StationId {
    StationId(id.to_owned())
}
fn monitor(id: &str) -> MonitorIdentity {
    MonitorIdentity::new(id)
}
fn layout() -> BridgeLayout {
    BridgeLayout::new(
        [monitor("view"), monitor("left"), monitor("right")],
        [station("helm"), station("science")],
        &monitor("view"),
    )
    .unwrap()
}

#[derive(Default)]
struct AssignmentHost {
    assignments: ConsoleAssignments,
    claims: PendingConsoleClaims,
    bus: PaneBus,
    sessions: SessionManager,
}

impl AssignmentHost {
    fn press(&mut self, layout: &BridgeLayout, action: LayoutAction) -> (BridgeLayout, bool) {
        apply_host_action(
            layout,
            &action,
            Some(&mut self.assignments),
            Some(&mut self.claims),
            Some(&self.bus),
            Some(&mut self.sessions),
        )
        .unwrap()
    }

    fn assign(&mut self, layout: &BridgeLayout, name: &str, screen: &str) -> BridgeLayout {
        self.press(
            layout,
            LayoutAction::AssignStation {
                station: station(name),
                monitor: monitor(screen),
            },
        )
        .0
    }

    fn app(self, layout: BridgeLayout) -> App {
        let mut app = App::new();
        app.insert_resource(self.assignments);
        app.insert_resource(self.claims);
        app.insert_resource(PaneBusResource(self.bus));
        app.insert_resource(Sessions(self.sessions));
        app.insert_resource(BridgeLayoutResource {
            layout,
            monitors: vec![],
            notices: vec![],
        });
        app.add_message::<InboundMessage>();
        app.add_systems(
            Update,
            (
                remember_console_assignments,
                apply_pending_console_claims,
                sync_console_assignments,
            )
                .chain(),
        );
        app
    }
}

fn claims(app: &mut App) -> Vec<(String, String)> {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .drain()
        .filter_map(|message| match message.msg {
            ClientMessage::SelectStation { station } => Some((message.token, station)),
            _ => None,
        })
        .collect()
}

#[test]
fn host_assignment_reserves_before_open_and_keeps_identity_until_off() {
    let mut host = AssignmentHost::default();
    let assigned = host.assign(&layout(), "helm", "left");
    let token = host.bus.console_assignments()[0].0.clone();
    assert!(!host
        .sessions
        .station_claim_allowed("phone", &station("helm")));
    assert!(host.bus.open_pane_for_name("helm").is_none());
    let (first, _) = open_console(&host.bus, Some(&mut host.claims), &station("helm"));
    host.sessions
        .register(token.clone(), "Helm".into())
        .unwrap();
    host.sessions.set_station(&token, Some(station("helm")));
    close_console(&host.bus, Some(&mut host.claims), &station("helm"));
    host.sessions.disconnect(&token);
    assert_eq!(
        host.assignments.monitor_for(&station("helm")),
        Some(&monitor("left"))
    );
    assert!(!host
        .sessions
        .station_claim_allowed("phone", &station("helm")));
    let moved = host.assign(&assigned, "helm", "right");
    let (second, _) = open_console(&host.bus, Some(&mut host.claims), &station("helm"));
    assert_ne!(first, second);
    assert_eq!(host.bus.token_of(second), Some(token.clone()));
    let (_, released) = host.press(
        &moved,
        LayoutAction::UnassignStation {
            station: station("helm"),
        },
    );
    assert!(released);
    assert!(host.assignments.is_empty());
    assert!(host.bus.console_assignments().is_empty());
    assert!(host
        .sessions
        .station_claim_allowed("phone", &station("helm")));
    assert!(host.bus.recreate(second).is_none());
    host.sessions.reconnect(&token).unwrap();
    host.sessions.set_station(&token, None);
    let mut app = host.app(layout());
    app.update();
    assert!(claims(&mut app).is_empty());
}

#[test]
fn a_connected_phone_refusal_changes_no_assignment_state() {
    let mut host = AssignmentHost::default();
    host.sessions
        .register("phone".into(), "Ada".into())
        .unwrap();
    host.sessions.set_station("phone", Some(station("helm")));
    let result = apply_host_action(
        &layout(),
        &LayoutAction::AssignStation {
            station: station("helm"),
            monitor: monitor("left"),
        },
        Some(&mut host.assignments),
        Some(&mut host.claims),
        Some(&host.bus),
        Some(&mut host.sessions),
    );
    assert!(
        matches!(*result.unwrap_err(), LayoutNotice::StationHeld { holder, .. } if holder == "Ada")
    );
    assert!(host.assignments.is_empty());
    assert!(host.bus.console_assignments().is_empty());
    assert_eq!(
        host.sessions.station_for_token("phone"),
        Some(&station("helm"))
    );
}

#[test]
fn selected_class_releases_old_intent_and_reserves_missing_screen_before_open() {
    let mut host = AssignmentHost::default();
    host.assign(&layout(), "helm", "left");
    let (old, _) = open_console(&host.bus, Some(&mut host.claims), &station("helm"));
    let old_token = host.bus.token_of(old).unwrap();
    let desired = HashMap::from([(station("science"), monitor("absent"))]);
    adopt_assignments(
        layout().roster(),
        desired,
        Some(&mut host.assignments),
        Some(&mut host.claims),
        Some(&host.bus),
        Some(&mut host.sessions),
    );
    assert!(host.bus.open_pane_for_name("helm").is_none());
    assert!(host.bus.recreate(old).is_none());
    assert!(host.sessions.native_station_for_token(&old_token).is_none());
    assert_eq!(
        host.assignments.monitor_for(&station("science")),
        Some(&monitor("absent"))
    );
    assert!(!host
        .sessions
        .station_claim_allowed("phone", &station("science")));
    assert!(host.bus.open_pane_for_name("science").is_none());
    host.sessions
        .register(old_token, "Late helm".into())
        .unwrap();
    let mut app = host.app(layout());
    app.update();
    assert!(claims(&mut app).is_empty());
}

#[test]
fn initial_claim_and_return_to_lobby_repair_share_one_emission_rule() {
    let mut host = AssignmentHost::default();
    let layout = host.assign(&layout(), "helm", "left");
    let (pane, _) = open_console(&host.bus, Some(&mut host.claims), &station("helm"));
    let token = host.bus.token_of(pane).unwrap();
    host.sessions
        .register(token.clone(), "Helm".into())
        .unwrap();
    let mut app = host.app(layout);
    app.update();
    assert_eq!(claims(&mut app), vec![(token.clone(), "helm".into())]);
    app.world_mut()
        .resource_mut::<Sessions>()
        .0
        .set_station(&token, Some(station("helm")));
    app.update();
    assert!(claims(&mut app).is_empty());
    app.world_mut()
        .resource_mut::<Sessions>()
        .0
        .clear_all_stations();
    app.update();
    assert_eq!(claims(&mut app), vec![(token, "helm".into())]);
}

#[test]
fn initial_claim_still_sends_once_when_tenure_already_matches() {
    let mut host = AssignmentHost::default();
    let layout = host.assign(&layout(), "helm", "left");
    let (pane, _) = open_console(&host.bus, Some(&mut host.claims), &station("helm"));
    let token = host.bus.token_of(pane).unwrap();
    host.sessions
        .register(token.clone(), "Helm".into())
        .unwrap();
    host.sessions.set_station(&token, Some(station("helm")));
    let mut app = host.app(layout);
    app.update();
    assert_eq!(claims(&mut app), vec![(token, "helm".into())]);
    app.update();
    assert!(claims(&mut app).is_empty());
}

#[test]
fn registered_pending_console_removed_from_roster_emits_no_stale_claim() {
    let mut host = AssignmentHost::default();
    host.assign(&layout(), "helm", "left");
    let (pane, _) = open_console(&host.bus, Some(&mut host.claims), &station("helm"));
    let token = host.bus.token_of(pane).unwrap();
    host.sessions
        .register(token.clone(), "Helm".into())
        .unwrap();
    let reduced =
        BridgeLayout::new([monitor("view")], [station("science")], &monitor("view")).unwrap();
    let mut app = host.app(reduced);
    app.update();
    assert!(
        claims(&mut app).is_empty(),
        "a removed Station must not claim before pruning"
    );
    assert!(app
        .world()
        .resource::<PaneBusResource>()
        .0
        .console_assignments()
        .is_empty());
    assert!(app.world().resource::<ConsoleAssignments>().is_empty());
    app.update();
    assert!(claims(&mut app).is_empty());
}

#[derive(Resource, Default)]
struct AssignmentChanges(Vec<bool>);
fn record_assignment_change(
    assignments: Res<ConsoleAssignments>,
    mut changes: ResMut<AssignmentChanges>,
) {
    changes.0.push(assignments.is_changed());
}

#[test]
fn waiting_for_identify_never_marks_desired_assignments_changed() {
    let mut host = AssignmentHost::default();
    let layout = host.assign(&layout(), "helm", "left");
    open_console(&host.bus, Some(&mut host.claims), &station("helm"));
    let mut app = host.app(layout);
    app.init_resource::<AssignmentChanges>();
    app.add_systems(
        Update,
        record_assignment_change.after(sync_console_assignments),
    );
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(
        app.world().resource::<AssignmentChanges>().0,
        vec![true, false, false, false]
    );
    assert!(claims(&mut app).is_empty());
}

#[test]
fn initial_claim_accepts_identify_before_600_waited_frames_and_expires_afterward() {
    for waited_frames in [599, 600] {
        let bus = PaneBus::default();
        let mut pending = PendingConsoleClaims::default();
        let (pane, _) = open_console(&bus, Some(&mut pending), &station("helm"));
        let token = bus.token_of(pane).unwrap();
        let mut app = App::new();
        app.insert_resource(pending);
        app.insert_resource(Sessions(SessionManager::new()));
        app.add_message::<InboundMessage>();
        // Exercise the initial policy alone: return-to-lobby repair deliberately
        // remains able to restore an assigned, connected Session after expiry.
        app.add_systems(Update, apply_pending_console_claims);
        for _ in 0..waited_frames {
            app.update();
        }
        assert!(claims(&mut app).is_empty());
        app.world_mut()
            .resource_mut::<Sessions>()
            .0
            .register(token.clone(), "Delayed helm".into())
            .unwrap();
        app.update();
        let expected = if waited_frames == 599 {
            vec![(token, "helm".into())]
        } else {
            vec![]
        };
        assert_eq!(
            claims(&mut app),
            expected,
            "Identify after {waited_frames} frames"
        );
        app.update();
        assert!(claims(&mut app).is_empty());
    }
}
