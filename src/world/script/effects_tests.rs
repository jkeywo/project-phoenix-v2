use super::*;
use crate::world::script::engine::runtime_engine;
use crate::world::script::flags::Flags;
use rhai::{Dynamic, Map};

#[test]
fn addressed_fractional_modifier_uses_the_ordinary_typed_parser() {
    let effects = run_buffered(
        r#"fn f(ctx) {
            ctx.effects.addressed(#{ type: "apply_modifier", tag: "escort",
                slot: "MaxSpeed", bonus: flt("1.5"), recipient_ship_slots: ["lead"] });
        }"#,
        "f",
    );
    let BufferedEffect::Action(TriggerAction::Addressed { recipients, action }) = &effects[0]
    else {
        panic!("expected addressed ordinary action");
    };
    assert_eq!(
        recipients.selectors,
        vec![crate::objective_instances::RecipientSelector::ShipSlot(
            "lead".into()
        )]
    );
    assert!(matches!(action.as_ref(), TriggerAction::ApplyModifier { bonus, .. } if *bonus == 1.5));
}

/// Build the `#{ effects, flags }` context one call reads. Flags share the one
/// ordered buffer (issue #981) so a flag write lands in `sink` alongside
/// effects; these tests write no flags.
fn make_ctx(sink: &EffectSink) -> Map {
    let mut ctx = Map::new();
    ctx.insert("effects".into(), Dynamic::from(sink.clone()));
    ctx.insert(
        "flags".into(),
        Dynamic::from(Flags::new(
            &crate::world::flags::FlagStore::new(),
            sink.clone(),
        )),
    );
    ctx
}

/// Compile `source` on a runtime engine and call `fn_name`, returning the
/// drained buffer verbatim. A local harness so this module's tests don't
/// depend on `RuntimeHost`'s failure-mode wrapper.
fn run_buffered(source: &str, fn_name: &str) -> Vec<BufferedEffect> {
    let engine = runtime_engine();
    let ast = engine.compile(source).expect("compiles");
    let sink = EffectSink::new();
    let ctx = make_ctx(&sink);
    let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", fn_name, ctx).expect("calls");
    sink.take()
}

/// Like [`run_buffered`] but for the effect-only (`Cmd`) verbs: unwrap each
/// buffered effect to its `ActionCmd`. Panics on a name-resolving `Action`, so
/// a test that uses this on a spawn/objective/faction verb fails loudly.
fn run(source: &str, fn_name: &str) -> Vec<ActionCmd> {
    run_buffered(source, fn_name)
        .into_iter()
        .map(|e| match e {
            BufferedEffect::Cmd(cmd) => cmd,
            BufferedEffect::Action(a) => {
                unreachable!("run(): expected only command effects, got {a:?}")
            }
        })
        .collect()
}

/// Compile and call, returning the drained buffer on success or the call
/// error — for the failure-path tests (a raised host fn discards the call).
fn run_result(
    source: &str,
    fn_name: &str,
) -> Result<Vec<BufferedEffect>, vellum_script::CallError> {
    let engine = runtime_engine();
    let ast = engine.compile(source).expect("compiles");
    let sink = EffectSink::new();
    let ctx = make_ctx(&sink);
    vellum_script::call_fn(&engine, &ast, "t.rhai", fn_name, ctx).map(|_| sink.take())
}

/// Compile and call, returning BOTH drained buffers: the ordered effects and
/// the comms opens (stamped with the unit path, as the host does).
fn run_with_opens(source: &str, fn_name: &str) -> (Vec<BufferedEffect>, Vec<OpenCommsRequest>) {
    let engine = runtime_engine();
    let ast = engine.compile(source).expect("compiles");
    let sink = EffectSink::new();
    let ctx = make_ctx(&sink);
    let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", fn_name, ctx).expect("calls");
    (sink.take(), sink.take_opens("t.rhai"))
}

/// The single `TriggerAction` serde produces for one action table, built
/// independently of this module's map extraction — the independent source of
/// truth for the M6 structural-parity assertions.
///
/// It went through `parse_world` and a `[[trigger]]` wrapper until issue #985
/// deleted that container. The TABLE is what mattered and the table survives:
/// `RawActionEntry` is the same struct the script host populates, and
/// `parse_action_entry` the same shared rule, so this still reaches the
/// parity target by the route that is not the one under test.
fn toml_action(action_body: &str) -> TriggerAction {
    let raw: crate::world::config::RawActionEntry =
        toml::from_str(action_body).expect("the action table parses");
    crate::world::config::parse_action_entry(&raw).expect("the action parses")
}

#[test]
fn complete_objective_drains_to_action_cmd() {
    let cmds = run(
        r#"fn on_x(ctx) { ctx.effects.complete_objective("obj1"); }"#,
        "on_x",
    );
    assert_eq!(
        cmds,
        vec![ActionCmd::CompleteObjective {
            id: "obj1".to_string()
        }]
    );
}

#[test]
fn game_over_emits_reason_then_transition_in_order() {
    let cmds = run(
        r#"fn end(ctx) { ctx.effects.game_over("hull breach"); }"#,
        "end",
    );
    assert_eq!(
        cmds,
        vec![
            ActionCmd::SetGameOverReason {
                reason: "hull breach".to_string(),
                outcome: None,
            },
            ActionCmd::SetNextState {
                phase: crate::core::messages::GamePhase::GameOver,
            },
        ]
    );
}

#[test]
fn multiple_effects_buffer_in_call_order() {
    let cmds = run(
        r#"fn on_x(ctx) {
                ctx.effects.fail_objective("a");
                ctx.effects.reset_trigger("b");
                ctx.effects.unload_world("w.toml");
            }"#,
        "on_x",
    );
    assert_eq!(
        cmds,
        vec![
            ActionCmd::FailObjective {
                id: "a".to_string()
            },
            ActionCmd::ResetTrigger {
                id: "b".to_string()
            },
            ActionCmd::UnloadWorld {
                path: "w.toml".to_string()
            },
        ]
    );
}

#[test]
fn take_empties_the_buffer() {
    let sink = EffectSink::new();
    sink.push(ActionCmd::CompleteObjective {
        id: "x".to_string(),
    });
    assert_eq!(sink.len(), 1);
    let _ = sink.take();
    assert!(sink.is_empty());
}

// ── M6 effects API: the four new verbs (issue #984) ───────────────────────

