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

The deterministic validator is:

```bash
python3 scripts/validate_review_evidence.py path/to/evidence.json
```

The validator enforces review-policy relationships in addition to basic structure:

- `FAIL` requires at least one `PROVEN` or `HIGH_CONFIDENCE` finding.
- `PASS` cannot contain an actionable finding.
- actionable findings require reproduction and attempted-disproof records.
- `PROVEN` findings require executable evidence or an unavoidable code path.
- finding IDs must be unique.

The model may investigate and generate evidence. The validator decides whether that evidence satisfies the contract.

`.review/examples/benchmark-001.json` is the first canonical evidence artifact and captures the controlled PR #14 retention benchmark.

## Review Verdict

A review ends with one of:

- `PASS` — no actionable defect found and required verification passed.
- `FAIL` — at least one `PROVEN` or `HIGH_CONFIDENCE` defect exists.
- `INCONCLUSIVE` — required evidence could not be obtained.

`PASS` does not mean the change is proven perfect. It means the defined review mission did not falsify its claims with the available evidence.

## Baseline Comparison

Whenever practical, a regression test should be checked against both:

- the PR/head branch
- the target/base branch

A finding is stronger when the test passes on the base branch and fails on the proposed change.

## Noise Budget

Prefer three well-supported findings over thirty speculative observations.

Do not emit a style comment unless the change creates measurable maintenance, correctness, safety, or consistency risk.
