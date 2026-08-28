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
