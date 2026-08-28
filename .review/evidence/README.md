# Persisted Review Evidence

This directory is the production namespace for persisted machine-readable review
evidence. Every `*.json` file at any depth beneath this directory is validated by
the Review Evidence CI workflow.

Schema files, canonical examples, and deliberately invalid test fixtures live
outside this directory so they are not discovered as persisted evidence.
