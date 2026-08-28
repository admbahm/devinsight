#!/usr/bin/env python3

import json
import re
import sys
from pathlib import Path

ACTIONABLE = {"PROVEN", "HIGH_CONFIDENCE"}
CLASSIFICATIONS = ACTIONABLE | {"POSSIBLE", "STYLE"}
SEVERITIES = {"critical", "high", "medium", "low"}
VERDICTS = {"PASS", "FAIL", "INCONCLUSIVE"}
EVIDENCE_TYPES = {"test", "command", "code_path", "history", "experiment"}
OUTCOMES = {"PASS", "FAIL", "NOT_RUN", "UNKNOWN"}


def fail(errors, path, message):
    errors.append(f"{path}: {message}")


def require_string(errors, obj, key, path, allow_empty=False):
    value = obj.get(key)
    if not isinstance(value, str) or (not allow_empty and not value.strip()):
        fail(errors, f"{path}.{key}", "must be a non-empty string")
    return value


def require_list(errors, obj, key, path, min_items=0):
    value = obj.get(key)
    if not isinstance(value, list):
        fail(errors, f"{path}.{key}", "must be an array")
        return []
    if len(value) < min_items:
        fail(errors, f"{path}.{key}", f"must contain at least {min_items} item(s)")
    return value


def validate_finding(errors, finding, index):
    path = f"findings[{index}]"
    if not isinstance(finding, dict):
        fail(errors, path, "must be an object")
        return None

    finding_id = require_string(errors, finding, "id", path)
    if isinstance(finding_id, str) and not re.fullmatch(r"DI-REV-\d+", finding_id):
        fail(errors, f"{path}.id", "must match DI-REV-<number>")

    classification = finding.get("classification")
    if classification not in CLASSIFICATIONS:
        fail(errors, f"{path}.classification", f"must be one of {sorted(CLASSIFICATIONS)}")

    severity = finding.get("severity")
    if severity not in SEVERITIES:
        fail(errors, f"{path}.severity", f"must be one of {sorted(SEVERITIES)}")

    for key in ("claim", "affected_behavior", "expected", "observed"):
        require_string(errors, finding, key, path)
    require_string(errors, finding, "remaining_uncertainty", path, allow_empty=True)

    locations = require_list(errors, finding, "locations", path, min_items=1)
    for location_index, location in enumerate(locations):
        location_path = f"{path}.locations[{location_index}]"
        if not isinstance(location, dict):
            fail(errors, location_path, "must be an object")
            continue
        require_string(errors, location, "path", location_path)

    evidence = require_list(errors, finding, "evidence", path, min_items=1)
    for evidence_index, item in enumerate(evidence):
        evidence_path = f"{path}.evidence[{evidence_index}]"
        if not isinstance(item, dict):
            fail(errors, evidence_path, "must be an object")
            continue
        if item.get("type") not in EVIDENCE_TYPES:
            fail(errors, f"{evidence_path}.type", f"must be one of {sorted(EVIDENCE_TYPES)}")
        require_string(errors, item, "detail", evidence_path)
        for outcome_key in ("base", "head"):
            if outcome_key in item and item[outcome_key] not in OUTCOMES:
                fail(errors, f"{evidence_path}.{outcome_key}", f"must be one of {sorted(OUTCOMES)}")

    reproduction = require_list(errors, finding, "reproduction", path)
    attempted_disproof = require_list(errors, finding, "attempted_disproof", path)

    if classification in ACTIONABLE:
        if not reproduction:
            fail(errors, f"{path}.reproduction", "actionable findings require reproduction steps or an executable reasoning path")
        if not attempted_disproof:
            fail(errors, f"{path}.attempted_disproof", "actionable findings require at least one attempted disproof")

    if classification == "PROVEN":
        executable = any(
            isinstance(item, dict) and item.get("type") in {"test", "command", "experiment"}
            for item in evidence
        )
        unavoidable = any(
            isinstance(item, dict) and item.get("type") == "code_path"
            for item in evidence
        )
        if not (executable or unavoidable):
            fail(errors, f"{path}.evidence", "PROVEN findings require executable evidence or an unavoidable code path")

    return classification


def validate(document):
    errors = []
    if not isinstance(document, dict):
        return ["root: must be an object"]

    if document.get("schema_version") != "1":
        fail(errors, "schema_version", 'must equal "1"')

    review = document.get("review")
    if not isinstance(review, dict):
        fail(errors, "review", "must be an object")
        review = {}

    require_string(errors, review, "repository", "review")
    pr = review.get("pr")
    if not isinstance(pr, int) or isinstance(pr, bool) or pr < 1:
        fail(errors, "review.pr", "must be a positive integer")
    for key in ("base_sha", "head_sha"):
        value = require_string(errors, review, key, "review")
        if isinstance(value, str) and len(value) < 7:
            fail(errors, f"review.{key}", "must contain at least 7 characters")

    verdict = review.get("verdict")
    if verdict not in VERDICTS:
        fail(errors, "review.verdict", f"must be one of {sorted(VERDICTS)}")

    findings = require_list(errors, document, "findings", "root")
    seen_ids = set()
    classifications = []
    for index, finding in enumerate(findings):
        classification = validate_finding(errors, finding, index)
        if classification:
            classifications.append(classification)
        if isinstance(finding, dict):
            finding_id = finding.get("id")
            if isinstance(finding_id, str):
                if finding_id in seen_ids:
                    fail(errors, f"findings[{index}].id", "must be unique")
                seen_ids.add(finding_id)

    actionable_count = sum(c in ACTIONABLE for c in classifications)
    if verdict == "FAIL" and actionable_count == 0:
        fail(errors, "review.verdict", "FAIL requires at least one PROVEN or HIGH_CONFIDENCE finding")
    if verdict == "PASS" and actionable_count > 0:
        fail(errors, "review.verdict", "PASS cannot contain PROVEN or HIGH_CONFIDENCE findings")

    return errors


def main():
    if len(sys.argv) != 2:
        print("usage: validate_review_evidence.py <evidence.json>", file=sys.stderr)
        return 2

    path = Path(sys.argv[1])
    try:
        document = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        print(f"INVALID: {exc}", file=sys.stderr)
        return 1

    errors = validate(document)
    if errors:
        print("INVALID review evidence:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print("VALID review evidence")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
