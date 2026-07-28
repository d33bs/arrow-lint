# ArrowDiff

ArrowDiff provides a Git-style comparison for Arrow datasets:

```bash
arrowdiff old.parquet new.parquet
```

The same command is available through the main CLI:

```bash
arrow-lint diff old.parquet new.parquet
```

## What It Compares

ArrowDiff scans both paths through the shared Rust dataset model and reports:

- field names, types, nullability, metadata, and column order
- columns whose Parquet value counts or min/max/null/distinct statistics changed
- dataset and file key-value metadata
- file and row counts
- row-group structure
- compression codecs
- estimated scan bytes and percentage change

Directories are compared recursively using the same discovery behavior as
ArrowLint.

## Output

Human-readable output is the default:

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

Use JSON for CI, notebooks, or downstream tooling:

```bash
arrowdiff old.parquet new.parquet --output json
```

Use Git-style exit status when differences should affect automation:

```bash
arrowdiff old.parquet new.parquet --exit-code
```

The command returns `0` after a successful comparison by default. With
`--exit-code`, it returns `1` when a difference is detected and `0` when the
reports are equivalent.

## Comparison Semantics

The current comparison basis is `metadata_and_statistics`. Parquet comparisons
read footer metadata without decoding column values and therefore do not prove
row-level equality. Column statistics can identify many changed columns, but
changes that preserve the same min/max/null/distinct summary may not be visible.
Arrow IPC and Feather files are read through the existing batch scanner to
establish row counts; they do not provide equivalent column-chunk statistics, so
their comparisons focus on schema, metadata, row count, and file size.

The JSON report includes `comparison_basis` and `has_changes` so applications
can enforce the level of evidence they require without reproducing ArrowDiff's
change predicate.