/// `game_over(reason, outcome)` emits the outcome-declaring pair, and does so
/// identically to the declarative `game_over` action dispatched — the outcome
/// (`victory`) rides through in `Some(_)`, reason first.
#[test]
fn game_over_with_outcome_matches_toml() {
    let cmds = run(
        r#"fn end(ctx) { ctx.effects.game_over("world.win", "victory"); }"#,
        "end",
    );
    assert_eq!(
        cmds,
        vec![
            ActionCmd::SetGameOverReason {
                reason: "world.win".to_string(),
                outcome: Some(crate::core::balance::Outcome::Victory),
            },
            ActionCmd::SetNextState {
                phase: crate::core::messages::GamePhase::GameOver,
            },
        ]
    );
    // Structural parity: the same two commands the TOML `game_over` action
    // dispatches (`game_over` needs no context, so a bare dispatch suffices).
    assert_eq!(
        cmds,
        dispatch_bare(&toml_action(
            "type = \"game_over\"\nmessage = \"world.win\"\noutcome = \"victory\""
        ))
    );
}

/// `defeat` is the other accepted outcome; nothing else is.
#[test]
fn game_over_accepts_defeat() {
    let cmds = run(
        r#"fn end(ctx) { ctx.effects.game_over("world.lose", "defeat"); }"#,
        "end",
    );
    assert_eq!(
        cmds[0],
        ActionCmd::SetGameOverReason {
            reason: "world.lose".to_string(),
            outcome: Some(crate::core::balance::Outcome::Defeat),
        }
    );
}

/// A scripted typo raises (discarding the call's effects, settled decision
/// 10) exactly as a bad declarative `outcome = "…"` fails the world load —
/// both route through `crate::core::balance::Outcome::parse`.
#[test]
fn game_over_rejects_an_unknown_outcome() {
    let err = run_result(
        r#"fn end(ctx) { ctx.effects.game_over("x", "victni"); }"#,
        "end",
    )
    .expect_err("an unknown outcome must raise");
    assert!(err.to_string().contains("outcome"), "{err}");
}

// ── report_row (issue #1344) ─────────────────────────────────────────────

/// The whole row reaches the queue as a RESOLVED command: nothing about a
/// report row needs name resolution, so it buffers as a `Cmd` and not an
/// `Action` — the opposite claim `destroy_entity_buffers_an_action_not_a_
/// resolved_command` makes about its own verb, and for the same reason:
/// which buffer a verb lands in IS its architecture.
#[test]
fn report_row_buffers_the_whole_row_as_a_resolved_command() {
    let cmds = run(
        r#"fn f(ctx) {
                ctx.effects.report_row(#{
                    id: "lyra",
                    heading: "world.x.report.lyra.heading",
                    outcome: "world.x.report.lyra.saved",
                    state: "saved",
                    score: 6,
                });
            }"#,
        "f",
    );
    assert_eq!(
        cmds,
        vec![ActionCmd::SetReportRow(crate::core::report::ReportRow {
            id: "lyra".to_string(),
            heading_id: "world.x.report.lyra.heading".to_string(),
            outcome_id: "world.x.report.lyra.saved".to_string(),
            state: crate::core::report::ReportRowState::Saved,
            score: 6,
        })]
    );
}

/// A negative score is the whole point of a SIGNED diagnostic — Lyra lost is
/// `-6` — so the int route must carry the sign through unchanged.
#[test]
fn report_row_carries_a_negative_score() {
    let cmds = run(
        r#"fn f(ctx) {
                ctx.effects.report_row(#{
                    id: "lyra", heading: "h", outcome: "o", state: "lost", score: -6,
                });
            }"#,
        "f",
    );
    match &cmds[0] {
        ActionCmd::SetReportRow(row) => {
            assert_eq!(row.score, -6);
            assert_eq!(row.state, crate::core::report::ReportRowState::Lost);
        }
        other => panic!("expected SetReportRow, got {other:?}"),
    }
}

/// `score` is the one optional key: a row that is purely a statement of
/// fact should not have to spell out a zero.
#[test]
fn report_row_defaults_its_score_to_zero() {
    let cmds = run(
        r#"fn f(ctx) {
                ctx.effects.report_row(#{
                    id: "records", heading: "h", outcome: "o", state: "neutral",
                });
            }"#,
        "f",
    );
    match &cmds[0] {
        ActionCmd::SetReportRow(row) => assert_eq!(row.score, 0),
        other => panic!("expected SetReportRow, got {other:?}"),
    }
}

/// A bad state word raises at the boundary — discarding the call's effects
/// (settled decision 10) — through the SAME parser the vocabulary defines,
/// exactly as a bad `narrative_outcome` word does.
#[test]
fn report_row_rejects_an_unknown_state() {
    let err = run_result(
        r#"fn f(ctx) {
                ctx.effects.report_row(#{
                    id: "lyra", heading: "h", outcome: "o", state: "rescued",
                });
            }"#,
        "f",
    )
    .expect_err("an unknown state must raise");
    assert!(err.to_string().contains("state"), "{err}");
}

/// Every text key is required, and PRESENT-BUT-EMPTY raises exactly as
/// missing does. A row with no heading or no outcome id renders as a blank
/// line on the crew's screen, which is worse than the call failing loudly —
/// and `heading: ""` is a string, so a presence-only check would have let
/// one through. It would then be dropped by `reportRows`
/// (gui/game-over-view.js) on both player surfaces while still contributing
/// its score to `MissionReport::total`, breaking the #1344 invariant that
/// the total equals the visible rows' hidden scores.
#[test]
fn report_row_requires_every_text_key() {
    for (missing, source) in [
        (
            "id",
            r#"fn f(ctx) { ctx.effects.report_row(#{ heading: "h", outcome: "o", state: "saved" }); }"#,
        ),
        (
            "heading",
            r#"fn f(ctx) { ctx.effects.report_row(#{ id: "r", outcome: "o", state: "saved" }); }"#,
        ),
        (
            "outcome",
            r#"fn f(ctx) { ctx.effects.report_row(#{ id: "r", heading: "h", state: "saved" }); }"#,
        ),
        (
            "state",
            r#"fn f(ctx) { ctx.effects.report_row(#{ id: "r", heading: "h", outcome: "o" }); }"#,
        ),
        // The empty-string arms. Same key, same raise, same message.
        (
            "id",
            r#"fn f(ctx) { ctx.effects.report_row(#{ id: "", heading: "h", outcome: "o", state: "saved" }); }"#,
        ),
        (
            "heading",
            r#"fn f(ctx) { ctx.effects.report_row(#{ id: "r", heading: "", outcome: "o", state: "saved" }); }"#,
        ),
        (
            "outcome",
            r#"fn f(ctx) { ctx.effects.report_row(#{ id: "r", heading: "h", outcome: "", state: "saved" }); }"#,
        ),
        // Whitespace is not content either: " " would render a blank line
        // just as "" does, and the JS filter tests emptiness after nothing.
        (
            "heading",
            r#"fn f(ctx) { ctx.effects.report_row(#{ id: "r", heading: "   ", outcome: "o", state: "saved" }); }"#,
        ),
        (
            "state",
            r#"fn f(ctx) { ctx.effects.report_row(#{ id: "r", heading: "h", outcome: "o", state: "" }); }"#,
        ),
    ] {
        let err = run_result(source, "f")
            .err()
            .unwrap_or_else(|| panic!("a row with a missing or empty `{missing}` must raise"));
        assert!(
            err.to_string().contains(missing),
            "the error must name the offending key `{missing}`: {err}"
        );
    }
}

