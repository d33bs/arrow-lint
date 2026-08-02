---
jupytext:
  text_representation:
    extension: .md
    format_name: myst
    format_version: '0.13'
    jupytext_version: 1.17.3
kernelspec:
  display_name: Python 3
  language: python
  name: python3
---

# It's Dangerous to Go Alone! Take `arrow-lint`.

_A torch for illuminating the hidden choices in your Parquet files._

```{image} ../images/parquet-writers-hero.png
---
alt: A pixel-art explorer uses a torch to inspect Parquet file metadata.
align: center
width: 85%
---
```

PyArrow, DuckDB, and Polars can write the same Arrow data to Parquet, but their
default files do not look the same. This experiment asks each library to write
one table, runs arrow-lint, and follows the metadata to the reason for each
result.

The three paths exercise distinct implementations: PyArrow's C++ writer,
DuckDB's writer, and Polars' native Rust writer. This is a format comparison,
not a query benchmark. File size and arrow-lint's estimated scan bytes are useful
clues, but they do not predict every workload.

## Build one table

The input has 250,000 rows and three simple columns. Repeating category and
score values give each writer an opportunity to use dictionary encoding.

```{code-cell} ipython3
from pathlib import Path
from tempfile import TemporaryDirectory

import duckdb
import polars as pl
import pyarrow as pa
import pyarrow.parquet as pq

from arrow_lint import diff, lint

ROW_COUNT = 250_000
workspace = TemporaryDirectory()
output_dir = Path(workspace.name)

table = pa.table(
    {
        "id": pa.array(range(ROW_COUNT), type=pa.int64()),
        "category": pa.array(
            (f"group-{index % 20}" for index in range(ROW_COUNT)),
            type=pa.string(),
        ),
        "score": pa.array(
            (float(index % 1_000) / 10 for index in range(ROW_COUNT)),
            type=pa.float64(),
        ),
    }
)

{
    "rows": table.num_rows,
    "columns": table.column_names,
    "pyarrow": pa.__version__,
    "duckdb": duckdb.__version__,
    "polars": pl.__version__,
}
```

## Let each writer choose its defaults

PyArrow writes the table directly. DuckDB reads the same in-memory Arrow table
and exports it with `COPY`. Polars imports the table and uses its native writer;
`use_pyarrow=False` is the Polars default. None of the commands specifies
compression, row-group size, or Parquet version.

```{code-cell} ipython3
pyarrow_path = output_dir / "pyarrow-default.parquet"
duckdb_path = output_dir / "duckdb-default.parquet"
polars_path = output_dir / "polars-default.parquet"

pq.write_table(table, pyarrow_path)

with duckdb.connect() as connection:
    connection.register("source", table)
    connection.execute(
        "COPY source TO ? (FORMAT PARQUET)",
        [str(duckdb_path)],
    )

frame = pl.from_arrow(table)
frame.write_parquet(polars_path)
```

The following summary reads only Parquet metadata. It does not scan the table.

```{code-cell} ipython3
def parquet_summary(path: Path) -> dict[str, object]:
    metadata = pq.ParquetFile(path).metadata
    compression = {
        metadata.row_group(group_index).column(column_index).compression
        for group_index in range(metadata.num_row_groups)
        for column_index in range(metadata.num_columns)
    }
    return {
        "file": path.name,
        "writer": metadata.created_by,
        "bytes": path.stat().st_size,
        "rows": metadata.num_rows,
        "row groups": metadata.num_row_groups,
        "rows per group": [
            metadata.row_group(index).num_rows
            for index in range(metadata.num_row_groups)
        ],
        "compression": ", ".join(sorted(compression)),
    }


pa.Table.from_pylist(
    [
        parquet_summary(pyarrow_path),
        parquet_summary(duckdb_path),
        parquet_summary(polars_path),
    ]
)
```

All three files contain 250,000 rows, but their physical choices differ:

- PyArrow writes one row group with Snappy compression.
- DuckDB writes two full 122,880-row groups and a 4,240-row tail, also with
  Snappy.
- Polars writes two balanced 125,000-row groups with Zstandard. That is the
  observed result for this input and locked Polars version when
  `row_group_size` is left unset.

The Polars default file is much smaller here, but most of that headline
difference is not a writer verdict: Zstandard and Snappy are different
compression choices.

## Ask arrow-lint

We keep informational findings in the display because they help distinguish a
shared observation from a writer-specific warning.

```{code-cell} ipython3
def diagnostic_rows(label: str, path: Path) -> list[dict[str, str]]:
    return [
        {
            "file": label,
            "rule": diagnostic["rule_id"],
            "severity": diagnostic["severity"],
            "message": diagnostic["message"],
        }
        for diagnostic in lint(path)["diagnostics"]
    ]


pa.Table.from_pylist(
    diagnostic_rows("PyArrow", pyarrow_path)
    + diagnostic_rows("DuckDB", duckdb_path)
    + diagnostic_rows("Polars", polars_path)
)
```

PyArrow and Polars receive only **AL004**, an informational note that the Arrow
schema has no custom key-value metadata. DuckDB receives the same note plus
three findings:

- **AL001** reports the 4,240-row tail as a tiny row group. DuckDB's default
  row-group size is 122,880 rows, so 250,000 rows naturally leave this small
  remainder.
- **AL010** finds `PLAIN_DICTIONARY`. DuckDB's default Parquet V1 output uses
  this legacy encoding identifier. The Parquet specification deprecates it for
  new files in favor of `RLE_DICTIONARY` for data pages.
