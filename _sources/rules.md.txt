# Rules

ArrowLint ships with built-in rules for common Arrow and Parquet quality issues.
Rules classified as errors identify internally inconsistent metadata. Warnings
identify deprecated features or broadly applicable interoperability and
performance concerns. Informational diagnostics identify review opportunities.

| Rule    | Severity | Category         | Purpose                                                 |
| ------- | -------- | ---------------- | ------------------------------------------------------- |
| `AL001` | warning  | performance      | flags tiny Parquet row groups                           |
| `AL002` | warning  | metadata         | flags Parquet column chunks missing statistics          |
| `AL003` | error    | schema           | flags schema drift across files in one target           |
| `AL004` | info     | metadata         | reports missing Arrow schema metadata                   |
| `AL005` | warning  | interoperability | flags timestamp choices that may not round-trip cleanly |
| `AL006` | warning  | encoding         | flags mixed dictionary encoding strategy                |
| `AL007` | info     | performance      | reports many small files                                |
| `AL008` | warning  | performance      | flags uncompressed Parquet columns                      |
| `AL009` | warning  | interoperability | flags the deprecated Parquet `INT96` physical type      |
| `AL010` | warning  | interoperability | flags deprecated Parquet encodings                      |
| `AL011` | warning  | interoperability | flags the deprecated Parquet `LZ4` codec                |
| `AL012` | error    | correctness      | flags impossible Parquet statistic counts               |
| `AL013` | error    | correctness      | flags inconsistent Parquet file and row-group counts    |
| `AL014` | warning  | metadata         | flags statistics that omit explicit null counts         |
| `AL015` | error    | correctness      | flags negative Parquet counts and byte sizes            |
| `AL100` | info     | extension        | identifies inputs handled by external format packs      |

## Parquet Validity and Compatibility

### `AL009` — Deprecated Parquet Physical Types

Reports each Parquet column chunk whose physical type is `INT96`. The Parquet
format explicitly deprecates `INT96` and directs new writers not to emit it.
Rewrite timestamp columns as `INT64` with a `TIMESTAMP` logical annotation.

This rule is a warning because existing readers commonly support legacy
`INT96` files, even though new files should not use the type.

Reference: [Parquet physical types](https://parquet.apache.org/docs/file-format/types/)

### `AL010` — Deprecated Parquet Encodings

Reports column chunks whose encoding set contains:

- `PLAIN_DICTIONARY`, which has been superseded by `RLE_DICTIONARY` for data
  pages.
- `BIT_PACKED`, which has been superseded by the hybrid `RLE` encoding for
  repetition and definition levels.

The diagnostic identifies the row group, column path, and all deprecated
encodings found in that column chunk. Rewrite the file with a current Parquet
writer rather than changing reader configuration.

Reference: [Parquet encoding definitions](https://parquet.apache.org/docs/file-format/data-pages/encodings/)

### `AL011` — Deprecated Parquet Compression

Reports column chunks using the deprecated `LZ4` codec. `LZ4` and `LZ4_RAW` are
different Parquet codec identifiers: this rule does not report `LZ4_RAW`.
Rewrite affected columns with `LZ4_RAW`, `ZSTD`, or `SNAPPY`, based on the
required reader compatibility and workload.

Reference: [Parquet compression codecs](https://github.com/apache/parquet-format/blob/master/Compression.md)

### `AL012` — Invalid Parquet Statistics

Reports impossible count metadata as an error:

- `null_count` greater than the column chunk's `num_values`.
- `distinct_count` greater than the column chunk's `num_values`.

Readers use statistics for predicate pruning. Impossible counts indicate
corrupt or incorrectly produced metadata and should not be trusted. Rewrite
the file before using its statistics for query planning.

Reference: [Parquet `Statistics` definition](https://github.com/apache/parquet-format/blob/master/src/main/thrift/parquet.thrift)

### `AL013` — Inconsistent Parquet Row Counts

Reports a file when `FileMetaData.num_rows` differs from the sum of
`RowGroup.num_rows`, or when that sum exceeds the signed 64-bit metadata range.
The rule does not run the comparison when a row group has a negative count;
`AL015` reports that more fundamental error instead.

This diagnostic is an error because both metadata locations describe the same
rows and must agree.

Reference: [Parquet metadata definition](https://github.com/apache/parquet-format/blob/master/src/main/thrift/parquet.thrift)

### `AL014` — Missing Parquet Null Counts

Reports a column chunk that has a statistics structure but does not include
`null_count`. The Parquet specification recommends writing this field even
when the count is zero or the column is required. Readers must distinguish a
missing count from a known zero count.

This rule does not report column chunks with no statistics at all; `AL002`
handles that case.

Reference: [Parquet `Statistics.null_count`](https://github.com/apache/parquet-format/blob/master/src/main/thrift/parquet.thrift)

### `AL015` — Invalid Parquet Size Metadata

Reports negative values in metadata fields that represent counts or sizes:

- File and row-group row counts.
- Row-group uncompressed byte sizes.
- Column-chunk value counts.
- Column-chunk compressed and uncompressed byte sizes.

Zero remains valid for empty files, empty row groups, and empty column chunks.
Negative values are structurally invalid and are reported as errors.

Reference: [Parquet metadata structures](https://github.com/apache/parquet-format/blob/master/src/main/thrift/parquet.thrift)

## Rule Scope

The new validity checks are metadata-only. They do not decode data pages or
compare stored statistics with every physical value. A clean result therefore
means the inspected footer metadata is internally consistent; it does not
prove that the complete file contents are uncorrupted.

## Selecting Rules

Run a focused rule set with repeatable CLI options:

```bash
arrow-lint lint dataset --only AL011 --only AL014
arrow-lint lint dataset --disable AL004
```

The native Rust CLI supports the same options:

```bash
cargo run -p arrowlint-cli -- lint dataset --only AL011
```

Persist selection in `.arrowlint.yaml` when a project needs the same policy on
every run:

```yaml
rules:
  only:
    - AL011
    - AL014
  disabled:
    - AL014
```

An empty or omitted `only` list enables all built-in and declarative rules. A
non-empty `only` list includes only matching rule IDs. `disabled` always takes
precedence, so the example runs `AL011` and suppresses `AL014`. Supplying
`--only` replaces the configured `only` list for that invocation; `--disable`
adds to the configured disabled rules.

## Declarative Rules

Use declarative YAML rules for simple metadata checks:

```yaml
rule: missing_crs
description: GeoParquet file is missing CRS metadata
severity: warning
applies_to: parquet
check:
  metadata_key: crs
```

Reference rule files from `.arrowlint.yaml`:

```yaml
rules:
  declarative_rule_files:
    - examples/rules/metadata.yaml
```

Declarative rules are best suited to metadata presence checks. Checks that need
record batch inspection, cross-file state, or format-specific metadata should be
implemented as Rust rules or dedicated rule packs.