/// The failure policy, stated where it can fail: a raised `report_row`
/// discards the WHOLE call's buffer, so a half-built row can never reach a
/// player surface beside the effects that were meant to accompany it.
#[test]
fn a_bad_report_row_discards_the_calls_other_effects() {
    let err = run_result(
        r#"fn f(ctx) {
                ctx.effects.narrative_beat("before");
                ctx.effects.report_row(#{ id: "r", heading: "h", outcome: "o", state: "nope" });
            }"#,
        "f",
    )
    .expect_err("an unknown state must raise");
    assert!(err.to_string().contains("state"), "{err}");
}

/// `add_faction_enemy(f, e)` buffers the DECLARATIVE `AddFactionEnemy` (faction
/// names, unresolved) — identical to the TOML action before UUID resolution.
#[test]
fn add_faction_enemy_matches_toml() {
    let effs = run_buffered(
        r#"fn f(ctx) { ctx.effects.add_faction_enemy("Harrow", "Alliance"); }"#,
        "f",
    );
    let toml =
        toml_action("type = \"add_faction_enemy\"\nfaction = \"Harrow\"\nenemy = \"Alliance\"");
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
    assert_eq!(
        effs,
        vec![BufferedEffect::Action(TriggerAction::AddFactionEnemy {
            faction: "Harrow".to_string(),
            enemy: "Alliance".to_string(),
        })]
    );
}

/// `remove_faction_enemy(f, e)` is `add_faction_enemy`'s mirror (issue
/// #1349): the same deferred DECLARATIVE action, the opposite direction, and
/// the opposite direction ONLY — a scenario that ends a hostility must get
/// back exactly the action its TOML twin would have produced, because the
/// applier's target re-validation (issue #710) hangs off that one variant.
#[test]
fn remove_faction_enemy_matches_toml() {
    let effs = run_buffered(
        r#"fn f(ctx) { ctx.effects.remove_faction_enemy("Harrow", "Alliance"); }"#,
        "f",
    );
    let toml =
        toml_action("type = \"remove_faction_enemy\"\nfaction = \"Harrow\"\nenemy = \"Alliance\"");
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
    assert_eq!(
        effs,
        vec![BufferedEffect::Action(TriggerAction::RemoveFactionEnemy {
            faction: "Harrow".to_string(),
            enemy: "Alliance".to_string(),
        })]
    );
}

/// The pair, in one call and in authored order. The buffer is ONE ordered
/// `Vec`, so a scenario that declares a hostility and later ends it applies
/// them in the sequence it wrote — and a direction that had been folded into
/// a single boolean setter could not have been ordered at all.
#[test]
fn the_faction_verbs_keep_their_authored_order() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.add_faction_enemy("Harrow", "Alliance");
                ctx.effects.remove_faction_enemy("Alliance", "Harrow");
            }"#,
        "f",
    );
    assert_eq!(
        effs,
        vec![
            BufferedEffect::Action(TriggerAction::AddFactionEnemy {
                faction: "Harrow".to_string(),
                enemy: "Alliance".to_string(),
            }),
            BufferedEffect::Action(TriggerAction::RemoveFactionEnemy {
                faction: "Alliance".to_string(),
                enemy: "Harrow".to_string(),
            }),
        ]
    );
}

// ── show_message (issue #1342) ────────────────────────────────────────────

#[test]
fn show_message_drains_to_action_cmd() {
    let cmds = run(
        r#"fn on_x(ctx) {
                ctx.effects.show_message("hail_debris", "world.probe.computer_message.text", "advisory", 10);
            }"#,
        "on_x",
    );
    assert_eq!(
        cmds,
        vec![ActionCmd::ShowComputerMessage {
            id: "hail_debris".to_string(),
            text: "world.probe.computer_message.text".to_string(),
            severity: crate::core::computer_message::ComputerMessageSeverity::Advisory,
            duration_secs: 10,
            station: None,
        }]
    );
}

#[test]
fn show_message_accepts_an_optional_station_cue() {
    let cmds = run(
        r#"fn on_x(ctx) {
                ctx.effects.show_message("charge_ready", "world.probe.computer_message.charge", "critical", 8, "tactical");
            }"#,
        "on_x",
    );
    assert_eq!(
        cmds,
        vec![ActionCmd::ShowComputerMessage {
            id: "charge_ready".to_string(),
            text: "world.probe.computer_message.charge".to_string(),
            severity: crate::core::computer_message::ComputerMessageSeverity::Critical,
            duration_secs: 8,
            station: Some(crate::core::messages::StationId("tactical".to_string())),
        }]
    );
}