- **AL006** sees mixed dictionary strategy for `score`. The two full groups use
  dictionary encoding, but the small final group uses plain encoding.

These are not corruption errors. arrow-lint is pointing out layout and
interoperability choices that deserve an explicit decision.

## Compare the files directly

`arrowdiff` finds another difference that file size alone cannot show. The
summary uses PyArrow as the baseline for both comparisons.

```{code-cell} ipython3
def comparison_summary(label: str, path: Path) -> dict[str, object]:
    comparison = diff(pyarrow_path, path)
    statistics = comparison["statistics"]
    schema_changes = [
        (
            f"{change['name']}: {change['old']['data_type']} -> "
            f"{change['new']['data_type']}"
        )
        for change in comparison["schema"]["changed"]
    ]
    return {
        "candidate": label,
        "schema identical": comparison["schema"]["identical"],
        "schema changes": schema_changes,
        "rows": statistics["new_row_count"],
        "row groups": statistics["new_row_group_count"],
        "compression changed": statistics["compression_changed"],
        "estimated scan change": (
            f"{statistics['estimated_scan_cost_change_percent']:+.2f}%"
        ),
    }


pa.Table.from_pylist(
    [
        comparison_summary("DuckDB", duckdb_path),
        comparison_summary("Polars", polars_path),
    ]
)
```

DuckDB preserves the Arrow schema. Polars preserves the values, but its native
string representation returns to Arrow as `large_string`, which uses 64-bit
offsets instead of `string`'s 32-bit offsets. ArrowDiff therefore reports
`Utf8` to `LargeUtf8` for `category` rather than calling the schemas identical.

```{code-cell} ipython3
polars_round_trip = pq.read_table(polars_path)
category_index = polars_round_trip.schema.get_field_index("category")
width_normalized = polars_round_trip.set_column(
    category_index,
    "category",
    polars_round_trip.column(category_index).cast(pa.string()),
)

{
    "written Arrow type": str(polars_round_trip.schema.field("category").type),
    "values match after offset-width normalization": width_normalized.equals(table),
}
```

That distinction can matter to strict schema contracts even though no category
value changed. arrow-lint's scan-cost estimate remains metadata-based, so read
its percentage as a size signal, not as a claim about query runtime.

## Normalize the obvious variables

For a clearer writer comparison, we can give all three files Snappy compression
and one 250,000-row group. DuckDB also gets Parquet V2; for this data, that
replaces the legacy `PLAIN_DICTIONARY` identifier with modern encodings.

These settings are intentionally tailored to this data. A production row-group
target should reflect file size, memory, parallelism, and the expected query
workload.

```{code-cell} ipython3
duckdb_tuned_path = output_dir / "duckdb-tuned.parquet"
polars_normalized_path = output_dir / "polars-normalized.parquet"

with duckdb.connect() as connection:
    connection.register("source", table)
    connection.execute(
        """
        COPY source TO ? (
            FORMAT PARQUET,
            PARQUET_VERSION 'V2',
            ROW_GROUP_SIZE 250000
        )
        """,
        [str(duckdb_tuned_path)],
    )

frame.write_parquet(
    polars_normalized_path,
    compression="snappy",
    row_group_size=ROW_COUNT,
)

pa.Table.from_pylist(
    [
        parquet_summary(pyarrow_path),
        parquet_summary(duckdb_tuned_path),
        parquet_summary(polars_normalized_path),
    ]
)
```

```{code-cell} ipython3
pa.Table.from_pylist(
    diagnostic_rows("PyArrow", pyarrow_path)
    + diagnostic_rows("DuckDB tuned", duckdb_tuned_path)
    + diagnostic_rows("Polars normalized", polars_normalized_path)
)
```

The tuned DuckDB file no longer triggers AL001, AL006, or AL010.
`ROW_GROUP_SIZE` removes the tiny tail and the mixed strategy it caused;
`PARQUET_VERSION 'V2'` removes the legacy encoding identifier from this file.
Polars was already clean under arrow-lint, so normalizing it is not a fix. It
simply removes compression and row-group count as explanations for the
remaining file and schema differences.

```{code-cell} ipython3
pa.Table.from_pylist(
    [
        comparison_summary("DuckDB tuned", duckdb_tuned_path),
        comparison_summary("Polars normalized", polars_normalized_path),
    ]
)
```

## Practical takeaways

1. Treat writer defaults as choices, not universal best practices.
1. Normalize compression and row-group size before comparing writer efficiency.
1. Check ArrowDiff when offset width or another logical type is contractually
   important; equal values do not guarantee identical Arrow schemas.
1. Inspect row-group boundaries when a file ends with a small remainder.
1. Use arrow-lint findings to explain metadata, then benchmark the real workload
   before drawing performance conclusions.

The relevant upstream references are the
[DuckDB Parquet overview](https://duckdb.org/docs/stable/data/parquet/overview),
[DuckDB Parquet performance guide](https://duckdb.org/docs/stable/data/parquet/tips),
[Polars Parquet writer](https://docs.pola.rs/api/python/stable/reference/api/polars.DataFrame.write_parquet.html),
[Polars Arrow conversion](https://docs.pola.rs/api/python/stable/reference/dataframe/api/polars.DataFrame.to_arrow.html),
[Parquet encoding specification](https://parquet.apache.org/docs/file-format/data-pages/encodings/),
and [`pyarrow.parquet.write_table`](https://arrow.apache.org/docs/python/generated/pyarrow.parquet.write_table.html).

```{code-cell} ipython3
---
tags: [remove-cell]
---
workspace.cleanup()
```
