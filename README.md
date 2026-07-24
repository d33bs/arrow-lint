# arrow-lint

Sharpen your Arrow datasets.

ArrowLint is a fast, opinionated, extensible linter for Apache Arrow datasets
and related formats. It provides quick local feedback, CI-friendly reports, and
a rule system designed for the Arrow ecosystem.

## Features

- Rust core engine with a common dataset model.
- Scanners for Parquet, Arrow IPC, and Feather.
- Python API and Python-first CLI.
- Native Rust CLI for direct integration.
- Built-in rules for schema consistency, metadata, row groups, statistics,
  compression, timestamp portability, dictionary encoding, and small files.
- JSON, SARIF, and human-readable reports.
- YAML declarative rules for simple metadata checks.
- Extension points for additional rule packs.

## Supported Inputs

ArrowLint scans:

- Apache Parquet
- Apache Arrow IPC files
- Feather files

The project also defines extension boundaries for Iceberg, Vortex, Lance, and
DuckDB-focused checks. These formats are represented in the format-pack registry
so rule packs can share a consistent discovery and reporting model.

## Installation

```bash
uv sync
uv run maturin develop
```

## CLI

```bash
uv run arrow-lint lint path/to/dataset --config .arrowlint.yaml
uv run arrow-lint lint path/to/dataset --output json
uv run arrow-lint lint path/to/dataset --output sarif > arrowlint.sarif
uv run arrow-lint rules
uv run arrow-lint formats
```

There is also a native Rust binary:

```bash
cargo run -p arrowlint-cli -- lint path/to/dataset
```

## Python API

```python
from arrow_lint import lint

report = lint("path/to/dataset", config=".arrowlint.yaml")
for diagnostic in report["diagnostics"]:
    print(diagnostic["rule_id"], diagnostic["message"])
```

## Configuration

ArrowLint reads YAML configuration:

```yaml
scan:
  recursive: true

rules:
  min_row_group_rows: 100000
  small_file_bytes: 67108864
  fail_on: error
  disabled: []
  declarative_rule_files:
    - examples/rules/metadata.yaml
```

Declarative rules cover simple metadata checks:

```yaml
rule: missing_crs
severity: warning
applies_to: parquet
check:
  metadata_key: crs
```

## Architecture

```text
Python API / CLI
        |
        v
PyO3 native module       Rust CLI
        |                   |
        +--------+----------+
                 v
          arrowlint-core
        /       |        \
 scanners  dataset model  rules
        \       |        /
              reports
```

The core crate owns scanning, diagnostics, rule execution, and rendering. The
Python package is deliberately thin so plugin authors and data teams get a
comfortable interface without compromising the Rust fast path.

## Extension Strategy

The built-in engine focuses on common Arrow, Feather, and Parquet checks. The
format pack registry defines stable boundaries for specialized rule packs:

- `arrowlint-iceberg` for manifests, snapshots, partition evolution, and delete files.
- `arrowlint-vortex` for array encodings, chunk sizing, and statistics.
- `arrowlint-lance` for fragments, indexes, schema evolution, and vector metadata.
- `arrowlint-duckdb` for Arrow export round-trips, nested types, timestamps, and writer settings.

Rule packs can start in this repository and move to separate packages when their
public APIs are stable.

## Development

```bash
cargo test
uv run maturin develop
uv run pytest
pre-commit run --all-files
```
