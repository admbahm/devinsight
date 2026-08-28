## Claim

What behavior does this change claim to add, fix, or preserve?

> This change claims that ...

## Why

What problem or failure mode motivated the change?

## Invariants

What must remain true after this change?

- [ ] Existing CLI behavior remains compatible unless intentionally changed.
- [ ] Stored JSONL schema remains stable unless intentionally changed.
- [ ] No new panic/hang path is introduced for expected ADB failures.
- [ ] Relevant parsing/filtering/storage/TUI state behavior is covered by tests.

Add or remove invariants as appropriate for this change.

## Failure Hypotheses

What conditions could falsify the claim?

- 

## Verification

Commands or experiments run:

```text
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Additional evidence:

- 

## Known Unknowns

What could not be verified locally or in CI?

- None

## Reviewer Mission

Do not review this PR for comment volume. Attempt to falsify the claim above. Report actionable defects only when supported by `PROVEN` or `HIGH_CONFIDENCE` evidence as defined in `docs/review-protocol.md`.
