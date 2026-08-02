# arrow-lint

Sharpen your Arrow datasets.

arrow-lint is a fast, opinionated, extensible linter for Apache Arrow datasets
and related formats. It provides quick local feedback, CI-friendly reports, and
a rule system designed for the Arrow ecosystem.

## Features

- Rust core engine with a common dataset model.
- Scanners for Parquet, Arrow IPC files and streams, Feather, and Vortex files.
- Iceberg table metadata JSON scanning, including gzip-compressed metadata.
- Lance table-manifest scanning for local `*.lance` dataset directories.
- Vortex postscript and footer metadata validation.
- Python API and Python-first CLI.
- Native Rust CLI for direct integration.
- ArrowDiff metadata and statistics comparison for dataset revisions.
- Built-in rules for schema consistency, metadata, row groups, statistics,
  compression, timestamp portability, dictionary encoding, small files, and
  GeoParquet, Iceberg, Lance, and Vortex metadata integrity.
- JSON, SARIF, and human-readable reports.
- YAML declarative rules for simple metadata checks.
- Extension points for additional rule packs.

## Supported Inputs

arrow-lint scans:

- Apache Parquet
- Apache Arrow IPC files and stored streams (`*.arrow`, `*.arrows`, and `*.ipc`)
- Feather files
- Apache Iceberg table metadata files (`*.metadata.json` and gzip variants)
- Lance dataset directories (`*.lance`)
- Vortex files (`*.vortex` and `*.vx`)

The project also defines an extension boundary for DuckDB-focused checks.
Iceberg table metadata, the latest local Lance manifest, and Vortex container
metadata are checked by built-in rules.

## Installation

```bash
uv sync
uv run maturin develop
```

## CLI

```bash
uv run arrow-lint lint path/to/dataset --config .arrowlint.yaml
uv run arrow-lint lint path/to/dataset --only AL011
uv run arrow-lint lint path/to/dataset --disable AL004
uv run arrow-lint lint path/to/dataset --output json
uv run arrow-lint lint path/to/dataset --output sarif > arrowlint.sarif
uv run arrow-lint rules
uv run arrow-lint formats
uv run arrow-lint diff old.parquet new.parquet
uv run arrowdiff old.parquet new.parquet
```

ArrowDiff compares schemas, column statistics, metadata, row-group layout,
compression, row counts, and estimated scan bytes. Parquet comparisons use
footer metadata without decoding column values:

```text
ArrowDiff
old.parquet → new.parquet

✓ Schema identical

Column changes
  - phenotype_score
  - temporary_column

Metadata
  Changed:
    ~ microscope_model

Statistics
  Row groups changed (1 → 2)
  Compression changed (UNCOMPRESSED → ZSTD)
  Estimated scan cost +18.0% (1000000 → 1180000 bytes)
```

Use `--output json` for automation and `--exit-code` to return status `1` when
differences are detected. ArrowDiff reports its comparison basis explicitly and
does not claim row-level equality. Arrow IPC and Feather inputs are read through
the existing batch scanner to establish row counts, but do not expose Parquet-
style column statistics.

There is also a native Rust binary:

```bash
cargo run -p arrowlint-cli -- lint path/to/dataset
cargo run -p arrowlint-cli -- diff old.parquet new.parquet
```

## Python API

```python
from arrow_lint import diff, lint

report = lint(
    "path/to/dataset",
    config=".arrowlint.yaml",
    only=["AL011", "AL014"],
)
for diagnostic in report["diagnostics"]:
    print(diagnostic["rule_id"], diagnostic["message"])

changes = diff("old.parquet", "new.parquet")
print(changes["statistics"]["estimated_scan_cost_change_percent"])
```

## Configuration

arrow-lint reads YAML configuration:

```yaml
scan:
  recursive: true

rules:
  min_row_group_rows: 100000
  small_file_bytes: 67108864
  iceberg_max_snapshots: 100
  iceberg_max_metadata_log_entries: 100
  lance_max_versions: 100
  lance_target_fragment_rows: 1048576
  lance_small_fragment_count: 8
  lance_deletion_compaction_threshold: 0.1
  fail_on: error
  only: []
  disabled: []
  declarative_rule_files:
    - examples/rules/metadata.yaml
```

An empty `only` list runs every rule. A non-empty list runs only those rule IDs.
`disabled` rules are removed after selection and therefore take precedence.
The repeatable CLI options `--only` and `--disable` provide invocation-specific
selection without editing the configuration file.

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
       /        |         \
 scanners  dataset model  rules
       \        |         /
         lint + diff reports
```

The core crate owns scanning, diagnostics, rule execution, dataset comparison,
and rendering. The Python package is deliberately thin so plugin authors and
data teams get a comfortable interface without compromising the Rust fast path.

## Extension Strategy

The built-in engine focuses on common Arrow, Feather, Parquet, Iceberg table
metadata, Lance table metadata, and Vortex file metadata checks. The format
pack registry defines stable boundaries for specialized rule packs:

- `arrowlint-duckdb` for Arrow export round-trips, nested types, timestamps, and writer settings.

Future Iceberg work can extend the built-in metadata scanner into manifest-list,
manifest, delete-file, and referenced data-file validation without changing the
public report model. Future Lance work can decode schema and index sections and
inspect data-file internals using the same model. Future Vortex work can decode
layout, dtype, statistics, and array payloads while preserving the bounded
metadata scan. Other rule packs can start in this repository and move to
separate packages when their public APIs are stable.

## Development

```bash
cargo test
uv run maturin develop
uv run pytest
pre-commit run --all-files
```
