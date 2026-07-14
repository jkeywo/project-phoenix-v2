from pathlib import Path

import pytest

from pasm.core.model import EntityId, SourceLocation


def test_entity_id_accepts_kebab_case() -> None:
    entity_id = EntityId("engineering-station")
    assert entity_id.value == "engineering-station"


@pytest.mark.parametrize(
    "value",
    ["", "Engineering-Station", "engineering_station", "engineering station"],
)
def test_entity_id_rejects_invalid_values(value: str) -> None:
    with pytest.raises(ValueError):
        EntityId(value)


def test_source_location_renders_section() -> None:
    location = SourceLocation(
        path=Path("spec/core/engineering.yaml"),
        line=4,
        column=7,
        section=("entities", "0", "core", "status"),
    )
    assert location.render() == "spec/core/engineering.yaml:4:7 [entities.0.core.status]"
