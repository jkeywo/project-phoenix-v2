"""One-shot offline draft of the first-party German String Table.

Requires an installed Argos en->de package (not a game/runtime dependency).
Existing nonblank German cells are left untouched. `de_source` records the
exact English cell, including any approval brackets; generated German removes
those brackets and carries honest `machine` provenance.
"""

from __future__ import annotations

import argparse
import csv
import json
from pathlib import Path
import re


PARAM = re.compile(r"\{[A-Za-z_][A-Za-z0-9_]*\}")
PROTECTED = re.compile(r"\{[A-Za-z_][A-Za-z0-9_]*\}|https?://[^\s)]+|#[0-9]+")
SENTINEL = re.compile(r"<x[0-9]+>")
DOMAIN_DRAFT = {}
for draft_file in ("german-domain-draft.json", "german-dynasty-draft.json"):
    DOMAIN_DRAFT.update(json.loads(Path(__file__).with_name(draft_file).read_text(encoding="utf-8")))

# Short UI labels provide too little context for the offline model. These
# conservative term choices remain machine-authored draft copy, not human
# editorial approval. Authored proper names and technical identifiers stay as
# the model produced them.
SHORT_UI_TERMS = {
    "Script": "Skript",
    "Save Script": "Skript speichern",
    "Game Master": "Spielleitung",
    "Game Masters": "Spielleitungen",
    "Comms": "Kommunikation",
    "Expand {name}": "{name} aufklappen",
    "Collapse {name}": "{name} zuklappen",
    "Mute {bus}": "{bus} stummschalten",
    "REPAIR TEAMS": "REPARATURTEAMS",
    "VIEWSCREEN": "HAUPTBILDSCHIRM",
    "TORPEDO TUBES": "TORPEDOROHRE",
    "SHIELD FOCUS": "SCHILDFOKUS",
    "HELM": "STEUERUNG",
    "ALERT": "ALARM",
}


def source_copy(english: str) -> str:
    """Approval brackets belong to English review, not German presentation."""
    return english[1:-1] if english.startswith("[") and english.endswith("]") else english


def parameters(text: str) -> list[str]:
    return sorted(PARAM.findall(text))


def protected_translation(text: str, translate) -> str:
    """Mask source tokens before translation and verify the restored result."""
    if text in SHORT_UI_TERMS:
        return SHORT_UI_TERMS[text]
    parts: list[str] = []

    def mask(match: re.Match[str]) -> str:
        parts.append(match.group())
        return f"<x{len(parts) - 1}>"

    masked = PROTECTED.sub(mask, text)
    translated = translate(masked)
    expected = [f"<x{index}>" for index in range(len(parts))]
    if sorted(SENTINEL.findall(translated)) != sorted(expected):
        # Segmenting loses some grammar, but it never silently drops a runtime
        # interpolation or a source link when the model mangles XML sentinels.
        chunks = PROTECTED.split(text)
        tokens = PROTECTED.findall(text)
        translated = "".join(
            translate(chunk) + (tokens[index] if index < len(tokens) else "")
            for index, chunk in enumerate(chunks)
        )
    else:
        for index, value in enumerate(parts):
            translated = translated.replace(f"<x{index}>", value)
    if parameters(translated) != parameters(text):
        raise ValueError(f"placeholder mismatch after translation: {text!r} -> {translated!r}")
    return translated


def generate(source: Path, destination: Path, translate, limit: int | None = None,
             reuse: Path | None = None) -> tuple[int, int]:
    with source.open(encoding="utf-8-sig", newline="") as stream:
        reader = csv.DictReader(stream)
        header = list(reader.fieldnames or [])
        rows = list(reader)
    if not {"id", "context", "en"}.issubset(header):
        raise ValueError("expected a first-party String Table with id, context and en")
    for column in ("de", "de_source", "de_provenance"):
        if column not in header:
            header.append(column)

    reusable = {}
    if reuse is not None:
        with reuse.open(encoding="utf-8-sig", newline="") as stream:
            reusable = {row["id"]: row for row in csv.DictReader(stream)}

    generated = retained = 0
    for row in rows:
        english = row["en"] or ""
        if not english:
            raise ValueError(f"empty English cell: {row['id']}")
        if row.get("de"):
            retained += 1
            continue
        domain = DOMAIN_DRAFT.get(row["id"])
        has_domain_draft = bool(domain and domain["en"] == english)
        previous = reusable.get(row["id"])
        if previous and previous.get("de") and previous.get("de_source") == english \
                and source_copy(english) not in SHORT_UI_TERMS and not has_domain_draft:
            for column in ("de", "de_source", "de_provenance"):
                row[column] = previous.get(column, "")
            retained += 1
            continue
        if limit is not None and generated >= limit:
            continue
        draft = domain["de"] if has_domain_draft else protected_translation(source_copy(english), translate)
        if parameters(draft) != parameters(english):
            raise ValueError(f"draft placeholder mismatch: {row['id']}")
        row["de"] = draft
        row["de_source"] = english
        row["de_provenance"] = "machine"
        generated += 1
        if generated % 100 == 0:
            print(f"translated {generated}/{len(rows) - retained}", flush=True)

    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=header, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    return generated, retained


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--limit", type=int, help="translate only this many new rows for a dry run")
    parser.add_argument("--reuse", type=Path, help="reuse complete translations whose English source still matches")
    args = parser.parse_args()
    from importlib.metadata import version
    import argostranslate.package
    import argostranslate.translate

    if version("argostranslate") != "1.11.0":
        raise SystemExit("Use Argos Translate 1.11.0 for this machine draft")
    packages = [package for package in argostranslate.package.get_installed_packages()
                if package.from_code == "en" and package.to_code == "de"]
    if len(packages) != 1 or packages[0].package_version != "1.3":
        raise SystemExit("Install only Argos en->de model package version 1.3 before generating")
    languages = {language.code: language for language in argostranslate.translate.get_installed_languages()}
    if not {"en", "de"}.issubset(languages):
        raise SystemExit("Install an Argos en->de model before generating the catalogue")
    translator = languages["en"].get_translation(languages["de"])
    if translator is None:
        raise SystemExit("No Argos en->de translation model is installed")
    generated, retained = generate(args.source, args.destination, translator.translate, args.limit, args.reuse)
    print(f"German rows: {generated} machine-generated, {retained} existing retained")


if __name__ == "__main__":
    main()
