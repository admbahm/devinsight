#!/usr/bin/env python3

import json
import sys
from pathlib import Path

from jsonschema.validators import validator_for

ACTIONABLE = {"PROVEN", "HIGH_CONFIDENCE"}
EXECUTABLE_EVIDENCE = {"test", "command", "experiment"}
EXECUTED_OUTCOMES = {"PASS", "FAIL"}
HIGH_CONFIDENCE_CODE_PATH_QUALIFICATIONS = {"STRONG", "UNAVOIDABLE"}
INCOMPLETE_VERIFICATION_OUTCOMES = {"NOT_RUN", "UNKNOWN"}
REPOSITORY_ROOT = Path(__file__).parents[1]
SCHEMA_PATH = REPOSITORY_ROOT / ".review" / "review-evidence.schema.json"
PERSISTED_EVIDENCE_ROOT = REPOSITORY_ROOT / ".review" / "evidence"
PROVEN_EXECUTION_ERROR = (
    "PROVEN executable evidence must include at least one structured PASS or FAIL "
    "outcome"
)


def fail(errors, path, message):
    errors.append(f"{path}: {message}")


def reject_non_json_constant(value):
    raise ValueError(f"{value} is not a valid JSON value")


def reject_duplicate_object_keys(pairs):
    document = {}
    for key, value in pairs:
        if key in document:
            raise ValueError(f"duplicate object key: {key!r}")
        document[key] = value
    return document


def parse_document(text):
    return json.loads(
        text,
        parse_constant=reject_non_json_constant,
        object_pairs_hook=reject_duplicate_object_keys,
    )


def format_json_path(parts):
    path = "root"
    for part in parts:
        if isinstance(part, int):
            path += f"[{part}]"
        else:
            path += f".{part}"
    return path


def load_structural_validator():
    schema = parse_document(SCHEMA_PATH.read_text())
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

    for verification_index, item in enumerate(document["verification"]):
        verification_path = f"verification[{verification_index}]"
        for key in ("id", "detail"):
            require_nonblank(errors, item[key], f"{verification_path}.{key}")
        if "command" in item:
            require_nonblank(
                errors, item["command"], f"{verification_path}.command"
            )

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
        has_qualifying_code_path = any(
            item["type"] == "code_path"
            and item["qualification"] in HIGH_CONFIDENCE_CODE_PATH_QUALIFICATIONS
            for item in finding["evidence"]
        )
        if not has_qualifying_code_path:
            fail(
                errors,
                f"{path}.evidence",
                "HIGH_CONFIDENCE findings require STRONG or UNAVOIDABLE code_path evidence",
            )
        return

    if classification != "PROVEN":
        return

    executable = False
    unavoidable = False
    for evidence_index, item in enumerate(finding["evidence"]):
        evidence_type = item["type"]
        if (
            evidence_type == "code_path"
            and item["qualification"] == "UNAVOIDABLE"
        ):
            unavoidable = True
        if evidence_type not in EXECUTABLE_EVIDENCE:
            continue

        outcomes = [item[key] for key in ("base", "head") if key in item]
        has_completed_outcome = any(
            outcome in EXECUTED_OUTCOMES for outcome in outcomes
        )
        if not has_completed_outcome:
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


def validate_verification_policy(errors, document):
    seen_ids = set()
    required_items = []
    for index, item in enumerate(document["verification"]):
        verification_id = item["id"]
        if verification_id in seen_ids:
            fail(errors, f"verification[{index}].id", "must be unique")
        seen_ids.add(verification_id)
        if item["required"]:
            required_items.append((index, item))

    verdict = document["review"]["verdict"]
    if verdict == "PASS":
        if not required_items:
            fail(
                errors,
                "verification",
                "PASS requires at least one required review-level verification item",
            )
        for index, item in required_items:
            if item["outcome"] != "PASS":
                fail(
                    errors,
                    f"verification[{index}].outcome",
                    "required verification must have outcome PASS for a PASS verdict",
                )

    if verdict == "INCONCLUSIVE" and not any(
        item["outcome"] in INCOMPLETE_VERIFICATION_OUTCOMES
        for _, item in required_items
    ):
        fail(
            errors,
            "verification",
            "INCONCLUSIVE requires a required verification item with outcome NOT_RUN or UNKNOWN",
        )


def validate(document):
    structural_errors = validate_structure(document)
    if structural_errors:
        return structural_errors

    errors = []
    validate_semantic_strings(errors, document)
    validate_verification_policy(errors, document)

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


def validate_file(path):
    try:
        document = parse_document(path.read_text())
    except (OSError, ValueError) as exc:
        return [f"INVALID: {exc}"]
    return validate(document)


def discover_persisted_evidence(root):
    return sorted(path for path in root.rglob("*.json") if path.is_file())


def validate_persisted_evidence(root):
    if not root.is_dir():
        return [], [f"{root}: persisted evidence directory does not exist"]

    paths = discover_persisted_evidence(root)
    errors = []
    for path in paths:
        for error in validate_file(path):
            errors.append(f"{path}: {error}")
    return paths, errors


def main():
    args = sys.argv[1:]
    if args and args[0] == "--persisted":
        if len(args) > 2:
            print(
                "usage: validate_review_evidence.py --persisted [directory]",
                file=sys.stderr,
            )
            return 2
        root = Path(args[1]) if len(args) == 2 else PERSISTED_EVIDENCE_ROOT
        paths, errors = validate_persisted_evidence(root)
        if errors:
            print("INVALID persisted review evidence:", file=sys.stderr)
            for error in errors:
                print(f"- {error}", file=sys.stderr)
            return 1
        print(f"VALID persisted review evidence ({len(paths)} artifact(s))")
        return 0

    if len(args) != 1:
        print(
            "usage: validate_review_evidence.py <evidence.json>", file=sys.stderr
        )
        return 2

    errors = validate_file(Path(args[0]))
    if errors:
        print("INVALID review evidence:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print("VALID review evidence")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
