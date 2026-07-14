import json
from pathlib import Path

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
