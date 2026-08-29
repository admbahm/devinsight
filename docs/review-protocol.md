# Agentic Review Protocol

DevInsight reviews changes as falsifiable claims about system behavior.

## Review Flow

### 1. Establish intent

Summarize the change in one sentence:

> This change claims that ...

Extract explicit requirements from the PR, issue, tests, documentation, and existing behavior. List unknown intent instead of inventing it.

### 2. Build a change map

Identify:

- changed files and functions
- direct callers/callees
- affected state or data boundaries
- existing tests
- externally visible behavior
- compatibility constraints

### 3. Generate hypotheses

For each meaningful behavior change, ask what would make the claim false.

Typical categories:

- malformed or unexpected input
- boundary values
- stale or missing state
- I/O and subprocess failure
- concurrency or ordering
- retry/repetition
- persistence corruption or schema drift
- backward compatibility
- resource exhaustion
- UI state diverging from underlying data

### 4. Investigate

Use the cheapest reliable evidence first:

1. code-path inspection
2. existing tests
3. targeted new regression test
4. broader test suite
5. runtime reproduction where available

Record commands and outcomes.

### 5. Challenge the finding

Before reporting a defect, attempt to invalidate it:

- Is the path reachable?
- Is there a guard elsewhere?
- Is the behavior intentional?
- Does an existing contract allow it?
- Does the test reproduce on `main` too?
- Is the test itself valid?

### 6. Produce the evidence report

Use this structure for each actionable finding:

```yaml
id: DI-REV-001
classification: PROVEN | HIGH_CONFIDENCE
severity: critical | high | medium | low
claim: "..."
affected_behavior: "..."
locations:
  - path: src/example.rs
    symbol: example_function
evidence:
  - type: test | command | code_path | history | experiment
    qualification: OBSERVED | STRONG | UNAVOIDABLE  # code_path only
    detail: "..."
expected: "..."
observed: "..."
reproduction:
  - "..."
attempted_disproof:
  - "..."
remaining_uncertainty: "none" | "..."
```

Non-actionable risks may be recorded as `POSSIBLE`, but they must be clearly separated from defects.

## Machine-Readable Evidence

A completed automated review must also be serializable as JSON conforming to `.review/review-evidence.schema.json`.

Schema version `3` separates two evidence scopes:

- `findings[].evidence` supports the classification of an individual finding.
- top-level `verification` records whether the review's required verification
  completed.

Each review-level verification item contains a stable `id`, a `type`, whether it
is `required`, a structured `outcome`, and a non-blank `detail`. A command may be
recorded when applicable. Verification outcomes are `PASS`, `FAIL`, `NOT_RUN`,
or `UNKNOWN`; only `PASS` is compatible with a required verification item in a
passing review, while `NOT_RUN` and `UNKNOWN` represent incomplete verification.

The deterministic validator is:

```bash
python3 -m pip install -r scripts/requirements-review-evidence.txt
python3 scripts/validate_review_evidence.py path/to/evidence.json
python3 scripts/validate_review_evidence.py --persisted .review/evidence
```

The validator uses `.review/review-evidence.schema.json` as the structural authority
and enforces review-policy relationships in addition to that structure:

- JSON object-member names must be unique at every nesting level; duplicates
  are rejected during parsing even when the repeated values are identical.
- `FAIL` requires at least one `PROVEN` or `HIGH_CONFIDENCE` finding.
- any actionable finding requires a `FAIL` verdict.
- actionable findings require non-blank reproduction and attempted-disproof records.
- every `code_path` evidence record requires one structured qualification:
  - `OBSERVED` records an ordinary path and does not independently admit an
    actionable classification;
  - `STRONG` can support `HIGH_CONFIDENCE` but not `PROVEN`;
  - `UNAVOIDABLE` can support `HIGH_CONFIDENCE` or `PROVEN`.
- `HIGH_CONFIDENCE` findings require `STRONG` or `UNAVOIDABLE` `code_path`
  evidence.
- `PROVEN` findings require executed evidence or `UNAVOIDABLE` `code_path`
  evidence.
- finding-level executable evidence never implies execution from prose; each
  `test`, `command`, or `experiment` record must include at least one structured
  `base` or `head` outcome of `PASS` or `FAIL` to support `PROVEN`.
- `PASS` requires at least one required review-level verification item and every
  required verification outcome must be `PASS`.
- `INCONCLUSIVE` requires at least one required review-level verification item
  with outcome `NOT_RUN` or `UNKNOWN`.
- review-level verification IDs and finding IDs must be unique within their
  respective collections.

The model may investigate and generate evidence. The validator decides whether that evidence satisfies the contract.

`.review/evidence/**/*.json` is the explicit namespace for persisted review
evidence. CI recursively validates every JSON artifact in that namespace.
Schema files, canonical examples, configuration, and deliberately invalid test
fixtures live outside it and are not discovered as persisted evidence.

`.review/examples/benchmark-001.json` is the first canonical evidence fixture,
captures the controlled PR #14 retention benchmark, and is validated separately
from persisted evidence.

## Review Verdict

A review ends with one of:

- `PASS` — no actionable defect found, required verification is represented,
  and every required verification item passed.
- `FAIL` — at least one `PROVEN` or `HIGH_CONFIDENCE` defect exists.
- `INCONCLUSIVE` — no actionable defect is admitted and at least one required
  verification item was not run or has an unknown outcome.

`PASS` does not mean the change is proven perfect. It means the defined review mission did not falsify its claims with the available evidence.

## Baseline Comparison

Whenever practical, a regression test should be checked against both:

- the PR/head branch
- the target/base branch

A finding is stronger when the test passes on the base branch and fails on the proposed change.

## Noise Budget

Prefer three well-supported findings over thirty speculative observations.

Do not emit a style comment unless the change creates measurable maintenance, correctness, safety, or consistency risk.
