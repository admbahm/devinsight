# DevInsight Agent Contract

## Engineering Doctrine

1. Understand the system.
2. Discover its limits.
3. Build what ought to exist next.
4. Prove that it works.

## Review Mission

When reviewing a change, do not optimize for comment volume. Optimize for justified findings.

Treat every change as a claim about system behavior. Attempt to falsify that claim before accepting it.

A review is incomplete until the reviewer has:

1. Read the diff and identified the intended behavior change.
2. Inspected relevant callers, tests, state transitions, persistence boundaries, and error paths.
3. Formed explicit failure hypotheses.
4. Gathered evidence with static inspection and, where practical, executable tests.
5. Tried to disprove each proposed finding.
6. Reported only findings whose confidence is supported by evidence.

## Finding Classes

- `PROVEN` — reproduced with executable evidence or directly demonstrated by an unavoidable code path.
- `HIGH_CONFIDENCE` — strong code-path evidence exists, but runtime reproduction is impractical or unavailable.
- `POSSIBLE` — plausible risk with incomplete evidence. Do not present as a defect.
- `STYLE` — non-functional preference. Do not block acceptance.

Only `PROVEN` and `HIGH_CONFIDENCE` findings should be treated as actionable defects by default.

## Evidence Requirements

Every actionable finding must contain:

- claim
- affected behavior
- relevant files/functions
- evidence
- reproduction steps or reasoning path
- expected behavior
- observed or inferred behavior
- confidence class
- severity rationale
- attempted disproof

Prefer a failing test over a speculative comment.

## Machine-Readable Verdict Evidence

Automated review artifacts use schema version `3` and record review-level
verification separately from finding-level evidence.

- Finding-level `test`, `command`, and `experiment` evidence counts as executed
  only when `base` or `head` records a completed `PASS` or `FAIL` outcome.
- Finding-level `code_path` evidence records a structured qualification:
  `OBSERVED` does not independently admit an actionable finding, `STRONG` can
  support `HIGH_CONFIDENCE`, and `UNAVOIDABLE` can support either actionable
  classification.
- `FAIL` requires at least one admissible `PROVEN` or `HIGH_CONFIDENCE` finding.
- `PASS` requires no actionable findings, at least one required review-level
  verification item, and `PASS` outcomes for every required verification item.
- `INCONCLUSIVE` requires no actionable findings and at least one required
  review-level verification item with a `NOT_RUN` or `UNKNOWN` outcome.

Persisted review artifacts belong under `.review/evidence/**/*.json`; canonical
examples and deliberately invalid fixtures are not persisted evidence.

## Review Boundaries

Reviewers may:

- inspect repository history and code
- search callers and related behavior
- run formatting, linting, builds, and tests
- create temporary or committed regression tests when asked
- propose patches

Reviewers must not:

- merge changes
- approve their own changes
- weaken tests merely to obtain a passing build
- silently change requirements
- report speculation as fact
- treat stylistic preference as correctness

## DevInsight-Specific Invariants

Unless a change explicitly states otherwise:

- DevInsight remains local-first and terminal-first.
- Android `adb logcat` is the supported source during the current reliability pass.
- Existing CLI behavior should remain backward compatible unless intentionally changed.
- Stored JSONL fields remain stable: `timestamp`, `level`, `tag`, `message`, `device_id`.
- TUI interaction must not corrupt or drop accepted log records because of rendering state.
- Failure to connect to ADB should fail clearly rather than panic or hang indefinitely.
- Changes affecting parsing, filtering, storage, or state transitions should include focused tests.

## Standard Verification

Run, at minimum:

```bash
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

If a command cannot run in the current environment, report that limitation explicitly.
