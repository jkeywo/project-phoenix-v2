from pathlib import Path

from pasm.core.validation import validate_spec_root
from pasm.domains.game_design.scenarios import load_scenario, validate_scenario


ROOT = Path(__file__).parents[2]


def test_authored_repair_helm_and_red_alert_scenarios_run() -> None:
    model = validate_spec_root(ROOT / "pasm" / "spec", workspace_root=ROOT).model
    for name in ("repair-diagnosis.yaml", "helm-control.yaml", "red-alert.yaml"):
        path = ROOT / "pasm" / "spec" / "scenarios" / name
        assert not validate_scenario(load_scenario(path), model.entities, path)


def test_scenario_detects_premature_reveal_and_wrong_role(tmp_path: Path) -> None:
    path = tmp_path / "bad.yaml"
    path.write_text("""scenario:
  id: bad-repair
  steps:
    - kind: action
      actor: station-owner
      value: dispatch-repair-team
    - kind: reveal
      value: onsite-noncore-damage-detail-information
""", encoding="utf-8")
    model = validate_spec_root(ROOT / "pasm" / "spec", workspace_root=ROOT).model
    findings = validate_scenario(load_scenario(path), model.entities, path)
    ids = {item.id for item in findings}
    assert "scenario-wrong-role-action:bad-repair:dispatch-repair-team:1" in ids
    assert "scenario-premature-reveal:bad-repair:onsite-noncore-damage-detail-information:2" in ids
    assert next(item for item in findings if item.id.startswith("scenario-wrong-role")).implementation_locations[0].line == 4


def test_scenario_rejects_malformed_and_unknown_steps(tmp_path: Path) -> None:
    path = tmp_path / "bad.yaml"
    path.write_text("scenario: []\n", encoding="utf-8")
    try:
        load_scenario(path)
    except ValueError:
        pass
    else:
        raise AssertionError("malformed scenario should fail")
    path.write_text("""scenario:
  id: unknown
  steps:
    - kind: action
      actor: engineering
      value: missing-action
""", encoding="utf-8")
    model = validate_spec_root(ROOT / "pasm" / "spec", workspace_root=ROOT).model
    findings = validate_scenario(load_scenario(path), model.entities, path)
    assert any(item.id.startswith("scenario-wrong-role-action:unknown:missing-action") for item in findings)


def test_scenario_detects_unreachable_authored_fact_transition(tmp_path: Path) -> None:
    path = tmp_path / "unreachable.yaml"
    path.write_text("""scenario:
  id: unreachable-repair
  initial_facts: [damage-reported]
  steps:
    - kind: action
      actor: engineering
      value: dispatch-repair-team
      requires_facts: [team-arrived]
""", encoding="utf-8")
    model = validate_spec_root(ROOT / "pasm" / "spec", workspace_root=ROOT).model
    findings = validate_scenario(load_scenario(path), model.entities, path)
    ids = {item.id for item in findings}
    assert "scenario-action-preconditions:unreachable-repair:dispatch-repair-team:1" in ids
    assert "scenario-unreachable-action:unreachable-repair:dispatch-repair-team:1" in ids
