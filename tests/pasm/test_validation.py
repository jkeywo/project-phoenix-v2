from pathlib import Path

from pasm.core.validation import validate_spec_root
from pasm.implementation.observation import observe_repository


FIXTURES = Path(__file__).parent / "fixtures"
MIGRATION_FIXTURES = FIXTURES / "migration"
REPOSITORY_FIXTURES = FIXTURES / "repository"
REPOSITORY_ROOT = Path(__file__).parents[2]


def test_valid_model_loads_without_errors() -> None:
    result = validate_spec_root(FIXTURES / "valid")
    assert result.ok is True
    assert len(result.model.entities) == 13
    assert all(f.severity.value != "error" for f in result.findings)
    minimal_entity = result.model.entity_by_id("engineering-station")
    assert minimal_entity is not None
    assert minimal_entity.references[0].target.value == "host-simulation"
    assert minimal_entity.implementation is not None
    assert minimal_entity.implementation.paths[0].as_posix() == "fixtures/valid/minimal.yaml"
    architecture_entity = result.model.entity_by_id("authoritative-state")
    assert architecture_entity is not None
    assert architecture_entity.architecture is not None
    assert architecture_entity.architecture.owner is not None
    assert architecture_entity.architecture.owner.value == "host-sim"


def test_authored_pasm_specs_validate() -> None:
    result = validate_spec_root(REPOSITORY_ROOT / "pasm" / "spec", workspace_root=REPOSITORY_ROOT)

    assert result.model.entity_by_id("helm-shared-motion-planner-rollout") is not None
    assert not any(
        finding.rule.startswith("yaml.") or finding.rule == "core.confidence-valid"
        for finding in result.findings
    ), [finding.summary for finding in result.findings]


def test_duplicate_entity_is_reported() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding_ids = {finding.id for finding in result.findings}
    assert "duplicate-entity:component:engineering-station" in finding_ids


def test_unknown_field_is_reported_with_location() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding = next(f for f in result.findings if f.id.startswith("unknown-entity-field"))
    assert finding.implementation_locations[0].path.as_posix() == "unknown_field.yaml"
    assert finding.implementation_locations[0].line == 3


def test_broken_reference_is_reported() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding = next(f for f in result.findings if f.id.startswith("unknown-reference"))
    assert finding.rule == "core.references-target-exists"


def test_invalid_status_is_reported() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding = next(f for f in result.findings if f.id.startswith("invalid-status"))
    assert finding.severity.value == "error"


def test_malformed_yaml_is_reported() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding = next(f for f in result.findings if f.id.startswith("yaml:malformed_yaml.yaml"))
    assert finding.implementation_locations[0].line is not None


def test_temporary_exception_requires_removal_condition() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding = next(
        f
        for f in result.findings
        if f.id.startswith("temporary-exception-without-removal")
    )
    assert finding.spec_entities[0].value == "legacy-panel"


def test_duplicate_authoritative_ownership_is_reported() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding = next(
        f for f in result.findings if f.id.startswith("duplicate-authoritative-ownership")
    )
    assert finding.rule == "architecture.single-authoritative-owner"


def test_forbidden_dependency_is_reported() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding = next(f for f in result.findings if f.id.startswith("forbidden-dependency"))
    assert finding.spec_entities[0].value == "mixed-dependency"


def test_non_authoritative_owner_is_reported() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding = next(f for f in result.findings if f.id.startswith("non-authoritative-owner"))
    assert finding.rule == "architecture.non-authoritative-cannot-own-authoritative-state"


def test_trust_boundary_message_requires_validator() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding = next(
        f
        for f in result.findings
        if f.id.startswith("trust-boundary-message-missing-validator")
    )
    assert finding.spec_entities[0].value == "unvalidated-command"


def test_missing_implementation_path_is_reported() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding = next(
        f for f in result.findings if f.id.startswith("missing-implementation-path")
    )
    assert finding.rule == "implementation.path-exists"


def test_empty_implementation_section_is_reported() -> None:
    result = validate_spec_root(FIXTURES / "invalid")
    finding = next(
        f for f in result.findings if f.id.startswith("empty-implementation-mapping")
    )
    assert finding.rule == "implementation.nonempty"


