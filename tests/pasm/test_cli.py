import json
from pathlib import Path

from pasm.audit import build_audit_bundle
from pasm.core.validation import validate_spec_root
from pasm.cli.main import main


FIXTURES = Path(__file__).parent / "fixtures"
MIGRATION_FIXTURES = FIXTURES / "migration"


def test_cli_validate_json_success(capsys, monkeypatch) -> None:
    monkeypatch.setattr(
        "sys.argv",
        ["pasm", "validate", str(FIXTURES / "valid"), "--json"],
    )
    exit_code = main()
    captured = capsys.readouterr()
    payload = json.loads(captured.out)

    assert exit_code == 0
    assert payload["ok"] is True
    assert payload["entity_count"] == 13


def test_cli_validate_text_failure(capsys, monkeypatch) -> None:
    monkeypatch.setattr(
        "sys.argv",
        ["pasm", "validate", str(FIXTURES / "invalid")],
    )
    exit_code = main()
    captured = capsys.readouterr()

    assert exit_code == 1
    assert "Status: FAILED" in captured.out


def test_cli_query_entity_json_success(capsys, monkeypatch) -> None:
    monkeypatch.setattr(
        "sys.argv",
        ["pasm", "query", "entity", "authoritative-state", str(FIXTURES / "valid"), "--json"],
    )
    exit_code = main()
    captured = capsys.readouterr()
    payload = json.loads(captured.out)

    assert exit_code == 0
    assert payload["id"]["value"] == "authoritative-state"
    assert payload["architecture"]["classification"] == "authoritative"


def test_cli_query_implementation_json_success(capsys, monkeypatch) -> None:
    monkeypatch.setattr(
        "sys.argv",
        ["pasm", "query", "implementation", "engineering-station", str(FIXTURES / "valid"), "--json"],
    )
    exit_code = main()
    captured = capsys.readouterr()
    payload = json.loads(captured.out)

    assert exit_code == 0
    assert payload["status"] == "declared"
    assert payload["paths"] == ["fixtures/valid/minimal.yaml"]


def test_cli_scan_json_success(capsys, monkeypatch) -> None:
    monkeypatch.setattr(
        "sys.argv",
        ["pasm", "scan", str(FIXTURES / "observed"), "--entity", "observed-repair-ui", "--json"],
    )
    exit_code = main()
    captured = capsys.readouterr()
    payload = json.loads(captured.out)

    assert exit_code == 0
    assert payload["entity_count"] == 1
    assert payload["entities"][0]["entity_id"] == "observed-repair-ui"
    assert any(
        symbol["name"] == "buildRepairConsoleState"
        for file in payload["entities"][0]["files"]
        for symbol in file["symbols"]
    )
    assert "inventory" in payload
    assert payload["inventory"]["files"]
    assert payload["inventory"]["dependencies"]


def test_cli_scan_json_includes_revision_linked_cargo_inventory(capsys, monkeypatch) -> None:
    repository_fixture = FIXTURES / "repository"
    monkeypatch.setattr(
        "sys.argv",
        [
            "pasm",
            "scan",
            str(repository_fixture / "spec"),
            "--workspace-root",
            str(repository_fixture),
            "--json",
        ],
    )
    exit_code = main()
    captured = capsys.readouterr()
    payload = json.loads(captured.out)

    assert exit_code == 0
    assert payload["inventory"]["revision"] is not None
    assert payload["inventory"]["cargo_packages"] == [
        {
            "dependencies": ["serde"],
            "manifest_path": "Cargo.toml",
            "name": "pasm-observation-fixture",
        }
    ]


def test_cli_query_migration_json_success(capsys, monkeypatch) -> None:
    monkeypatch.setattr(
        "sys.argv",
        [
            "pasm",
            "query",
            "migration",
            "helm-driver-rollout",
            str(MIGRATION_FIXTURES / "valid"),
            "--workspace-root",
            str(FIXTURES.parent),
            "--json",
        ],
    )
    exit_code = main()
    captured = capsys.readouterr()
    payload = json.loads(captured.out)

    assert exit_code == 0
    assert payload["legacy_entities"] == [{"value": "legacy-helm-driver"}]
    assert payload["target_entities"] == [{"value": "helm-motion-planner-target"}]


def test_cli_traceability_json_success(capsys, monkeypatch) -> None:
    repository_root = Path(__file__).parents[2]
    monkeypatch.setattr(
        "sys.argv",
        [
            "pasm", "traceability", str(repository_root / "pasm" / "spec"),
            "--workspace-root", str(repository_root), "--json",
        ],
    )
    exit_code = main()
    captured = capsys.readouterr()
    payload = json.loads(captured.out)

    assert exit_code == 0
    repair_row = next(row for row in payload["rows"] if row["design_entity"]["value"] == "dispatch-repair-team")
    # See tests/pasm/test_cross_domain.py for the rationale: issue #736
    # (commit 43282651) made two of this verb's three realizing entities
    # `observed` while the message stayed `declared` (hence `mixed`), and moved
    # the host router from `src/console/repair/server.rs` to
    # `src/console/repair/dispatch.rs`.
    assert repair_row["implementation_status"] == "mixed"
    assert "src/console/repair/dispatch.rs" in repair_row["implementation_paths"]


def test_cli_audit_bundle_json_success(capsys, monkeypatch) -> None:
    monkeypatch.setattr("sys.argv", ["pasm", "audit", "bundle", "observed-repair-ui", str(FIXTURES / "observed"), "--workspace-root", str(FIXTURES.parent)])
    assert main() == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload["schema_version"] == 1
    assert payload["entity"]["id"]["value"] == "observed-repair-ui"
    assert len(payload["bundle_sha256"]) == 64


def test_cli_audit_report_persists_bundle_bound_evidence(tmp_path: Path, capsys, monkeypatch) -> None:
    model = validate_spec_root(FIXTURES / "observed", workspace_root=FIXTURES.parent).model
    entity = model.entity_by_id("observed-repair-ui")
    assert entity is not None
    bundle = build_audit_bundle(entity, FIXTURES.parent)
    bundle_path = tmp_path / "bundle.json"
    bundle_path.write_text(json.dumps(bundle), encoding="utf-8")
    report_path = tmp_path / "report.json"
    report_path.write_text(json.dumps({
        "schema_version": 1,
        "audit_kind": "architecture",
        "repository_revision": bundle["repository_revision"],
        "bundle_sha256": bundle["bundle_sha256"],
        "entity_ids": bundle["entity_ids"],
        "findings": [],
    }), encoding="utf-8")
    history = tmp_path / "history"
    monkeypatch.setattr("sys.argv", ["pasm", "audit", "report", str(report_path), "--bundle", str(bundle_path), "--persist-dir", str(history), "--json"])
    assert main() == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload["audit_kind"] == "architecture"
    assert Path(payload["persisted_path"]).is_file()
    assert (history / "bundles" / f"{bundle['bundle_sha256']}.json").is_file()


def test_cli_context_json_success(capsys, monkeypatch) -> None:
    monkeypatch.setattr("sys.argv", ["pasm", "context", "--entity", "authoritative-state", "--depth", "1", str(FIXTURES / "valid"), "--json"])
    assert main() == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload["seeds"] == ["authoritative-state"]
    assert payload["dependency_depth"] == 1
