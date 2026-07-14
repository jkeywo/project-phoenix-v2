from pathlib import Path

from pasm.core.validation import validate_spec_root


REPOSITORY_ROOT = Path(__file__).parents[2]


def test_authored_engineering_diagnosis_design_is_typed_and_valid() -> None:
    result = validate_spec_root(REPOSITORY_ROOT / "pasm" / "spec", workspace_root=REPOSITORY_ROOT)

    engineering = result.model.entity_by_id("engineering")
    onsite_detail = result.model.entity_by_id("onsite-noncore-damage-detail-information")
    helm_decision = result.model.entity_by_id("helm-course-and-speed")
    target_alert = result.model.entity_by_id("selected-target-red-alert-information")
    assert engineering is not None and engineering.game_design is not None
    assert onsite_detail is not None and onsite_detail.game_design is not None
    assert engineering.game_design.protected_decisions[0].value == "repair-team-dispatch"
    assert onsite_detail.game_design.visibility is not None
    assert onsite_detail.game_design.visibility.value == "hidden"
    assert onsite_detail.game_design.reveal_conditions
    assert helm_decision is not None and helm_decision.game_design is not None
    assert helm_decision.game_design.protected is True
    assert target_alert is not None and target_alert.game_design is not None
    assert target_alert.game_design.permitted_viewers[0].value == "sensors-operator"
    assert not any(finding.rule.startswith("game-design.") for finding in result.findings)


def test_game_design_validators_report_missing_design_integrity(tmp_path: Path) -> None:
    (tmp_path / "invalid.yaml").write_text(
        """entities:
  - verb: repair
    core: {status: accepted}
    game_design: {}
  - decision: priority
    core: {status: accepted}
    game_design: {protected: true}
  - information: fault-detail
    core: {status: accepted}
    game_design: {visibility: hidden}
  - resource: repair-capacity
    core: {status: accepted}
    game_design: {}
  - failure: damaged-system
    core: {status: accepted}
    game_design: {terminal: false}
""",
        encoding="utf-8",
    )

    result = validate_spec_root(tmp_path)
    finding_ids = {finding.id for finding in result.findings}

    assert "player-verb-missing-owner:repair" in finding_ids
    assert "protected-decision-missing-owner:priority" in finding_ids
    assert "protected-decision-missing-bypass-policy:priority" in finding_ids
    assert "hidden-information-missing-reveal-condition:fault-detail" in finding_ids
    assert "resource-missing-source:repair-capacity" in finding_ids
    assert "resource-missing-sink:repair-capacity" in finding_ids
    assert "failure-missing-consequence:damaged-system" in finding_ids
    assert "nonterminal-failure-missing-recovery:damaged-system" in finding_ids


def test_game_design_unknown_field_has_source_location(tmp_path: Path) -> None:
    (tmp_path / "invalid.yaml").write_text(
        """entities:
  - role: engineering
    core: {status: accepted}
    game_design:
      unknown_rule: no
""",
        encoding="utf-8",
    )

    result = validate_spec_root(tmp_path)
    finding = next(item for item in result.findings if item.id.startswith("unknown-field"))

    assert finding.rule == "yaml.unknown-field"
    assert finding.implementation_locations[0].path.as_posix() == "invalid.yaml"
    assert finding.implementation_locations[0].line == 5


def test_documented_game_design_vocabulary_and_aliases_are_validated(tmp_path: Path) -> None:
    (tmp_path / "documented.yaml").write_text(
        """entities:
  - player_role: engineering
    core: {status: accepted}
  - action: choose-repair-priority
    core: {status: accepted}
    game_design:
      player_role: engineering
      protected: true
      must_not_be: [fully-automated]
  - information_set: fault-detail
    core: {status: accepted}
    game_design:
      visibility: hidden
      reveal_condition: [successful-diagnosis]
      permitted_viewers: [engineering]
  - failure_state: damaged-system
    core: {status: accepted}
    game_design:
      consequences: [reduced-capability]
      terminal: false
      recovery_paths: [repair-system]
""",
        encoding="utf-8",
    )

    result = validate_spec_root(tmp_path)

    assert not any(finding.rule.startswith("game-design.") for finding in result.findings)
    action = result.model.entity_by_id("choose-repair-priority")
    information = result.model.entity_by_id("fault-detail")
    assert action is not None and action.game_design is not None
    assert information is not None and information.game_design is not None
    assert action.game_design.owner_role is not None
    assert action.game_design.owner_role.value == "engineering"
    assert information.game_design.reveal_conditions == ("successful-diagnosis",)


def test_tuning_playtest_and_semantic_findings_require_field_level_data(tmp_path: Path) -> None:
    (tmp_path / "invalid.yaml").write_text(
        """entities:
  - tuning: repair-speed
    core: {status: accepted}
    game_design: {}
  - playtest-claim: repair-pressure
    core: {status: accepted}
    game_design: {}
  - role: engineering
    core: {status: accepted}
  - information: core-detail
    core: {status: accepted}
    game_design:
      visibility: role-visible
      permitted_viewers: [core-detail]
""",
        encoding="utf-8",
    )

    result = validate_spec_root(tmp_path)
    finding_ids = {finding.id for finding in result.findings}
    viewer_finding = next(item for item in result.findings if item.id == "information-viewer-not-role:core-detail:core-detail")

    assert "tuning-missing-affected-mechanics:repair-speed" in finding_ids
    assert "tuning-missing-intended-directional-effect:repair-speed" in finding_ids
    assert "tuning-missing-bounds:repair-speed" in finding_ids
    assert "tuning-missing-maturity:repair-speed" in finding_ids
    assert "playtest-claim-missing-claim:repair-pressure" in finding_ids
    assert "playtest-claim-missing-support:repair-pressure" in finding_ids
    assert viewer_finding.implementation_locations[0].line == 14