/// Every severity word parses, case-insensitively — the same boundary
/// `ComputerMessageSeverity::parse` unit-tests directly, proven here
/// through the real host fn.
#[test]
fn show_message_accepts_every_severity() {
    for (word, expected) in [
        (
            "info",
            crate::core::computer_message::ComputerMessageSeverity::Info,
        ),
        (
            "ADVISORY",
            crate::core::computer_message::ComputerMessageSeverity::Advisory,
        ),
        (
            "Warning",
            crate::core::computer_message::ComputerMessageSeverity::Warning,
        ),
        (
            "critical",
            crate::core::computer_message::ComputerMessageSeverity::Critical,
        ),
    ] {
        let cmds = run(
            &format!(
                r#"fn on_x(ctx) {{ ctx.effects.show_message("id", "text.id", "{word}", 5); }}"#
            ),
            "on_x",
        );
        match &cmds[0] {
            ActionCmd::ShowComputerMessage { severity, .. } => {
                assert_eq!(*severity, expected, "word {word:?}")
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

/// An unknown severity raises at the script boundary — discarding the
/// call's effects (settled decision 10) — exactly as `narrative_outcome`'s
/// outcome word does.
#[test]
fn show_message_rejects_an_unknown_severity() {
    let err = run_result(
        r#"fn on_x(ctx) { ctx.effects.show_message("id", "text.id", "urgent", 5); }"#,
        "on_x",
    )
    .expect_err("an unknown severity must raise");
    assert!(err.to_string().contains("severity"), "{err}");
}

#[test]
fn presentation_hosts_use_the_declarative_parser_and_buffer_one_shared_action() {
    let sound: RawActionEntry = toml::from_str("type = 'presentation'\nentity = 'player'\n[presentation.sound]\nid = 'weapons'\nsource = 'raider'\n").unwrap();
    let parsed = parse_action_entry(&sound).unwrap();
    let from_script = run_buffered(
        r#"fn on_x(ctx) { ctx.effects.sound("player", "weapons", "raider"); }"#,
        "on_x",
    );
    assert!(matches!(&from_script[0], BufferedEffect::Action(action) if action == &parsed));
    let raw: RawActionEntry = toml::from_str("type = 'presentation'\nentity = 'player'\n[presentation.title_card]\ntitle = 'Arrival'\nsubtitle = 'Stand by'\nduration_ticks = 10\n").unwrap();
    assert!(matches!(
        parse_action_entry(&raw).unwrap(),
        TriggerAction::Presentation {
            cue: crate::gm_presentation::PresentationCue::TitleCard {
                duration_ticks: 10,
                ..
            },
            ..
        }
    ));
    let effects = run_buffered(
        r#"fn on_x(ctx) { ctx.effects.title_card("player", "Arrival", "Stand by", 10); ctx.effects.force_view("player", "sensors_radar", 20); ctx.effects.incoming_comms("player", "message", 30); ctx.effects.clear_presentation("player", "card"); }"#,
        "on_x",
    );
    assert_eq!(effects.len(), 4);
    assert!(effects.iter().all(|effect| matches!(effect, BufferedEffect::Action(TriggerAction::Presentation { ship, .. }) if ship == "player")));
    assert!(matches!(
        &effects[0],
        BufferedEffect::Action(TriggerAction::Presentation {
            cue: crate::gm_presentation::PresentationCue::TitleCard {
                duration_ticks: 10,
                ..
            },
            ..
        })
    ));
    for source in [
        r#"fn on_x(ctx) { ctx.effects.title_card("player", "Title", "", 0); }"#,
        r#"fn on_x(ctx) { ctx.effects.force_view("player", "invalid", 20); }"#,
        r#"fn on_x(ctx) { ctx.effects.clear_presentation("player", "world"); }"#,
    ] {
        assert!(run_result(source, "on_x").is_err());
    }
}

/// A non-positive duration raises too — "positive simulation-time
/// duration" is AC1's own wording.
#[test]
fn show_message_rejects_a_non_positive_duration() {
    for bad in [0, -5] {
        let err = run_result(
            &format!(
                r#"fn on_x(ctx) {{ ctx.effects.show_message("id", "text.id", "info", {bad}); }}"#
            ),
            "on_x",
        )
        .expect_err("a non-positive duration must raise");
        assert!(err.to_string().contains("duration_secs"), "{err}");
    }
}

/// A validation failure discards the WHOLE call's effects — the buffer
/// carries nothing from an earlier effect in the same handler either
/// (settled decision 10, same contract every other validated verb here
/// holds).
#[test]
fn show_message_failure_discards_earlier_effects_in_the_same_call() {
    let result = run_result(
        r#"fn on_x(ctx) {
                ctx.effects.narrative_beat("before");
                ctx.effects.show_message("id", "text.id", "not-a-severity", 5);
            }"#,
        "on_x",
    );
    assert!(result.is_err());
}

// ── destroy_entity (issue #1033) ─────────────────────────────────────────

/// `destroy_entity(name)` buffers the DECLARATIVE `DestroyEntity` — the entity
/// NAME, unresolved — identical to the TOML action before UUID resolution, the
/// same assertion `add_faction_enemy_matches_toml` makes about its twin.
#[test]
fn destroy_entity_matches_toml() {
    let effs = run_buffered(
        r#"fn f(ctx) { ctx.effects.destroy_entity("skyhook"); }"#,
        "f",
    );
    let toml = toml_action("type = \"destroy_entity\"\nentity = \"skyhook\"");
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
    assert_eq!(
        effs,
        vec![BufferedEffect::Action(TriggerAction::DestroyEntity {
            entity: "skyhook".to_string(),
        })]
    );
}

/// The load-bearing shape claim, stated where it can fail: a destroy buffers as
/// an `Action`, NOT a `Cmd`.
///
/// This is the whole architecture in one assertion. A `Cmd` is applied
/// directly and would despawn the entity while chaining nothing; an `Action` is
/// resolved through `dispatch_destroy_entity`, which pushes
/// `WorldEvent::Destroyed` onto `new_events` beside the command — and that
/// event is what makes `on_destroyed` / `on_all_destroyed` fire off a scripted
/// removal. A refactor that "simplified" this into a resolved command would
/// pass every other test in this module and silently break chaining.
#[test]
fn destroy_entity_buffers_an_action_not_a_resolved_command() {
    let effs = run_buffered(
        r#"fn f(ctx) { ctx.effects.destroy_entity("skyhook"); }"#,
        "f",
    );
    assert!(
        matches!(effs.as_slice(), [BufferedEffect::Action(_)]),
        "a destroy must defer name resolution to dispatch — a resolved Cmd \
             would despawn without chaining a Destroyed event, got {effs:?}"
    );
}

/// A destroy keeps its authored position among the other effects, so a handler
/// that raises a flag, destroys a structure and completes an objective applies
/// them in that order.
#[test]
fn destroy_entity_interleaves_in_authored_order() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.complete_objective("first");
                ctx.effects.destroy_entity("skyhook");
                ctx.effects.fail_objective("last");
            }"#,
        "f",
    );
    assert_eq!(
        effs,
        vec![
            BufferedEffect::Cmd(ActionCmd::CompleteObjective {
                id: "first".to_string()
            }),
            BufferedEffect::Action(TriggerAction::DestroyEntity {
                entity: "skyhook".to_string(),
            }),
            BufferedEffect::Cmd(ActionCmd::FailObjective {
                id: "last".to_string()
            }),
        ]
    );
}

/// `add_objective(#{…})` reads the script map into the SAME `TriggerAction`
/// the combat_test-style declarative action parses — directive (`Destroy`) and
/// utility (`base_priority: 80` INT → `80.0`) included.
#[test]
fn add_objective_matches_toml() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "obj-destroy-wave-1",
                    text: "world.combat_test.trigger.action.obj_destroy_wave_1.text",
                    mandatory: true,
                    targets: ["wave_1"],
                    target: "wave_1",
                    directive_kind: "Destroy",
                    base_priority: 80,
                });
            }"#,
        "f",
    );
    let toml = toml_action(
        "type = \"add_objective\"\n\
             id = \"obj-destroy-wave-1\"\n\
             text = \"world.combat_test.trigger.action.obj_destroy_wave_1.text\"\n\
             mandatory = true\n\
             targets = [\"wave_1\"]\n\
             target = \"wave_1\"\n\
             directive_kind = \"Destroy\"\n\
             base_priority = 80.0",
    );
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
}

