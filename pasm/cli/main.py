from __future__ import annotations

import argparse
import json
from pathlib import Path
from dataclasses import asdict, is_dataclass
from enum import Enum

from pasm.core.validation import ValidationResult, validate_spec_root
from pasm.implementation.observation import observe_entity_implementation, observe_repository


def main() -> int:
    parser = argparse.ArgumentParser(prog="pasm")
    subparsers = parser.add_subparsers(dest="command", required=True)

    validate_parser = subparsers.add_parser("validate", help="Validate PASM YAML files.")
    validate_parser.add_argument(
        "spec_root",
        nargs="?",
        default="pasm/spec",
        help="Directory containing PASM YAML files.",
    )
    validate_parser.add_argument(
        "--json",
        action="store_true",
        help="Emit machine-readable JSON.",
    )
    validate_parser.add_argument(
        "--workspace-root",
        help="Repository root used to resolve implementation paths.",
    )
    scan_parser = subparsers.add_parser(
        "scan",
        help="Build a repository inventory and report declared implementation observations.",
    )
    scan_parser.add_argument(
        "spec_root",
        nargs="?",
        default="pasm/spec",
        help="Directory containing PASM YAML files.",
    )
    scan_parser.add_argument(
        "--entity",
        dest="entity_id",
        help="Optional semantic entity ID to scan. Defaults to all entities with implementation mappings.",
    )
    scan_parser.add_argument(
        "--json",
        action="store_true",
        help="Emit machine-readable JSON.",
    )
    scan_parser.add_argument(
        "--workspace-root",
        help="Repository root used to resolve implementation paths.",
    )
    query_parser = subparsers.add_parser("query", help="Query PASM model data.")
    query_subparsers = query_parser.add_subparsers(dest="query_command", required=True)
    entity_parser = query_subparsers.add_parser("entity", help="Show one entity by semantic ID.")
    entity_parser.add_argument("entity_id", help="Semantic entity ID to load.")
    entity_parser.add_argument(
        "spec_root",
        nargs="?",
        default="pasm/spec",
        help="Directory containing PASM YAML files.",
    )
    entity_parser.add_argument(
        "--json",
        action="store_true",
        help="Emit machine-readable JSON.",
    )
    entity_parser.add_argument(
        "--workspace-root",
        help="Repository root used to resolve implementation paths.",
    )
    implementation_parser = query_subparsers.add_parser(
        "implementation", help="Show declared implementation mapping for one entity."
    )
    implementation_parser.add_argument("entity_id", help="Semantic entity ID to load.")
    implementation_parser.add_argument(
        "spec_root",
        nargs="?",
        default="pasm/spec",
        help="Directory containing PASM YAML files.",
    )
    implementation_parser.add_argument(
        "--json",
        action="store_true",
        help="Emit machine-readable JSON.",
    )
    implementation_parser.add_argument(
        "--workspace-root",
        help="Repository root used to resolve implementation paths.",
    )
    migration_parser = query_subparsers.add_parser(
        "migration", help="Show declared migration semantics for one entity."
    )
    migration_parser.add_argument("entity_id", help="Semantic entity ID to load.")
    migration_parser.add_argument(
        "spec_root",
        nargs="?",
        default="pasm/spec",
        help="Directory containing PASM YAML files.",
    )
    migration_parser.add_argument(
        "--json",
        action="store_true",
        help="Emit machine-readable JSON.",
    )
    migration_parser.add_argument(
        "--workspace-root",
        help="Repository root used to resolve implementation paths.",
    )

    args = parser.parse_args()
    if args.command == "validate":
        result = _validate_from_args(args)
        if args.json:
            print(_result_to_json(result))
        else:
            print(_result_to_text(result))
        return result.exit_code
    if args.command == "scan":
        result = _validate_from_args(args)
        entities = list(result.model.entities)
        if args.entity_id:
            entity = result.model.entity_by_id(args.entity_id)
            if entity is None:
                payload = {
                    "ok": False,
                    "error": f"Entity '{args.entity_id}' was not found.",
                    "spec_root": result.model.spec_root.as_posix(),
                }
                if args.json:
                    print(json.dumps(payload, indent=2, sort_keys=True))
                else:
                    print(payload["error"])
                return 1
            entities = [entity]

        workspace_root = _workspace_root_from_args(args, result)
        inventory = observe_repository(workspace_root)
        observations = [
            observe_entity_implementation(entity, workspace_root)
            for entity in entities
            if entity.implementation is not None
        ]
        if args.json:
            print(_scan_to_json(observations, inventory, result))
        else:
            print(_scan_to_text(observations, inventory, result))
        return result.exit_code
    if args.command == "query" and args.query_command == "entity":
        result = _validate_from_args(args)
        if not result.ok:
            if args.json:
                print(_result_to_json(result))
            else:
                print(_result_to_text(result))
            return result.exit_code
        entity = result.model.entity_by_id(args.entity_id)
        if entity is None:
            payload = {
                "ok": False,
                "error": f"Entity '{args.entity_id}' was not found.",
                "spec_root": result.model.spec_root.as_posix(),
            }
            if args.json:
                print(json.dumps(payload, indent=2, sort_keys=True))
            else:
                print(payload["error"])
            return 1
        if args.json:
            print(json.dumps(_json_ready(entity), indent=2, sort_keys=True))
        else:
            print(_entity_to_text(entity))
        return 0
    if args.command == "query" and args.query_command == "implementation":
        result = _validate_from_args(args)
        if not result.ok:
            if args.json:
                print(_result_to_json(result))
            else:
                print(_result_to_text(result))
            return result.exit_code
        entity = result.model.entity_by_id(args.entity_id)
        if entity is None:
            payload = {
                "ok": False,
                "error": f"Entity '{args.entity_id}' was not found.",
                "spec_root": result.model.spec_root.as_posix(),
            }
            if args.json:
                print(json.dumps(payload, indent=2, sort_keys=True))
            else:
                print(payload["error"])
            return 1
        if entity.implementation is None:
            payload = {
                "ok": False,
                "error": f"Entity '{args.entity_id}' has no declared implementation mapping.",
            }
            if args.json:
                print(json.dumps(payload, indent=2, sort_keys=True))
            else:
                print(payload["error"])
            return 1
        if args.json:
            print(json.dumps(_json_ready(entity.implementation), indent=2, sort_keys=True))
        else:
            print(_implementation_to_text(args.entity_id, entity.implementation))
        return 0
    if args.command == "query" and args.query_command == "migration":
        result = _validate_from_args(args)
        if not result.ok:
            if args.json:
                print(_result_to_json(result))
            else:
                print(_result_to_text(result))
            return result.exit_code
        entity = result.model.entity_by_id(args.entity_id)
        if entity is None:
            payload = {
                "ok": False,
                "error": f"Entity '{args.entity_id}' was not found.",
                "spec_root": result.model.spec_root.as_posix(),
            }
            if args.json:
                print(json.dumps(payload, indent=2, sort_keys=True))
            else:
                print(payload["error"])
            return 1
        if entity.migration is None:
            payload = {
                "ok": False,
                "error": f"Entity '{args.entity_id}' has no declared migration semantics.",
            }
            if args.json:
                print(json.dumps(payload, indent=2, sort_keys=True))
            else:
                print(payload["error"])
            return 1
        if args.json:
            print(json.dumps(_json_ready(entity.migration), indent=2, sort_keys=True))
        else:
            print(_migration_to_text(args.entity_id, entity.migration))
        return 0

    parser.error(f"Unsupported command: {args.command}")
    return 2


