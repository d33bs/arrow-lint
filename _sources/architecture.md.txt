# Architecture

ArrowLint has a Rust-first core with a Python-first user experience.

```text
CLI / Python API
        |
        v
PyO3 native module
        |
        v
Rust core engine
        |
  +-----+------+------+
  |            |      |
scanners   dataset  rules
  |         model    |
  +-----+------+------+
        |
        v
 lint reports: text, JSON, SARIF
 dataset diffs: text, JSON
```

## Core Boundaries

- `arrowlint-core` owns scanning, the common dataset model, rules, dataset
  comparison, plugin traits, declarative rule evaluation, and report rendering.
- `arrowlint-python` exposes the Rust core through PyO3.
- `arrowlint-cli` provides a native binary for direct Rust integration.
- `src/arrow_lint` provides the Python API and user-facing `arrow-lint` CLI.

## Dataset Model

Every scanner returns the same model:

- files
- format
- size and row counts
- Arrow schema fields and metadata
- Parquet row groups and column chunk metadata where available

This keeps rules format-aware without making every rule parse every format.
ArrowDiff reuses the same model to compare field structure, metadata, Parquet
statistics, row-group layout, compression, and estimated scan bytes.

## Extension Point

Rust plugins implement the `Rule` trait and register with `RuleRegistry`.
Python rule packs can use the public Python API and declarative rule files for
configuration-driven checks. Format-specific Rust rule packs can add scanners,
rules, and report metadata while using the same dataset and diagnostic model.