#[test]
fn named_objective_instance_rhai_matches_toml_and_addresses_one_instance() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "hold",
                    instance_id: "lead",
                    text: "objective.hold",
                    recipient_ship_slots: ["lead"],
                    recipient_factions: ["alliance"],
                });
            }"#,
        "f",
    );
    let toml = toml_action(
        "type = \"add_objective\"\n\
             id = \"hold\"\n\
             instance_id = \"lead\"\n\
             text = \"objective.hold\"\n\
             recipient_ship_slots = [\"lead\"]\n\
             recipient_factions = [\"alliance\"]",
    );
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);

    let commands = run(
        r#"fn f(ctx) {
                ctx.effects.set_objective_progress("hold", "lead", flt("0.75"));
                ctx.effects.complete_objective("hold", "lead");
                ctx.effects.fail_objective("hold", "other");
            }"#,
        "f",
    );
    assert!(matches!(
        &commands[0],
        ActionCmd::SetObjectiveInstanceProgress { key, progress }
            if key.objective_id == "hold" && key.instance_id == "lead" && *progress == 0.75
    ));
    assert!(matches!(
        &commands[1],
        ActionCmd::CompleteObjectiveInstance { key }
            if key.objective_id == "hold" && key.instance_id == "lead"
    ));
    assert!(matches!(
        &commands[2],
        ActionCmd::FailObjectiveInstance { key }
            if key.objective_id == "hold" && key.instance_id == "other"
    ));
}

/// Issue #1139: the script front-end carries Scan's shared target through
/// the same mission directive parser as declarative TOML.
#[test]
fn add_objective_scan_matches_toml() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "obj-scan-rung",
                    text: "world.objective.scan_rung",
                    target: "Ladder Depot B",
                    directive_kind: "Scan",
                    base_priority: 52,
                });
            }"#,
        "f",
    );
    let toml = toml_action(
        "type = \"add_objective\"\n\
             id = \"obj-scan-rung\"\n\
             text = \"world.objective.scan_rung\"\n\
             target = \"Ladder Depot B\"\n\
             directive_kind = \"Scan\"\n\
             base_priority = 52.0",
    );
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
}

/// The scripted World surface must not lose keys before the same typed
/// Directive contract sees them. Otherwise TOML would reject a typo while
/// the equivalent `#{ ... }` silently accepted it.
#[test]
fn add_objective_rejects_an_unknown_directive_field() {
    let err = run_result(
        r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "unknown-field",
                    text: "Unknown",
                    directive_kind: "Patrol",
                    directive_waypoints: ["alpha"],
                });
            }"#,
        "f",
    )
    .expect_err("an unknown scripted Directive field must raise");
    assert!(
        err.to_string()
            .contains("unknown Directive field `directive_waypoints`"),
        "{err}"
    );
}

/// `add_objective` now READS `modifiers`/`zero_gates` (the utility-config
/// milestone): the script arrays-of-maps, with `flt("…")` fractional thresholds
/// and weights, build the identical `TriggerAction::AddObjective` — same
/// `UtilityConfig` — the declarative twin parses through the shared
/// `parse_utility_config`. This is the parity the FRACTIONAL test-infra worlds
/// ride on: `no_float` Rhai could not author `weight = 1.25` before `flt`.
#[test]
fn add_objective_reads_modifiers_and_zero_gates_matching_toml() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "obj-utility",
                    text: "world.obj.text",
                    base_priority: flt("50.5"),
                    modifiers: [
                        #{ condition: "enemy_near", threshold: flt("0.5"), weight: flt("2.0") },
                        #{ condition: "low_hull", weight: flt("1.25") },
                    ],
                    zero_gates: [
                        #{ condition: "shields_down" },
                        #{ condition: "power_low", threshold: flt("0.2") },
                    ],
                });
            }"#,
        "f",
    );
    let toml = toml_action(
            "type = \"add_objective\"\n\
             id = \"obj-utility\"\n\
             text = \"world.obj.text\"\n\
             base_priority = 50.5\n\
             modifiers = [{ condition = \"enemy_near\", threshold = 0.5, weight = 2.0 }, { condition = \"low_hull\", weight = 1.25 }]\n\
             zero_gates = [{ condition = \"shields_down\" }, { condition = \"power_low\", threshold = 0.2 }]",
        );
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
}

/// A `modifier` missing its required `weight` RAISES (discarding the call,
/// settled decision 10) — the same loud failure a declarative modifier without a
/// `weight` gives, rather than a silently degraded `UtilityConfig`.
#[test]
fn add_objective_rejects_a_modifier_without_a_weight() {
    let err = run_result(
        r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "o", text: "t",
                    modifiers: [#{ condition: "enemy_near" }],
                });
            }"#,
        "f",
    )
    .expect_err("a weightless modifier must raise");
    assert!(err.to_string().contains("weight"), "{err}");
}

/// Issue #1110: a scripted `command_stance` map and the declarative
/// `command_stance` table build the BYTE-IDENTICAL `TriggerAction::AddObjective`
/// — same target Station id, same `StationStanceConfig` — because both run
/// through the one `parse_command_stance` seam. Two front-ends, one parser.
#[test]
fn add_objective_command_stance_matches_toml() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "obj-escort",
                    text: "world.obj.text",
                    command_stance: #{
                        station: "tactical",
                        id: "objective-escort",
                        kind: "standard",
                        high_alert: true,
                        persist_behind_human: true,
                    },
                });
            }"#,
        "f",
    );
    let toml = toml_action(
            "type = \"add_objective\"\n\
             id = \"obj-escort\"\n\
             text = \"world.obj.text\"\n\
             command_stance = { station = \"tactical\", id = \"objective-escort\", kind = \"standard\", high_alert = true, persist_behind_human = true }",
        );
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);
}

/// A `command_stance` with no `station` RAISES (discarding the call, settled
/// decision 10): there is no target Station to lend the stance to. The same
/// loud failure the declarative `parse_command_stance` gives a blank station.
#[test]
fn add_objective_rejects_a_command_stance_without_a_station() {
    let err = run_result(
        r#"fn f(ctx) {
                ctx.effects.add_objective(#{
                    id: "o", text: "t",
                    command_stance: #{ id: "x", kind: "standard" },
                });
            }"#,
        "f",
    )
    .expect_err("a stationless command_stance must raise");
    assert!(err.to_string().contains("station"), "{err}");
}

// ── Feature A: the `flt("…")` fractional-data marker (no_float-safe) ───────