def _validate_from_args(args) -> ValidationResult:
    workspace_root = getattr(args, "workspace_root", None)
    return validate_spec_root(
        Path(args.spec_root),
        Path(workspace_root) if workspace_root else None,
    )


def _workspace_root_from_args(args, result: ValidationResult) -> Path:
    workspace_root = getattr(args, "workspace_root", None)
    return (
        Path(workspace_root).resolve()
        if workspace_root
        else result.model.spec_root.parent.parent.resolve()
    )


def _result_to_text(result: ValidationResult) -> str:
    lines = [
        f"Spec root: {result.model.spec_root.as_posix()}",
        f"Entities: {len(result.model.entities)}",
        f"Findings: {len(result.findings)}",
        "Status: OK" if result.ok else "Status: FAILED",
    ]
    for finding in result.findings:
        lines.append(
            f"[{finding.severity.value}] {finding.id} | {finding.summary}"
        )
        for location in finding.implementation_locations:
            lines.append(f"  at {location.render()}")
    return "\n".join(lines)


def _result_to_json(result: ValidationResult) -> str:
    payload = {
        "ok": result.ok,
        "exit_code": result.exit_code,
        "spec_root": result.model.spec_root.as_posix(),
        "entity_count": len(result.model.entities),
        "finding_count": len(result.findings),
        "entities": [_json_ready(entity) for entity in result.model.entities],
        "findings": [_json_ready(finding) for finding in result.findings],
    }
    return json.dumps(payload, indent=2, sort_keys=True)