def test_observed_symbol_and_message_mismatches_are_reported() -> None:
    result = validate_spec_root(FIXTURES / "observed")
    finding_ids = {finding.id for finding in result.findings}

    assert "missing-observed-symbol:stale-repair-ui:missingSymbol" in finding_ids
    assert "missing-observed-message:stale-repair-ui:MissingMessage" in finding_ids


def test_observed_symbol_matches_do_not_report_findings() -> None:
    result = validate_spec_root(FIXTURES / "observed")
    finding_ids = {finding.id for finding in result.findings}

    assert "missing-observed-symbol:observed-repair-ui:buildRepairConsoleState" not in finding_ids
    assert "missing-observed-symbol:observed-repair-ui:handle_dispatch_repair_team" not in finding_ids
    assert "missing-observed-message:observed-repair-ui:DispatchRepairTeam" not in finding_ids


def test_repository_inventory_records_languages_cargo_and_local_edges() -> None:
    inventory = observe_repository(REPOSITORY_FIXTURES)

    assert inventory.revision is not None
    assert inventory.cargo_packages[0].name == "pasm-observation-fixture"
    assert inventory.cargo_packages[0].dependencies == ("serde",)
    assert {file.language for file in inventory.files} == {"html", "javascript", "rust", "typescript"}
    assert any(
        edge.source.as_posix() == "src/alpha.rs" and edge.target.as_posix() == "src/gamma.rs"
        for edge in inventory.dependencies
    )
    assert any(
        edge.source.as_posix() == "ui/page.html" and edge.target.as_posix() == "ui/app.js"
        for edge in inventory.dependencies
    )


def test_repository_symbol_references_are_source_located() -> None:
    from pasm.implementation.observation import find_repository_symbol_references

    references = find_repository_symbol_references(
        observe_repository(REPOSITORY_FIXTURES), "run"
    )

    assert any(reference.path.as_posix() == "src/alpha.rs" for reference in references)
    assert all(reference.line is not None for reference in references)


def test_observed_repository_dependency_drift_is_reported() -> None:
    result = validate_spec_root(REPOSITORY_FIXTURES / "spec", workspace_root=REPOSITORY_FIXTURES)
    finding_ids = {finding.id for finding in result.findings}

    assert "missing-observed-dependency:alpha:beta" in finding_ids
    assert "undeclared-observed-dependency:alpha:gamma" in finding_ids
    assert "missing-observed-dependency:web-shell:web-app" not in finding_ids
    assert "missing-observed-dependency:web-app:web-helper" not in finding_ids


def test_migration_valid_fixture_reports_pending_symbol_removal_only() -> None:
    result = validate_spec_root(MIGRATION_FIXTURES / "valid", workspace_root=FIXTURES.parent)
    finding_ids = {finding.id for finding in result.findings}

    assert result.ok is True
    assert "migration-removal-condition-pending:helm-driver-rollout:1" in finding_ids
    assert not any(finding_id.startswith("undeclared-legacy-caller:helm-driver-rollout") for finding_id in finding_ids)


def test_migration_invalid_fixture_reports_undeclared_legacy_caller() -> None:
    result = validate_spec_root(MIGRATION_FIXTURES / "invalid", workspace_root=FIXTURES.parent)
    finding_ids = {finding.id for finding in result.findings}

    assert result.ok is False
    assert "undeclared-legacy-caller:invalid-helm-driver-rollout:invalid-rogue-caller:oldHelmDriver" in finding_ids


def test_migration_invalid_fixture_reports_overlap_and_target_residue() -> None:
    result = validate_spec_root(MIGRATION_FIXTURES / "invalid", workspace_root=FIXTURES.parent)
    finding_ids = {finding.id for finding in result.findings}

    assert "migration-overlapping-writers:invalid-helm-driver-rollout:invalid-migration-authoritative-state" in finding_ids
    assert "migration-target-still-references-legacy:invalid-helm-driver-rollout:invalid-helm-motion-planner-target:oldHelmDriver" in finding_ids
