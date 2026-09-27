"""Focused checks for the offline, one-shot German catalogue generator."""

import csv
import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("generate-german.py")
SPEC = importlib.util.spec_from_file_location("generate_german", SCRIPT)
generator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(generator)


class GenerateGermanTests(unittest.TestCase):
    def test_short_ui_terms_keep_imperative_meaning(self):
        self.assertEqual(generator.protected_translation("Save Script", lambda value: value), "Skript speichern")
        self.assertEqual(generator.protected_translation("Expand {name}", lambda value: value), "{name} aufklappen")

    def test_masks_parameters_and_keeps_english_approval_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.csv"
            result = Path(directory) / "result.csv"
            source.write_text("id,context,en\ncount,Rows,[{n} rows checked.]\n", encoding="utf-8")
            count, retained = generator.generate(source, result, lambda value: value.replace("rows checked", "Zeilen geprüft"))
            with result.open(encoding="utf-8", newline="") as stream:
                row = next(csv.DictReader(stream))
            self.assertEqual((count, retained), (1, 0))
            self.assertEqual(row["de"], "{n} Zeilen geprüft.")
            self.assertEqual(row["de_source"], "[{n} rows checked.]")
            self.assertEqual(row["de_provenance"], "machine")

    def test_retains_existing_translation_and_recovers_mangled_tokens(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.csv"
            result = Path(directory) / "result.csv"
            source.write_text("id,context,en,de,de_source,de_provenance\n"
                              "saved,,Save,Gespeichert,Save,human\n"
                              "new,,{count} events on #995,,,\n", encoding="utf-8")
            count, retained = generator.generate(source, result, lambda value: value.replace("<x0>", "?"))
            with result.open(encoding="utf-8", newline="") as stream:
                rows = list(csv.DictReader(stream))
            self.assertEqual((count, retained), (1, 1))
            self.assertEqual((rows[0]["de"], rows[0]["de_provenance"]), ("Gespeichert", "human"))
            self.assertEqual(rows[1]["de"], "{count} events on #995")

    def test_reuses_only_matching_machine_draft_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.csv"
            prior = Path(directory) / "prior.csv"
            result = Path(directory) / "result.csv"
            source.write_text("id,context,en\na,,New English\nb,,Same English\n", encoding="utf-8")
            prior.write_text("id,context,en,de,de_source,de_provenance\n"
                             "a,,Old English,Alt,Old English,machine\n"
                             "b,,Same English,Gleich,Same English,machine\n", encoding="utf-8")
            count, retained = generator.generate(source, result, lambda _: "Neu", reuse=prior)
            with result.open(encoding="utf-8", newline="") as stream:
                rows = list(csv.DictReader(stream))
            self.assertEqual((count, retained), (1, 1))
            self.assertEqual([row["de"] for row in rows], ["Neu", "Gleich"])

    def test_domain_draft_is_bound_to_exact_english_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.csv"
            result = Path(directory) / "result.csv"
            source.write_text("id,context,en\nworld.alliance_convoy.transport.bell,,Bell\n",
                              encoding="utf-8")
            generator.generate(source, result, lambda _: "Modell")
            with result.open(encoding="utf-8", newline="") as stream:
                self.assertEqual(next(csv.DictReader(stream))["de"], "Bell")
            source.write_text("id,context,en\nworld.alliance_convoy.transport.bell,,Bell II\n",
                              encoding="utf-8")
            generator.generate(source, result, lambda _: "Modell")
            with result.open(encoding="utf-8", newline="") as stream:
                self.assertEqual(next(csv.DictReader(stream))["de"], "Modell")

    def test_dynasty_correction_is_bound_to_exact_english_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.csv"
            result = Path(directory) / "result.csv"
            entry = generator.DOMAIN_DRAFT["dynasty.boost.toggle"]
            source.write_text(f"id,context,en\ndynasty.boost.toggle,,{entry['en']}\n", encoding="utf-8")
            generator.generate(source, result, lambda _: "Modell")
            with result.open(encoding="utf-8", newline="") as stream:
                self.assertEqual(next(csv.DictReader(stream))["de"], entry["de"])
            source.write_text("id,context,en\ndynasty.boost.toggle,,[Revised boost]\n",
                              encoding="utf-8")
            generator.generate(source, result, lambda _: "Modell")
            with result.open(encoding="utf-8", newline="") as stream:
                self.assertEqual(next(csv.DictReader(stream))["de"], "Modell")


if __name__ == "__main__":
    unittest.main()