def _scan_to_text(observations, inventory, result) -> str:
    lines = [
        "Validation: OK" if result.ok else "Validation: FAILED",
        f"Repository revision: {inventory.revision or 'unavailable'}",
        f"Repository files: {len(inventory.files)}",
        f"Observed dependencies: {len(inventory.dependencies)}",
        f"Cargo packages: {len(inventory.cargo_packages)}",
    ]
    for observation in observations:
        lines.append(f"Entity: {observation.entity_id}")
        lines.append(f"Observed files: {len(observation.files)}")
        for observed_file in observation.files:
            lines.append(
                f"  - {observed_file.path.as_posix()} [{observed_file.language}; "
                f"{len(observed_file.symbols)} symbols, {len(observed_file.imports)} imports]"
            )
    return "\n".join(lines) if lines else "No implementation mappings were eligible for scanning."


def _scan_to_json(observations, inventory, result) -> str:
    payload = {
        "ok": result.ok,
        "exit_code": result.exit_code,
        "entity_count": len(observations),
        "findings": [_json_ready(finding) for finding in result.findings],
        "entities": [
            {
                "entity_id": observation.entity_id,
                "file_count": len(observation.files),
                "files": [
                    {
                        "path": observed_file.path.as_posix(),
                        "language": observed_file.language,
                        "symbols": [
                            {
                                "name": symbol.name,
                                "kind": symbol.kind,
                                "location": _json_ready(symbol.location),
                            }
                            for symbol in observed_file.symbols
                        ],
                    }
                    for observed_file in observation.files
                ],
            }
            for observation in observations
        ],
        "inventory": {
            "workspace_root": inventory.workspace_root.as_posix(),
            "revision": inventory.revision,
            "cargo_packages": [
                {
                    "name": package.name,
                    "manifest_path": package.manifest_path.as_posix(),
                    "dependencies": list(package.dependencies),
                }
                for package in inventory.cargo_packages
            ],
            "files": [
                {
                    "path": observed_file.path.as_posix(),
                    "language": observed_file.language,
                    "symbols": [
                        {
                            "name": symbol.name,
                            "kind": symbol.kind,
                            "location": _json_ready(symbol.location),
                        }
                        for symbol in observed_file.symbols
                    ],
                    "imports": [
                        {
                            "kind": imported.kind,
                            "target": imported.target,
                            "location": _json_ready(imported.location),
                        }
                        for imported in observed_file.imports
                    ],
                }
                for observed_file in inventory.files
            ],
            "dependencies": [
                {
                    "source": edge.source.as_posix(),
                    "target": edge.target.as_posix(),
                    "kind": edge.kind,
                    "location": _json_ready(edge.location),
                }
                for edge in inventory.dependencies
            ],
        },
    }
    return json.dumps(payload, indent=2, sort_keys=True)