/// The sharpest `flt` parity: a `flt("0.9")` override leaf must produce the
/// IDENTICAL `toml::Value` the declarative `target_speed = 0.9` carries — the
/// byte-identity a converted FRACTIONAL world depends on (`f64::from_str` ≡ toml's
/// float parse for canonical decimals). This is why a fractional constant can be
/// transported through `no_float` script as opaque data with no determinism loss.
#[test]
fn flt_override_leaf_matches_declarative_float() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                    position: [0, 0, 0],
                    overrides: #{
                        helm_console: #{
                            target_speed: flt("0.9"),
                        },
                    },
                });
            }"#,
        "f",
    );
    let toml = toml_action(
        "type = \"spawn_entity\"\n\
             template_path = \"assets/entities/ship.toml\"\n\
             name = \"x\"\n\
             position = [0.0, 0.0, 0.0]\n\
             overrides = { helm_console = { target_speed = 0.9 } }",
    );
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);

    // Pin the conversion concretely: the `flt("0.9")` leaf is the identical toml
    // FLOAT `0.9` — so a regression in the `RealLit` branch is unmistakable.
    let BufferedEffect::Action(TriggerAction::SpawnEntity { overrides, .. }) = &effs[0] else {
        panic!("expected a spawn action, got {:?}", effs[0]);
    };
    let speed = overrides
        .as_ref()
        .and_then(|o| o.get("helm_console"))
        .and_then(|h| h.get("target_speed"))
        .expect("override target_speed present");
    assert_eq!(speed, &toml::Value::Float(0.9));
}

/// The doctrine-ARRAY shape the converted worlds actually author (issue #984
/// review advisory): a `flt` leaf nested Array→Map deep —
/// `overrides.behaviour.doctrine = [ { … target_speed = 0.9 … } ]` — must
/// still equal the declarative twin. `flt_override_leaf_matches_declarative_float`
/// pins Map→Map recursion; this pins the Array→Map→RealLit path the probe/duel
/// spawn overrides (and combat_test's waves) ride through `dynamic_to_toml`.
#[test]
fn flt_inside_a_doctrine_array_override_matches_declarative() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                    position: [0, 0, 0],
                    overrides: #{
                        behaviour: #{
                            doctrine: [
                                #{
                                    id: "kill",
                                    base_priority: 80,
                                    target_speed: flt("0.9"),
                                    maintain_range: 25,
                                },
                            ],
                        },
                    },
                });
            }"#,
        "f",
    );
    let toml = toml_action(
            "type = \"spawn_entity\"\n\
             template_path = \"assets/entities/ship.toml\"\n\
             name = \"x\"\n\
             position = [0.0, 0.0, 0.0]\n\
             overrides = { behaviour = { doctrine = [ { id = \"kill\", base_priority = 80.0, target_speed = 0.9, maintain_range = 25.0 } ] } }",
        );
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);

    // Pin the nested leaf concretely, as the flat-leaf test does.
    let BufferedEffect::Action(TriggerAction::SpawnEntity { overrides, .. }) = &effs[0] else {
        panic!("expected a spawn action, got {:?}", effs[0]);
    };
    let speed = overrides
        .as_ref()
        .and_then(|o| o.get("behaviour"))
        .and_then(|b| b.get("doctrine"))
        .and_then(|d| d.get(0))
        .and_then(|row| row.get("target_speed"))
        .expect("doctrine[0].target_speed present");
    assert_eq!(speed, &toml::Value::Float(0.9));
}

/// An unparseable `flt("…")` RAISES (discarding the call, settled decision 10),
/// exactly as a malformed declarative float fails the world load. The parse
/// happens once, at map-build time, so the raise pre-empts the effect entirely.
#[test]
fn flt_rejects_an_unparseable_string() {
    let err = run_result(
            r#"fn f(ctx) { ctx.effects.add_objective(#{ id: "o", text: "t", base_priority: flt("xyz") }); }"#,
            "f",
        )
        .expect_err("an unparseable flt must raise");
    assert!(err.to_string().contains("flt"), "{err}");
}

// ── Feature B: the `int(…)` integer-target marker (issue #1048) ───────────

/// The mirror of `flt_override_leaf_matches_declarative_float`: an `int(3)`
/// override leaf must produce the IDENTICAL `toml::Value` — a toml INTEGER,
/// not the ambient float default — the declarative
/// `repair.repair_team_count = 3` carries, AND that value must actually
/// deserialize into the genuine `u32` `EntityConfig` field it targets
/// (`entities::config::RepairConfig::repair_team_count`). That last step is
/// the whole point of #1048: before the marker existed, this same leaf
/// rendered as a toml FLOAT and could not deserialize into an integer field
/// at all.
#[test]
fn int_override_leaf_matches_declarative_integer_and_deserializes() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                    position: [0, 0, 0],
                    overrides: #{
                        repair: #{
                            repair_team_count: int(3),
                        },
                    },
                });
            }"#,
        "f",
    );
    let toml = toml_action(
        "type = \"spawn_entity\"\n\
             template_path = \"assets/entities/ship.toml\"\n\
             name = \"x\"\n\
             position = [0.0, 0.0, 0.0]\n\
             overrides = { repair = { repair_team_count = 3 } }",
    );
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);

    // Pin the conversion concretely: an `int(3)` leaf is the toml INTEGER
    // `3`, not the ambient toml FLOAT a bare `3` would render as.
    let BufferedEffect::Action(TriggerAction::SpawnEntity { overrides, .. }) = &effs[0] else {
        panic!("expected a spawn action, got {:?}", effs[0]);
    };
    let count = overrides
        .as_ref()
        .and_then(|o| o.get("repair"))
        .and_then(|r| r.get("repair_team_count"))
        .expect("override repair_team_count present");
    assert_eq!(count, &toml::Value::Integer(3));

    // The crux of #1048: the override actually deserializes into the
    // genuine integer field. Before the fix this `try_into` would fail
    // (`invalid type: floating point`3`, expected u32`).
    let repair: crate::entities::config::RepairConfig = overrides
        .as_ref()
        .and_then(|o| o.get("repair"))
        .cloned()
        .expect("repair override present")
        .try_into()
        .expect("an int(3) leaf must deserialize into RepairConfig's genuine u32 field");
    assert_eq!(repair.repair_team_count, 3);
}

