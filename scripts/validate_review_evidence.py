#!/usr/bin/env python3

import json
import sys
from pathlib import Path

from jsonschema.validators import validator_for

ACTIONABLE = {"PROVEN", "HIGH_CONFIDENCE"}
EXECUTABLE_EVIDENCE = {"test", "command", "experiment"}
EXECUTED_OUTCOMES = {"PASS", "FAIL"}
SCHEMA_PATH = Path(__file__).parents[1] / ".review" / "review-evidence.schema.json"
PROVEN_EXECUTION_ERROR = (
    "PROVEN executable evidence with structured outcomes must show at least one "
    "PASS or FAIL execution"
)


def fail(errors, path, message):
    errors.append(f"{path}: {message}")


def reject_non_json_constant(value):
    raise ValueError(f"{value} is not a valid JSON value")


def parse_document(text):
    return json.loads(text, parse_constant=reject_non_json_constant)


def format_json_path(parts):
    path = "root"
    for part in parts:
        if isinstance(part, int):
            path += f"[{part}]"
        else:
            path += f".{part}"
    return path


def load_structural_validator():
    schema = json.loads(SCHEMA_PATH.read_text())
    validator_class = validator_for(schema)
    validator_class.check_schema(schema)
    return validator_class(schema)


STRUCTURAL_VALIDATOR = load_structural_validator()


def validate_structure(document):
    schema_errors = sorted(
        STRUCTURAL_VALIDATOR.iter_errors(document),
        key=lambda error: tuple(str(part) for part in error.absolute_path),
    )
    return [
        f"{format_json_path(error.absolute_path)}: {error.message}"
        for error in schema_errors
    ]


def require_nonblank(errors, value, path):
    if not value.strip():
        fail(errors, path, "must contain non-whitespace text")


def validate_semantic_strings(errors, document):
    review = document["review"]
    for key in ("repository", "base_sha", "head_sha"):
        require_nonblank(errors, review[key], f"review.{key}")

    for finding_index, finding in enumerate(document["findings"]):
        finding_path = f"findings[{finding_index}]"
        for key in (
            "claim",
            "affected_behavior",
            "expected",
            "observed",
            "remaining_uncertainty",
        ):
            require_nonblank(errors, finding[key], f"{finding_path}.{key}")

        for location_index, location in enumerate(finding["locations"]):
            location_path = f"{finding_path}.locations[{location_index}]"
            require_nonblank(errors, location["path"], f"{location_path}.path")
            if "symbol" in location:
                require_nonblank(errors, location["symbol"], f"{location_path}.symbol")

        for evidence_index, item in enumerate(finding["evidence"]):
            require_nonblank(
                errors,
                item["detail"],
                f"{finding_path}.evidence[{evidence_index}].detail",
            )

        for key in ("reproduction", "attempted_disproof"):
            for record_index, record in enumerate(finding[key]):
                require_nonblank(
                    errors,
                    record,
                    f"{finding_path}.{key}[{record_index}]",
                )


def validate_finding_policy(errors, finding, index):
    path = f"findings[{index}]"
    classification = finding["classification"]

    if classification in ACTIONABLE:
        if not finding["reproduction"]:
            fail(
                errors,
                f"{path}.reproduction",
                "actionable findings require reproduction steps or an executable reasoning path",
            )
        if not finding["attempted_disproof"]:
            fail(
                errors,
                f"{path}.attempted_disproof",
                "actionable findings require at least one attempted disproof",
            )

    if classification == "HIGH_CONFIDENCE":
        has_code_path = any(
            item["type"] == "code_path" for item in finding["evidence"]
        )
        if not has_code_path:
            fail(
                errors,
                f"{path}.evidence",
                "HIGH_CONFIDENCE findings require code_path evidence",
            )
        return

    if classification != "PROVEN":
        return

    executable = False
    unavoidable = False
    for evidence_index, item in enumerate(finding["evidence"]):
        evidence_type = item["type"]
        if evidence_type == "code_path":
            unavoidable = True
        if evidence_type not in EXECUTABLE_EVIDENCE:
            continue

        outcomes = [item[key] for key in ("base", "head") if key in item]
        explicitly_unexecuted = outcomes and not any(
            outcome in EXECUTED_OUTCOMES for outcome in outcomes
        )
        if explicitly_unexecuted:
            fail(
                errors,
                f"{path}.evidence[{evidence_index}]",
                PROVEN_EXECUTION_ERROR,
            )
        else:
            executable = True

    if not (executable or unavoidable):
        fail(
            errors,
            f"{path}.evidence",
            "PROVEN findings require executed evidence or an unavoidable code path",
        )


def validate(document):
    structural_errors = validate_structure(document)
    if structural_errors:
        return structural_errors

    errors = []
    validate_semantic_strings(errors, document)

    seen_ids = set()
    classifications = []
    for index, finding in enumerate(document["findings"]):
        validate_finding_policy(errors, finding, index)
        classifications.append(finding["classification"])

        finding_id = finding["id"]
        if finding_id in seen_ids:
            fail(errors, f"findings[{index}].id", "must be unique")
        seen_ids.add(finding_id)

    actionable_count = sum(c in ACTIONABLE for c in classifications)
    verdict = document["review"]["verdict"]
    if verdict == "FAIL" and actionable_count == 0:
        fail(
            errors,
            "review.verdict",
            "FAIL requires at least one PROVEN or HIGH_CONFIDENCE finding",
        )
    if verdict == "PASS" and actionable_count > 0:
        fail(
            errors,
            "review.verdict",
            "PASS cannot contain actionable findings; actionable findings require FAIL",
        )
    if verdict == "INCONCLUSIVE" and actionable_count > 0:
        fail(
            errors,
            "review.verdict",
            "INCONCLUSIVE cannot contain actionable findings; actionable findings require FAIL",
        )

    return errors


def main():
    if len(sys.argv) != 2:
        print("usage: validate_review_evidence.py <evidence.json>", file=sys.stderr)
        return 2

    path = Path(sys.argv[1])
    try:
        document = parse_document(path.read_text())
    except (OSError, ValueError) as exc:
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