def _entity_to_text(entity) -> str:
    lines = [
        f"{entity.kind}: {entity.id.value}",
        f"Status: {entity.status.value}",
        f"Confidence: {entity.confidence.value}",
    ]
    if entity.title:
        lines.append(f"Title: {entity.title}")
    if entity.summary:
        lines.append(f"Summary: {entity.summary}")
    if entity.architecture is not None:
        lines.append("Architecture:")
        for label, value in (
            ("kind", entity.architecture.kind),
            ("classification", entity.architecture.classification),
            ("authority", entity.architecture.authority),
            ("owner", entity.architecture.owner.value if entity.architecture.owner else None),
            ("owns", ", ".join(item.value for item in entity.architecture.owns) or None),
            ("reads", ", ".join(item.value for item in entity.architecture.reads) or None),
            ("writes", ", ".join(item.value for item in entity.architecture.writes) or None),
            ("sends", ", ".join(item.value for item in entity.architecture.sends) or None),
            ("depends_on", ", ".join(item.value for item in entity.architecture.depends_on) or None),
            ("must_not_depend_on", ", ".join(item.value for item in entity.architecture.must_not_depend_on) or None),
            ("runs_in", ", ".join(item.value for item in entity.architecture.runs_in) or None),
            ("producer", ", ".join(item.value for item in entity.architecture.producer) or None),
            ("consumer", ", ".join(item.value for item in entity.architecture.consumer) or None),
            ("validator", ", ".join(item.value for item in entity.architecture.validator) or None),
            ("trust_boundary", entity.architecture.trust_boundary),
        ):
            if value:
                lines.append(f"  {label}: {value}")
    if entity.migration is not None:
        lines.append("Migration:")
        lines.append(f"  legacy_entities: {', '.join(item.value for item in entity.migration.legacy_entities) or '(none)'}")
        lines.append(f"  target_entities: {', '.join(item.value for item in entity.migration.target_entities) or '(none)'}")
    lines.append(f"Source: {entity.source_location.render()}")
    return "\n".join(lines)


def _implementation_to_text(entity_id: str, implementation) -> str:
    lines = [f"Implementation: {entity_id}"]
    if implementation.status is not None:
        lines.append(f"Status: {implementation.status.value}")
    for label, values in (
        ("paths", [path.as_posix() for path in implementation.paths]),
        ("legacy_paths", [path.as_posix() for path in implementation.legacy_paths]),
        ("target_paths", [path.as_posix() for path in implementation.target_paths]),
        ("symbols", list(implementation.symbols)),
        ("messages", list(implementation.messages)),
        ("tests", list(implementation.tests)),
    ):
        if values:
            lines.append(f"{label}:")
            for value in values:
                lines.append(f"  - {value}")
    return "\n".join(lines)


def _migration_to_text(entity_id: str, migration) -> str:
    lines = [f"Migration: {entity_id}"]
    for label, values in (
        ("legacy_entities", [item.value for item in migration.legacy_entities]),
        ("target_entities", [item.value for item in migration.target_entities]),
        ("approved_legacy_callers", [item.value for item in migration.approved_legacy_callers]),
        ("temporary_adapters", [item.value for item in migration.temporary_adapters]),
        ("legacy_symbols", list(migration.legacy_symbols)),
        ("target_symbols", list(migration.target_symbols)),
    ):
        if values:
            lines.append(f"{label}:")
            for value in values:
                lines.append(f"  - {value}")
    if migration.removal_conditions:
        lines.append("removal_conditions:")
        for condition in migration.removal_conditions:
            lines.append(f"  - {condition.predicate.value}: {condition.subject}")
    return "\n".join(lines)


def _json_ready(value):
    if is_dataclass(value):
        return {key: _json_ready(item) for key, item in asdict(value).items()}
    if isinstance(value, Enum):
        return value.value
    if isinstance(value, Path):
        return value.as_posix()
    if isinstance(value, tuple):
        return [_json_ready(item) for item in value]
    if isinstance(value, list):
        return [_json_ready(item) for item in value]
    if isinstance(value, dict):
        return {key: _json_ready(item) for key, item in value.items()}
    return value


if __name__ == "__main__":
    raise SystemExit(main())