/// The control, mirroring `spawn_entity_override_without_a_tombstone_still_applies`'s
/// role for the tombstone test: an UNMARKED int on the SAME integer-target
/// field still renders as the ambient toml FLOAT, exactly as before #1048.
/// The marker is opt-in, not a new schema-aware default — pinned here so a
/// regression that made `dynamic_to_toml` "smart" about `repair_team_count`
/// specifically (the hand-maintained field table issue #1048 explicitly
/// rejected) would fail this test, not silently pass it.
#[test]
fn a_bare_int_on_the_same_integer_field_still_renders_as_the_ambient_float() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                    position: [0, 0, 0],
                    overrides: #{
                        repair: #{
                            repair_team_count: 3,
                        },
                    },
                });
            }"#,
        "f",
    );
    let BufferedEffect::Action(TriggerAction::SpawnEntity { overrides, .. }) = &effs[0] else {
        panic!("expected a spawn action, got {:?}", effs[0]);
    };
    let count = overrides
        .as_ref()
        .and_then(|o| o.get("repair"))
        .and_then(|r| r.get("repair_team_count"))
        .expect("override repair_team_count present");
    assert_eq!(
        count,
        &toml::Value::Float(3.0),
        "an unmarked int must still render as the ambient float default"
    );
    // And, unmarked, it does NOT deserialize into the integer field — the
    // authoring mistake `int(…)` exists to let an author avoid.
    let repair_result: Result<crate::entities::config::RepairConfig, _> = overrides
        .as_ref()
        .and_then(|o| o.get("repair"))
        .cloned()
        .expect("repair override present")
        .try_into();
    assert!(
        repair_result.is_err(),
        "a float leaf must NOT silently coerce into the integer field"
    );
}

/// `spawn_entity(#{…})` — the sharpest parity: the Rhai-int `position:
/// [900, 0, -560]` must produce the identical `[f32; 3]` the declarative float
/// `[900.0, 0.0, -560.0]` parses to, AND the `overrides` map must convert to
/// the identical `toml::Value` (numeric leaves as floats) the declarative
/// `overrides` carries.
#[test]
fn spawn_entity_matches_toml_including_int_to_float_and_overrides() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship_harrow_destroyer.toml",
                    name: "wave_1_bonus",
                    position: [900, 0, -560],
                    groups: ["hostiles", "wave_1"],
                    overrides: #{
                        weapons_console: #{
                            radar: #{
                                range: 200,
                                shows: ["player", "ship", "station"],
                            },
                        },
                    },
                });
            }"#,
        "f",
    );
    let toml = toml_action(
            "type = \"spawn_entity\"\n\
             template_path = \"assets/entities/ship_harrow_destroyer.toml\"\n\
             name = \"wave_1_bonus\"\n\
             position = [900.0, 0.0, -560.0]\n\
             groups = [\"hostiles\", \"wave_1\"]\n\
             overrides = { weapons_console = { radar = { range = 200.0, shows = [\"player\", \"ship\", \"station\"] } } }",
        );
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);

    // Pin the two conversions concretely so a regression is unmistakable.
    let BufferedEffect::Action(TriggerAction::SpawnEntity {
        position,
        overrides,
        ..
    }) = &effs[0]
    else {
        panic!("expected a spawn action, got {:?}", effs[0]);
    };
    assert_eq!(*position, Some([900.0_f32, 0.0, -560.0]));
    let range = overrides
        .as_ref()
        .and_then(|o| o.get("weapons_console"))
        .and_then(|w| w.get("radar"))
        .and_then(|r| r.get("range"))
        .expect("override range present");
    assert_eq!(
        range,
        &toml::Value::Float(200.0),
        "a no_float INT override leaf must render as a toml FLOAT"
    );
}

/// The anchor/position XOR is enforced by the SHARED parser: neither given
/// raises (discarding the call), exactly as the declarative parse errors.
#[test]
fn spawn_entity_requires_anchor_xor_position() {
    let err = run_result(
        r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                });
            }"#,
        "f",
    )
    .expect_err("neither anchor nor position must raise");
    assert!(err.to_string().contains("anchor"), "{err}");
}

/// A handler may DELEGATE to a helper fn in the same unit, and the helper's
/// effects land in the CALLER's buffer, in authored order (issue #984, M6
/// duel harness).
///
/// This is what lets a world author one parameterised spawn body and call it
/// from N one-line handlers instead of repeating the body N times: `ctx` is
/// copied into the callee (Rhai passes maps by value), but the `effects`
/// handle inside it is an [`EffectSink`] — `Arc<Mutex<_>>` — so every copy
/// pushes onto the ONE buffer the host drains. `duel.toml`'s generated slot
/// drivers ride on this; nothing else shipped did, so it is pinned here.
#[test]
fn a_helper_fn_shares_the_callers_effect_buffer() {
    let effs = run_buffered(
        r#"
            fn spawn_slot(ctx, name, template) {
                ctx.effects.spawn_entity(#{
                    template_path: template,
                    name: name,
                    position: [0, 0, 0],
                });
            }

            fn f(ctx) {
                ctx.effects.complete_objective("before");
                spawn_slot(ctx, "side_b_1", "assets/entities/ship.toml");
                ctx.effects.complete_objective("after");
            }"#,
        "f",
    );
    let spawned = toml_action(
        "type = \"spawn_entity\"\n\
             template_path = \"assets/entities/ship.toml\"\n\
             name = \"side_b_1\"\n\
             position = [0.0, 0.0, 0.0]",
    );
    assert_eq!(
        effs,
        vec![
            BufferedEffect::Cmd(ActionCmd::CompleteObjective {
                id: "before".to_string()
            }),
            BufferedEffect::Action(spawned),
            BufferedEffect::Cmd(ActionCmd::CompleteObjective {
                id: "after".to_string()
            }),
        ],
        "a helper fn's effects must interleave in the caller's buffer"
    );
}

/// A `Cmd` effect and an `Action` effect keep their authored order in the one
/// shared buffer — the interleaving guarantee flag writes also rely on.
#[test]
fn cmd_and_action_effects_interleave_in_authored_order() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.complete_objective("first");
                ctx.effects.add_faction_enemy("Harrow", "Alliance");
                ctx.effects.fail_objective("last");
            }"#,
        "f",
    );
    assert_eq!(
        effs,
        vec![
            BufferedEffect::Cmd(ActionCmd::CompleteObjective {
                id: "first".to_string()
            }),
            BufferedEffect::Action(TriggerAction::AddFactionEnemy {
                faction: "Harrow".to_string(),
                enemy: "Alliance".to_string(),
            }),
            BufferedEffect::Cmd(ActionCmd::FailObjective {
                id: "last".to_string()
            }),
        ]
    );
}

// ── open_comms (issue #984) ──────────────────────────────────────────────

