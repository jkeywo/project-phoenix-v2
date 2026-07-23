from pathlib import Path

from pasm.core.validation import validate_spec_root
from pasm.integration.traceability import build_traceability_rows


REPOSITORY_ROOT = Path(__file__).parents[2]


def test_authored_design_slices_have_traceability_rows() -> None:
    result = validate_spec_root(REPOSITORY_ROOT / "pasm" / "spec", workspace_root=REPOSITORY_ROOT)
    rows = {row.design_entity.value: row for row in build_traceability_rows(result.model.entities)}

    # `dispatch-repair-team` realizes across three architecture entities:
    # repair-console-interface, dispatch-repair-team-command, host-repair-router.
    # Issue #736 ("explicit Repair command gateway and host admission seam",
    # commit 43282651) promoted the console interface and the host router from
    # `declared` to evidence-backed `observed`, while the message entity stayed
    # `declared` — so the realizing set is genuinely no longer uniform and
    # `mixed` is the correct roll-up. The same commit extracted the host router
    # out of `src/console/repair/server.rs` into a new
    # `src/console/repair/dispatch.rs`, which is where the dispatch handler and
    # its admission-ordered registration now live.
    assert rows["dispatch-repair-team"].implementation_status == "mixed"
    assert "src/console/repair/dispatch.rs" in rows["dispatch-repair-team"].implementation_paths
    # Issue #748 landed the ToggleRedAlert→SetRedAlert migration: the
    # `set-red-alert` verb's realizing architecture entities
    # (`red-alert-console-interface`, `set-red-alert-command`) flipped from
    # proposed to implemented with declared implementation mappings, so the
    # traceability roll-up is now `declared` rather than `declared-design-only`.
    assert rows["set-red-alert"].implementation_status == "declared"
    assert rows["set-helm-actuator-input"].architecture_links


def test_cross_domain_checks_report_missing_paths_with_field_location(tmp_path: Path) -> None:
    (tmp_path / "model.yaml").write_text(
        """entities:
  - component: captain-interface
    core: {status: accepted}
  - component: command-router
    core: {status: accepted}
    architecture: {authority: authoritative}
  - component: unmapped-router
    core: {status: implemented}
    architecture: {authority: authoritative}
  - role: captain
    core: {status: accepted}
    game_design:
      architecture_links: [captain-interface]
  - verb: set-alert
    core: {status: accepted}
    game_design:
      owner_role: captain
  - information: private-alert
    core: {status: accepted}
    game_design:
      visibility: hidden
      reveal_conditions: [team-arrives]
  - decision: guarded-alert
    core: {status: accepted}
    game_design:
      architecture_links: [captain-interface]
      owner_role: captain
      protected: true
      must_not_be: [fully-automated]
      enforcement_links: [unmapped-router]
  - role: invalid-link-role
    core: {status: accepted}
    game_design:
      architecture_links: [captain]
""",
        encoding="utf-8",
    )

    result = validate_spec_root(tmp_path)
    finding_ids = {finding.id for finding in result.findings}
    action_finding = next(item for item in result.findings if item.id == "role-action-missing-architecture-link:set-alert")
    mapping_finding = next(item for item in result.findings if item.id == "design-link-missing-implementation:guarded-alert:unmapped-router")

    assert "information-missing-enforcement-link:private-alert" in finding_ids
    assert "protected-decision-missing-authoritative-enforcement:guarded-alert" not in finding_ids
    assert "design-link-missing-implementation:guarded-alert:unmapped-router" in finding_ids
    assert "cross-domain-link-not-architecture:invalid-link-role:captain" in finding_ids
    assert action_finding.implementation_locations[0].line == 14
    assert mapping_finding.implementation_locations[0].line == 30
