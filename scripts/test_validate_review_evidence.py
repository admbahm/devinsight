#!/usr/bin/env python3

import copy
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("validate_review_evidence.py")
spec = importlib.util.spec_from_file_location("review_validator", MODULE_PATH)
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)

EXAMPLE_PATH = Path(__file__).parents[1] / ".review" / "examples" / "benchmark-001.json"


class ReviewEvidenceValidatorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.example = validator.parse_document(EXAMPLE_PATH.read_text())

    def make_pass_review(self):
        document = copy.deepcopy(self.example)
        document["review"]["verdict"] = "PASS"
        document["verification"] = [
            {
                "id": "required-suite",
                "type": "test",
                "required": True,
                "outcome": "PASS",
                "detail": "The required verification suite completed successfully.",
                "command": "python3 scripts/test_validate_review_evidence.py",
            }
        ]
        document["findings"] = []
        return document

    def make_inconclusive_review(self, outcome="NOT_RUN"):
        document = copy.deepcopy(self.example)
        document["review"]["verdict"] = "INCONCLUSIVE"
        document["verification"] = [
            {
                "id": "required-runtime",
                "type": "test",
                "required": True,
                "outcome": outcome,
                "detail": "The required runtime verification could not be completed.",
            }
        ]
        document["findings"] = []
        return document

    def run_persisted_cli(self, root):
        return subprocess.run(
            [sys.executable, str(MODULE_PATH), "--persisted", str(root)],
            check=False,
            capture_output=True,
            text=True,
        )

    def run_single_cli(self, path):
        return subprocess.run(
            [sys.executable, str(MODULE_PATH), str(path)],
            check=False,
            capture_output=True,
            text=True,
        )

    def document_text_with_pr(self, representation):
        text = EXAMPLE_PATH.read_text()
        return text.replace('"pr": 14', f'"pr": {representation}', 1)

    def write_document(self, path, document):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(document))

    def assert_duplicate_keys_rejected(self, text):
        with self.assertRaisesRegex(ValueError, "duplicate object key"):
            validator.parse_document(text)

    def test_benchmark_001_is_valid(self):
        self.assertEqual(validator.validate(self.example), [])

    def test_previous_schema_versions_are_rejected(self):
        for version in ("1", "2"):
            with self.subTest(version=version):
                document = copy.deepcopy(self.example)
                document["schema_version"] = version

                errors = validator.validate(document)

                self.assertTrue(any("schema_version" in error for error in errors))

    def test_schema_requires_review_level_verification_collection(self):
        document = copy.deepcopy(self.example)
        del document["verification"]

        errors = validator.validate(document)

        self.assertTrue(
            any("'verification' is a required property" in error for error in errors)
        )

    def test_fail_requires_actionable_finding(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["classification"] = "POSSIBLE"

        errors = validator.validate(document)

        self.assertTrue(any("FAIL requires" in error for error in errors))

    def test_fail_accepts_admissible_actionable_finding(self):
        self.assertEqual(validator.validate(self.example), [])

    def test_pass_rejects_actionable_finding(self):
        document = self.make_pass_review()
        document["findings"] = [copy.deepcopy(self.example["findings"][0])]

        errors = validator.validate(document)

        self.assertTrue(any("PASS cannot contain" in error for error in errors))

    def test_inconclusive_rejects_actionable_finding(self):
        for classification in ("PROVEN", "HIGH_CONFIDENCE"):
            with self.subTest(classification=classification):
                document = self.make_inconclusive_review()
                finding = copy.deepcopy(self.example["findings"][0])
                finding["classification"] = classification
                if classification == "HIGH_CONFIDENCE":
                    finding["evidence"] = [
                        {
                            "type": "code_path",
                            "qualification": "UNAVOIDABLE",
                            "detail": "The failing path is unconditional.",
                        }
                    ]
                document["findings"] = [finding]

                errors = validator.validate(document)

                self.assertTrue(
                    any("INCONCLUSIVE cannot contain" in error for error in errors)
                )

    def test_pass_requires_review_level_verification(self):
        document = self.make_pass_review()
        document["verification"] = []

        errors = validator.validate(document)

        self.assertTrue(
            any("PASS requires at least one required" in error for error in errors)
        )

    def test_pass_rejects_only_optional_verification(self):
        document = self.make_pass_review()
        document["verification"][0]["required"] = False

        errors = validator.validate(document)

        self.assertTrue(
            any("PASS requires at least one required" in error for error in errors)
        )

    def test_pass_rejects_nonpassing_required_verification(self):
        for outcome in ("FAIL", "NOT_RUN", "UNKNOWN"):
            with self.subTest(outcome=outcome):
                document = self.make_pass_review()
                document["verification"][0]["outcome"] = outcome

                errors = validator.validate(document)

                self.assertTrue(
                    any(
                        "required verification must have outcome PASS" in error
                        for error in errors
                    )
                )

    def test_pass_accepts_completed_required_verification(self):
        self.assertEqual(validator.validate(self.make_pass_review()), [])

    def test_pass_allows_incomplete_optional_verification(self):
        document = self.make_pass_review()
        document["verification"].append(
            {
                "id": "optional-runtime",
                "type": "experiment",
                "required": False,
                "outcome": "NOT_RUN",
                "detail": "An optional experiment was not run.",
            }
        )

        self.assertEqual(validator.validate(document), [])

    def test_inconclusive_requires_incomplete_required_verification(self):
        for outcome in (None, "PASS", "FAIL"):
            with self.subTest(outcome=outcome):
                document = self.make_inconclusive_review()
                if outcome is None:
                    document["verification"] = []
                else:
                    document["verification"][0]["outcome"] = outcome

                errors = validator.validate(document)

                self.assertTrue(
                    any("INCONCLUSIVE requires" in error for error in errors)
                )

    def test_inconclusive_accepts_not_run_or_unknown_required_verification(self):
        for outcome in ("NOT_RUN", "UNKNOWN"):
            with self.subTest(outcome=outcome):
                self.assertEqual(
                    validator.validate(self.make_inconclusive_review(outcome)), []
                )

    def test_inconclusive_rejects_incomplete_optional_verification_only(self):
        document = self.make_inconclusive_review()
        document["verification"][0]["required"] = False

        errors = validator.validate(document)

        self.assertTrue(any("INCONCLUSIVE requires" in error for error in errors))

    def test_verification_ids_must_be_unique(self):
        document = self.make_pass_review()
        document["verification"].append(copy.deepcopy(document["verification"][0]))

        errors = validator.validate(document)

        self.assertTrue(any("verification[1].id" in error for error in errors))

    def test_verification_detail_and_command_must_be_nonblank(self):
        document = self.make_pass_review()
        document["verification"][0]["detail"] = "   "
        document["verification"][0]["command"] = "\t"

        errors = validator.validate(document)

        self.assertTrue(any("verification[0].detail" in error for error in errors))
        self.assertTrue(any("verification[0].command" in error for error in errors))

    def test_high_confidence_rejects_history_only_evidence(self):
        document = copy.deepcopy(self.example)
        finding = document["findings"][0]
        finding["classification"] = "HIGH_CONFIDENCE"
        finding["evidence"] = [
            {"type": "history", "detail": "A nearby function once had a similar bug."}
        ]

        errors = validator.validate(document)

        self.assertTrue(
            any(
                "HIGH_CONFIDENCE findings require STRONG or UNAVOIDABLE code_path"
                in error
                for error in errors
            )
        )

    def test_high_confidence_rejects_observed_code_path_evidence(self):
        document = copy.deepcopy(self.example)
        finding = document["findings"][0]
        finding["classification"] = "HIGH_CONFIDENCE"
        finding["evidence"] = [
            {
                "type": "code_path",
                "qualification": "OBSERVED",
                "detail": "A conditional path was observed.",
            }
        ]

        errors = validator.validate(document)

        self.assertTrue(
            any("require STRONG or UNAVOIDABLE" in error for error in errors)
        )

    def test_high_confidence_accepts_strong_or_unavoidable_code_path_evidence(self):
        for qualification in ("STRONG", "UNAVOIDABLE"):
            with self.subTest(qualification=qualification):
                document = copy.deepcopy(self.example)
                finding = document["findings"][0]
                finding["classification"] = "HIGH_CONFIDENCE"
                finding["evidence"] = [
                    {
                        "type": "code_path",
                        "qualification": qualification,
                        "detail": "The code path strongly supports the finding.",
                    }
                ]

                self.assertEqual(validator.validate(document), [])

    def test_high_confidence_rejects_blank_code_path_detail(self):
        document = copy.deepcopy(self.example)
        finding = document["findings"][0]
        finding["classification"] = "HIGH_CONFIDENCE"
        finding["evidence"] = [
            {
                "type": "code_path",
                "qualification": "STRONG",
                "detail": " \t ",
            }
        ]

        errors = validator.validate(document)

        self.assertTrue(
            any(
                "evidence[0].detail" in error
                and "must contain non-whitespace text" in error
                for error in errors
            )
        )

    def test_code_path_requires_qualification(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["evidence"] = [
            {"type": "code_path", "detail": "No qualification is recorded."}
        ]

        errors = validator.validate(document)

        self.assertTrue(
            any("qualification" in error and "required property" in error for error in errors)
        )

    def test_code_path_rejects_unknown_qualification(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["evidence"] = [
            {
                "type": "code_path",
                "qualification": "SPECULATIVE",
                "detail": "The qualification is outside the contract.",
            }
        ]

        errors = validator.validate(document)

        self.assertTrue(
            any("qualification" in error and "not one of" in error for error in errors)
        )

    def test_non_code_path_rejects_qualification(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["evidence"] = [
            {
                "type": "test",
                "qualification": "UNAVOIDABLE",
                "detail": "The completed test does not use code-path qualification.",
                "head": "FAIL",
            }
        ]

        errors = validator.validate(document)

        self.assertTrue(any("qualification" in error for error in errors))

    def test_proven_rejects_observed_or_strong_code_path_without_execution(self):
        for qualification in ("OBSERVED", "STRONG"):
            with self.subTest(qualification=qualification):
                document = copy.deepcopy(self.example)
                document["findings"][0]["evidence"] = [
                    {
                        "type": "code_path",
                        "qualification": qualification,
                        "detail": "The path does not establish an unavoidable failure.",
                    }
                ]

                errors = validator.validate(document)

                self.assertTrue(
                    any("PROVEN findings require" in error for error in errors)
                )

    def test_proven_accepts_unavoidable_code_path_without_execution(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["evidence"] = [
            {
                "type": "code_path",
                "qualification": "UNAVOIDABLE",
                "detail": "The failing branch is unavoidable for the accepted input.",
            }
        ]

        self.assertEqual(validator.validate(document), [])

    def test_multiple_observed_code_paths_do_not_substitute_for_qualification(self):
        document = copy.deepcopy(self.example)
        finding = document["findings"][0]
        finding["classification"] = "HIGH_CONFIDENCE"
        finding["evidence"] = [
            {
                "type": "code_path",
                "qualification": "OBSERVED",
                "detail": f"Ordinary path {index} was observed.",
            }
            for index in range(3)
        ]

        errors = validator.validate(document)

        self.assertTrue(
            any("require STRONG or UNAVOIDABLE" in error for error in errors)
        )

    def test_unavoidable_code_path_does_not_hide_unexecuted_proven_evidence(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["evidence"] = [
            {
                "type": "code_path",
                "qualification": "UNAVOIDABLE",
                "detail": "The failing path is unavoidable.",
            },
            {
                "type": "test",
                "detail": "The test did not complete.",
                "head": "NOT_RUN",
            },
        ]

        errors = validator.validate(document)

        self.assertTrue(any("evidence[1]" in error for error in errors))

    def test_proven_rejects_executable_evidence_without_outcomes(self):
        for evidence_type in ("test", "command", "experiment"):
            with self.subTest(evidence_type=evidence_type):
                document = copy.deepcopy(self.example)
                document["findings"][0]["evidence"] = [
                    {"type": evidence_type, "detail": "No outcome is recorded."}
                ]

                errors = validator.validate(document)

                self.assertTrue(
                    any("must include at least one structured" in error for error in errors)
                )

    def test_proven_accepts_completed_base_only_outcome(self):
        for outcome in ("PASS", "FAIL"):
            with self.subTest(outcome=outcome):
                document = copy.deepcopy(self.example)
                document["findings"][0]["evidence"] = [
                    {
                        "type": "test",
                        "detail": "The base execution completed.",
                        "base": outcome,
                    }
                ]

                self.assertEqual(validator.validate(document), [])

    def test_proven_accepts_completed_head_only_outcome(self):
        for outcome in ("PASS", "FAIL"):
            with self.subTest(outcome=outcome):
                document = copy.deepcopy(self.example)
                document["findings"][0]["evidence"] = [
                    {
                        "type": "test",
                        "detail": "The head execution completed.",
                        "head": outcome,
                    }
                ]

                self.assertEqual(validator.validate(document), [])

    def test_proven_rejects_only_not_run_or_unknown_outcomes(self):
        for evidence in (
            {"base": "NOT_RUN"},
            {"head": "UNKNOWN"},
            {"base": "NOT_RUN", "head": "UNKNOWN"},
            {"base": "UNKNOWN", "head": "NOT_RUN"},
        ):
            with self.subTest(evidence=evidence):
                document = copy.deepcopy(self.example)
                document["findings"][0]["evidence"] = [
                    {"type": "test", "detail": "No execution completed.", **evidence}
                ]

                errors = validator.validate(document)

                self.assertTrue(
                    any("must include at least one structured" in error for error in errors)
                )

    def test_proven_accepts_valid_executed_evidence(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["evidence"] = [
            {
                "type": "test",
                "detail": "The regression ran on both revisions.",
                "base": "PASS",
                "head": "FAIL",
            }
        ]

        self.assertEqual(validator.validate(document), [])

    def test_proven_mixed_evidence_does_not_hide_unexecuted_record(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["evidence"] = [
            {"type": "test", "detail": "This test did not run.", "head": "NOT_RUN"},
            {"type": "test", "detail": "This test completed.", "head": "FAIL"},
            {
                "type": "code_path",
                "qualification": "UNAVOIDABLE",
                "detail": "The failing path is unconditional.",
            },
        ]

        errors = validator.validate(document)

        self.assertTrue(any("evidence[0]" in error for error in errors))

    def test_proven_requires_executable_or_unavoidable_evidence(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["evidence"] = [
            {"type": "history", "detail": "A nearby function once had a similar bug."}
        ]

        errors = validator.validate(document)

        self.assertTrue(any("PROVEN findings require" in error for error in errors))

    def test_actionable_finding_requires_attempted_disproof(self):
        document = copy.deepcopy(self.example)
        document["findings"][0]["attempted_disproof"] = []

        errors = validator.validate(document)

        self.assertTrue(any("attempted disproof" in error for error in errors))

    def test_finding_ids_must_be_unique(self):
        document = copy.deepcopy(self.example)
        document["findings"].append(copy.deepcopy(document["findings"][0]))

        errors = validator.validate(document)

        self.assertTrue(any("must be unique" in error for error in errors))

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

    def test_parser_preserves_exact_decimal_values(self):
        cases = (
            ("0.99999999999999999", Decimal("0.99999999999999999")),
            ("1.0000000000000001", Decimal("1.0000000000000001")),
            ("9007199254740993e0", Decimal("9007199254740993")),
        )
        for representation, expected in cases:
            with self.subTest(representation=representation):
                document = validator.parse_document(
                    self.document_text_with_pr(representation)
                )

                self.assertIsInstance(document["review"]["pr"], Decimal)
                self.assertEqual(document["review"]["pr"], expected)

    def test_parser_preserves_large_integer_tokens_across_runtime_boundary(self):
        for digits in (4300, 4301, 10000):
            with self.subTest(digits=digits):
                document = validator.parse_document(
                    self.document_text_with_pr("9" * digits)
                )
                value = document["review"]["pr"]

                self.assertIsInstance(value, int)
                self.assertGreaterEqual(value, 10 ** (digits - 1))
                self.assertLess(value, 10**digits)
                self.assertEqual(value % 1_000_000, 999_999)
                self.assertEqual(validator.validate(document), [])

    def test_schema_rejects_exact_fractional_pr_values(self):
        for representation in (
            "0.99999999999999999",
            "1.0000000000000001",
            "1e-1",
            "-1.5",
        ):
            with self.subTest(representation=representation):
                document = validator.parse_document(
                    self.document_text_with_pr(representation)
                )

                errors = validator.validate(document)

                self.assertTrue(
                    any("review.pr" in error and "integer" in error for error in errors)
                )

    def test_schema_accepts_exact_integer_representations(self):
        large_integer = "9" * 512
        for representation in (
            "1",
            "14",
            "1.0",
            "10e-1",
            "9007199254740993e0",
            "1e1000",
            large_integer,
        ):
            with self.subTest(representation=representation):
                document = validator.parse_document(
                    self.document_text_with_pr(representation)
                )

                self.assertEqual(validator.validate(document), [])

    def test_schema_rejects_zero_and_negative_integer_pr_values(self):
        for representation in ("0", "0.0", "-1", "-1e3", "-" + "9" * 4301):
            with self.subTest(representation=representation):
                document = validator.parse_document(
                    self.document_text_with_pr(representation)
                )

                errors = validator.validate(document)

                self.assertTrue(
                    any("review.pr" in error and "minimum" in error for error in errors)
                )

    def test_single_file_cli_rejects_lossy_fractional_pr_values(self):
        for representation in (
            "0.99999999999999999",
            "1.0000000000000001",
        ):
            with self.subTest(representation=representation):
                with tempfile.TemporaryDirectory() as directory:
                    path = Path(directory) / "fractional-pr.json"
                    path.write_text(self.document_text_with_pr(representation))

                    result = self.run_single_cli(path)

                    self.assertEqual(result.returncode, 1)
                    self.assertIn("review.pr", result.stderr)

    def test_persisted_cli_rejects_lossy_fractional_pr_value(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / ".review" / "evidence"
            path = root / "fractional-pr.json"
            path.parent.mkdir(parents=True)
            path.write_text(self.document_text_with_pr("0.99999999999999999"))

            result = self.run_persisted_cli(root)

            self.assertEqual(result.returncode, 1)
            self.assertIn("fractional-pr.json", result.stderr)
            self.assertIn("review.pr", result.stderr)

    def test_single_file_cli_accepts_large_integer_runtime_boundary(self):
        for digits in (4300, 4301):
            with self.subTest(digits=digits):
                with tempfile.TemporaryDirectory() as directory:
                    path = Path(directory) / f"integer-{digits}.json"
                    path.write_text(self.document_text_with_pr("9" * digits))

                    result = self.run_single_cli(path)

                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("VALID review evidence", result.stdout)

    def test_persisted_cli_accepts_large_integer_runtime_boundary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / ".review" / "evidence"
            for digits in (4300, 4301):
                path = root / f"integer-{digits}.json"
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(self.document_text_with_pr("9" * digits))

            result = self.run_persisted_cli(root)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("2 artifact(s)", result.stdout)

    def test_parser_rejects_duplicate_top_level_keys(self):
        self.assert_duplicate_keys_rejected(
            '{"schema_version":"3","schema_version":"2"}'
        )

    def test_parser_rejects_duplicate_review_verdict(self):
        self.assert_duplicate_keys_rejected(
            '{"review":{"verdict":"FAIL","verdict":"PASS"}}'
        )

    def test_parser_rejects_duplicate_verification_outcome(self):
        self.assert_duplicate_keys_rejected(
            '{"verification":[{"outcome":"FAIL","outcome":"PASS"}]}'
        )

    def test_parser_rejects_duplicate_nested_evidence_properties(self):
        self.assert_duplicate_keys_rejected(
            '{"findings":[{"evidence":[{"detail":"first","detail":"second"}]}]}'
        )

    def test_parser_rejects_identical_duplicate_values_at_every_nesting_level(self):
        documents = (
            '{"schema_version":"3","schema_version":"3"}',
            '{"review":{"verdict":"PASS","verdict":"PASS"}}',
            '{"verification":[{"outcome":"PASS","outcome":"PASS"}]}',
            '{"findings":[{"evidence":[{"detail":"same","detail":"same"}]}]}',
        )
        for text in documents:
            with self.subTest(text=text):
                self.assert_duplicate_keys_rejected(text)

    def test_valid_persisted_evidence_passes_cli_discovery(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / ".review" / "evidence"
            self.write_document(root / "valid.json", self.example)

            result = self.run_persisted_cli(root)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("1 artifact(s)", result.stdout)

    def test_missing_persisted_evidence_directory_fails_cli_discovery(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / ".review" / "evidence"

            result = self.run_persisted_cli(root)

            self.assertEqual(result.returncode, 1)
            self.assertIn("directory does not exist", result.stderr)

    def test_invalid_persisted_evidence_fails_cli_discovery(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / ".review" / "evidence"
            self.write_document(root / "invalid.json", {})

            result = self.run_persisted_cli(root)

            self.assertEqual(result.returncode, 1)
            self.assertIn("invalid.json", result.stderr)
            self.assertIn("schema_version", result.stderr)

    def test_one_invalid_persisted_artifact_fails_multiple_artifact_batch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / ".review" / "evidence"
            self.write_document(root / "valid.json", self.example)
            self.write_document(root / "nested" / "invalid.json", {})

            discovered = validator.discover_persisted_evidence(root)
            result = self.run_persisted_cli(root)

            self.assertEqual(
                [path.relative_to(root).as_posix() for path in discovered],
                ["nested/invalid.json", "valid.json"],
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("nested/invalid.json", result.stderr)

    def test_duplicate_key_persisted_artifact_fails_batch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / ".review" / "evidence"
            self.write_document(root / "valid.json", self.example)
            duplicate = json.dumps(self.make_pass_review()).replace(
                '"verdict": "PASS"',
                '"verdict": "FAIL", "verdict": "PASS"',
            )
            duplicate_path = root / "nested" / "duplicate.json"
            duplicate_path.parent.mkdir(parents=True, exist_ok=True)
            duplicate_path.write_text(duplicate)

            result = self.run_persisted_cli(root)

            self.assertEqual(result.returncode, 1)
            self.assertIn("nested/duplicate.json", result.stderr)
            self.assertIn("duplicate object key", result.stderr)


if __name__ == "__main__":
    unittest.main()
