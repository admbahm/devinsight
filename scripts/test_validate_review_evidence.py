#!/usr/bin/env python3

import copy
import importlib.util
import json
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("validate_review_evidence.py")
spec = importlib.util.spec_from_file_location("review_validator", MODULE_PATH)
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)

EXAMPLE_PATH = Path(__file__).parents[1] / ".review" / "examples" / "benchmark-001.json"


class ReviewEvidenceValidatorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.example = json.loads(EXAMPLE_PATH.read_text())

    def test_benchmark_001_is_valid(self):
        self.assertEqual(validator.validate(self.example), [])

    def test_fail_requires_actionable_finding(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["classification"] = "POSSIBLE"

        errors = validator.validate(document)

        self.assertTrue(any("FAIL requires" in error for error in errors))

    def test_pass_rejects_actionable_finding(self):
        document = copy.deepcopy(self.example)
        document["review"]["verdict"] = "PASS"

        errors = validator.validate(document)

        self.assertTrue(any("PASS cannot contain" in error for error in errors))

    def test_schema_rejects_non_string_location_symbol(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["locations"][0]["symbol"] = 42

        errors = validator.validate(document)

        self.assertTrue(
            any("locations[0].symbol" in error and "string" in error for error in errors)
        )

    def test_parser_rejects_non_json_numeric_constants(self):
        for constant in ("NaN", "Infinity", "-Infinity"):
            with self.subTest(constant=constant):
                text = json.dumps(self.example)[:-1] + f', "non_json": {constant}}}'

                with self.assertRaisesRegex(ValueError, "not a valid JSON value"):
                    validator.parse_document(text)

    def test_inconclusive_rejects_actionable_finding(self):
        for classification in ("PROVEN", "HIGH_CONFIDENCE"):
            with self.subTest(classification=classification):
                document = copy.deepcopy(self.example)
                document["review"]["verdict"] = "INCONCLUSIVE"
                document["findings"][0]["classification"] = classification

                errors = validator.validate(document)

                self.assertTrue(
                    any("INCONCLUSIVE cannot contain" in error for error in errors)
                )

    def test_high_confidence_rejects_history_only_evidence(self):
        document = copy.deepcopy(self.example)
        finding = document["findings"][0]
        finding["classification"] = "HIGH_CONFIDENCE"
        finding["evidence"] = [
            {"type": "history", "detail": "A nearby function once had a similar bug."}
        ]

        errors = validator.validate(document)

        self.assertTrue(
            any("HIGH_CONFIDENCE findings require code_path" in error for error in errors)
        )

    def test_high_confidence_rejects_not_run_test_only_evidence(self):
        document = copy.deepcopy(self.example)
        finding = document["findings"][0]
        finding["classification"] = "HIGH_CONFIDENCE"
        finding["evidence"] = [
            {
                "type": "test",
                "detail": "The required runtime was unavailable.",
                "head": "NOT_RUN",
            }
        ]

        errors = validator.validate(document)

        self.assertTrue(
            any("HIGH_CONFIDENCE findings require code_path" in error for error in errors)
        )

    def test_high_confidence_accepts_code_path_evidence(self):
        document = copy.deepcopy(self.example)
        finding = document["findings"][0]
        finding["classification"] = "HIGH_CONFIDENCE"
        finding["evidence"] = [
            {"type": "code_path", "detail": "The failing branch is unconditional."}
        ]

        self.assertEqual(validator.validate(document), [])

    def test_high_confidence_rejects_blank_code_path_detail(self):
        document = copy.deepcopy(self.example)
        finding = document["findings"][0]
        finding["classification"] = "HIGH_CONFIDENCE"
        finding["evidence"] = [{"type": "code_path", "detail": " \t "}]

        errors = validator.validate(document)

        self.assertTrue(
            any(
                "evidence[0].detail" in error
                and "must contain non-whitespace text" in error
                for error in errors
            )
        )

    def test_proven_rejects_unexecuted_evidence_and_blank_semantic_records(self):
        document = copy.deepcopy(self.example)
        finding = document["findings"][0]
        finding["evidence"] = [
            {
                "type": "test",
                "detail": "No test was executed.",
                "base": "NOT_RUN",
                "head": "NOT_RUN",
            }
        ]
        finding["reproduction"] = ["   "]
        finding["attempted_disproof"] = ["\t"]

        errors = validator.validate(document)

        self.assertTrue(
            any(
                "reproduction[0]" in error
                and "must contain non-whitespace text" in error
                for error in errors
            )
        )
        self.assertTrue(
            any(
                "attempted_disproof[0]" in error
                and "must contain non-whitespace text" in error
                for error in errors
            )
        )
        self.assertTrue(
            any(
                "evidence[0]" in error
                and "must show at least one PASS or FAIL" in error
                for error in errors
            )
        )

    def test_actionable_finding_requires_attempted_disproof(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["attempted_disproof"] = []

        errors = validator.validate(document)

        self.assertTrue(any("attempted disproof" in error for error in errors))

    def test_proven_requires_executable_or_unavoidable_evidence(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["evidence"] = [
            {"type": "history", "detail": "A nearby function once had a similar bug."}
        ]

        errors = validator.validate(document)

        self.assertTrue(any("PROVEN findings require" in error for error in errors))

    def test_finding_ids_must_be_unique(self):
        document = copy.deepcopy(self.example)
        document["findings"].append(copy.deepcopy(document["findings"][0]))

        errors = validator.validate(document)

        self.assertTrue(any("must be unique" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
