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
- parsed Iceberg table metadata retained internally during rule evaluation

This keeps rules format-aware without making every rule parse every format.
ArrowDiff reuses the same model to compare field structure, metadata, Parquet
statistics, row-group layout, compression, and estimated scan bytes.

Iceberg metadata JSON is parsed once by the scanner and retained outside the
serialized report model. This avoids reparsing for every rule and prevents a
complete table history from being duplicated into JSON and SARIF reports.

## Extension Point

Rust plugins implement the `Rule` trait and register with `RuleRegistry`.
Python rule packs can use the public Python API and declarative rule files for
configuration-driven checks. Format-specific Rust rule packs can add scanners,
rules, and report metadata while using the same dataset and diagnostic model.

## Development Tooling Boundary

ArrowLint applies lint policy to datasets. It does not replace source-code
formatters, language linters, or repository hook orchestration. This repository
uses `prek` for those development checks and keeps dataset rule selection in
the Rust engine, where the Python and native CLIs share identical semantics.

Future interoperability work should use deterministic producer-consumer
fixtures across Arrow implementations. Each producer should write a stable
fixture, every compatible consumer should scan it, and ArrowLint should verify
that diagnostics and normalized metadata remain consistent. Performance gates
can similarly build on ArrowDiff's estimated scan-cost change rather than
introducing a separate comparison model.