/// The full authored form buffers one request with every optional carried,
/// and the drain stamps the running unit's path (the script never names it).
#[test]
fn open_comms_buffers_the_request_with_its_metadata() {
    let (effs, opens) = run_with_opens(
        r#"fn f(ctx) {
                ctx.effects.open_comms(#{
                    from: "axiom",
                    node_fn: "hail_axiom",
                    display_name: "Axiom Control",
                    thread_id: "aphelion",
                    urgent: true,
                });
            }"#,
        "f",
    );
    assert!(
        effs.is_empty(),
        "an open is not an ActionCmd/TriggerAction effect"
    );
    assert_eq!(
        opens,
        vec![OpenCommsRequest {
            sender_uuid: None,
            recipient_ship: None,
            recipients: None,
            from: "axiom".to_string(),
            root_fn: "hail_axiom".to_string(),
            display_name: Some("Axiom Control".to_string()),
            thread_id: Some("aphelion".to_string()),
            priority: crate::core::messages::CommsPriority::Urgent,
            urgent: true,
            script_path: "t.rhai".to_string(),
            origin_layer: None,
        }]
    );
}

/// Only `from` and `node_fn` are required; the rest default the way the
/// declarative `[[comms]]` template's optional fields do.
#[test]
fn open_comms_defaults_every_optional_key() {
    let (_e, opens) = run_with_opens(
        r#"fn f(ctx) { ctx.effects.open_comms(#{ from: "axiom", node_fn: "hail" }); }"#,
        "f",
    );
    assert_eq!(
        opens,
        vec![OpenCommsRequest {
            sender_uuid: None,
            recipient_ship: None,
            recipients: None,
            from: "axiom".to_string(),
            root_fn: "hail".to_string(),
            display_name: None,
            thread_id: None,
            priority: crate::core::messages::CommsPriority::Routine,
            urgent: false,
            script_path: "t.rhai".to_string(),
            origin_layer: None,
        }]
    );
}

#[test]
fn open_comms_accepts_authoritative_critical_priority() {
    let (_effects, opens) = run_with_opens(
        r#"fn f(ctx) {
                ctx.effects.open_comms(#{
                    from: "axiom",
                    node_fn: "hail",
                    priority: "critical",
                    urgent: false,
                });
            }"#,
        "f",
    );
    assert_eq!(opens.len(), 1);
    assert_eq!(
        opens[0].priority,
        crate::core::messages::CommsPriority::Critical
    );
    assert!(
        opens[0].urgent,
        "legacy urgency is only the compatibility projection"
    );
}

/// A missing required key raises, discarding the whole call (settled decision
/// 10) — including the effects authored before it.
#[test]
fn open_comms_raises_on_a_missing_required_key() {
    for (spec, wanted) in [
        (r#"#{ node_fn: "hail" }"#, "from"),
        (r#"#{ from: "axiom" }"#, "node_fn"),
    ] {
        let src = format!(
            "fn f(ctx) {{ ctx.effects.complete_objective(\"before\"); \
                 ctx.effects.open_comms({spec}); }}"
        );
        let err = run_result(&src, "f").expect_err("a missing required key must raise");
        assert!(
            err.to_string().contains(wanted),
            "the error should name `{wanted}`: {err}"
        );
    }
}

/// `node_fn`, not `fn`: `fn` is a Rhai keyword and the map-literal parser
/// takes only an identifier or a string as a property name, so `#{ fn: … }`
/// never even compiles. Pinned so the naming choice is evidence, not taste.
#[test]
fn fn_is_not_usable_as_a_map_key() {
    let engine = runtime_engine();
    assert!(
        engine
            .compile(r#"fn f(ctx) { ctx.effects.open_comms(#{ from: "a", fn: "b" }); }"#)
            .is_err(),
        "`fn` as a bare map key must be a parse error (hence `node_fn`)"
    );
}

/// Opens ride a SECOND buffer, so they cannot perturb the authored order of
/// the `Cmd`/`Action` sequence the applier dispatches — the one ordering
/// guarantee flag writes and name-resolving effects depend on.
#[test]
fn open_comms_does_not_disturb_authored_effect_order() {
    let (effs, opens) = run_with_opens(
        r#"fn f(ctx) {
                ctx.effects.complete_objective("first");
                ctx.effects.open_comms(#{ from: "axiom", node_fn: "hail" });
                ctx.effects.add_faction_enemy("Harrow", "Alliance");
                ctx.effects.fail_objective("last");
            }"#,
        "f",
    );
    assert_eq!(
        effs,
        vec![
            BufferedEffect::Cmd(ActionCmd::CompleteObjective {
                id: "first".to_string()
            }),
            BufferedEffect::Action(TriggerAction::AddFactionEnemy {
                faction: "Harrow".to_string(),
                enemy: "Alliance".to_string(),
            }),
            BufferedEffect::Cmd(ActionCmd::FailObjective {
                id: "last".to_string()
            }),
        ],
        "the ordered buffer must read exactly as it does without the open"
    );
    assert_eq!(opens.len(), 1);
}

/// A minimal context-free dispatch of one action to its `ActionCmd`s, for the
/// `game_over` structural-parity assertion (which needs no resolution). Mirrors
/// the comms module's `dispatch_toml`.
fn dispatch_bare(action: &TriggerAction) -> Vec<ActionCmd> {
    use crate::world::dispatch::{dispatch_action, DispatchContext};
    use std::collections::HashMap;
    let names: HashMap<String, String> = HashMap::new();
    let base_flags = crate::world::flags::FlagStore::new();
    let layers = HashMap::new();
    let anchors = HashMap::new();
    let uuid = || "uuid".to_string();
    let ctx = DispatchContext {
        origin_layer: None,
        entity_name: None,
        name_to_uuid: &names,
        base_flags: &base_flags,
        layers: &layers,
        base_anchors: &anchors,
        factions: None,
        uuid_source: &uuid,
        template_loader: &crate::entities::loader::WasmTemplateLoader,
    };
    dispatch_action(action, &ctx).commands
}
#[test]
fn contact_report_rhai_matches_typed_toml_and_rejects_negative_cadence() {
    let actions = run_buffered(
        r#"fn run(ctx) { ctx.effects.set_contact_report("observer", "target", 12, 500, true); ctx.effects.clear_contact_report("observer", "target"); }"#,
        "run",
    );
    let expected = toml_action(
        r#"type = "set_contact_information"
entity = "observer"
contact_information = { set_report_policy = { target = "target", policy = { delay_ticks = 12, position_step_mm = 500, hide_identity = true } } }"#,
    );
    assert!(matches!(&actions[0], BufferedEffect::Action(action) if action == &expected));
    assert!(
        matches!(&actions[1], BufferedEffect::Action(TriggerAction::SetContactInformation { change: crate::gm_information::ContactInformationChange::ClearReportPolicy { target }, .. }) if target == "target")
    );
    assert!(run_result(
        r#"fn run(ctx) { ctx.effects.set_contact_report("observer", "target", -1, 500, true); }"#,
        "run"
    )
    .is_err());
    assert!(run_result(
        r#"fn run(ctx) { ctx.effects.set_contact_report("observer", "target", 0, 0, false); }"#,
        "run"
    )
    .is_err());
}
