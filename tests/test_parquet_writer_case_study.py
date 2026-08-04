from __future__ import annotations

from pathlib import Path
from typing import Any

import duckdb
import polars as pl
import pyarrow as pa
import pyarrow.parquet as pq

from arrow_lint import diff, lint

ROW_COUNT = 250_000


def test_parquet_writer_case_study_stays_reproducible(
    tmp_path: Path,
) -> None:
    table = _case_study_table()
    paths = _write_case_study_files(tmp_path, table)

    _assert_metadata_and_values(table, paths)
    _assert_lint_results(paths)
    _assert_diff_results(paths)
    _assert_case_study_document()


def _write_case_study_files(
    tmp_path: Path,
    table: pa.Table,
) -> dict[str, Path]:
    paths = {
        "pyarrow": tmp_path / "pyarrow.parquet",
        "duckdb": tmp_path / "duckdb-default.parquet",
        "polars": tmp_path / "polars-default.parquet",
        "duckdb_tuned": tmp_path / "duckdb-tuned.parquet",
        "polars_normalized": tmp_path / "polars-normalized.parquet",
    }

    pq.write_table(table, paths["pyarrow"])
    frame = pl.from_arrow(table)
    if not isinstance(frame, pl.DataFrame):
        raise TypeError
    frame.write_parquet(paths["polars"])
    frame.write_parquet(
        paths["polars_normalized"],
        compression="snappy",
        row_group_size=ROW_COUNT,
    )
    with duckdb.connect() as connection:
        connection.register("source", table)
        connection.execute(
            "COPY source TO ? (FORMAT PARQUET)",
            [str(paths["duckdb"])],
        )
        connection.execute(
            """
            COPY source TO ? (
                FORMAT PARQUET,
                PARQUET_VERSION 'V2',
                ROW_GROUP_SIZE 250000
            )
            """,
            [str(paths["duckdb_tuned"])],
        )

    return paths


def _assert_metadata_and_values(
    table: pa.Table,
    paths: dict[str, Path],
) -> None:
    pyarrow_metadata = pq.ParquetFile(paths["pyarrow"]).metadata
    duckdb_metadata = pq.ParquetFile(paths["duckdb"]).metadata
    polars_metadata = pq.ParquetFile(paths["polars"]).metadata
    duckdb_tuned_metadata = pq.ParquetFile(paths["duckdb_tuned"]).metadata
    polars_normalized_metadata = pq.ParquetFile(paths["polars_normalized"]).metadata

    assert pyarrow_metadata.num_rows == ROW_COUNT
    assert duckdb_metadata.num_rows == ROW_COUNT
    assert polars_metadata.num_rows == ROW_COUNT
    assert duckdb_tuned_metadata.num_rows == ROW_COUNT
    assert polars_normalized_metadata.num_rows == ROW_COUNT
    assert _row_group_rows(pyarrow_metadata) == [ROW_COUNT]
    assert _row_group_rows(duckdb_metadata) == [122_880, 122_880, 4_240]
    assert _row_group_rows(polars_metadata) == [125_000, 125_000]
    assert _row_group_rows(duckdb_tuned_metadata) == [ROW_COUNT]
    assert _row_group_rows(polars_normalized_metadata) == [ROW_COUNT]
    assert _column_encodings(duckdb_metadata, "score") == [
        ("PLAIN_DICTIONARY",),
        ("PLAIN_DICTIONARY",),
        ("PLAIN",),
    ]
    assert "PLAIN_DICTIONARY" not in {
        encoding
        for column_encodings in _all_column_encodings(duckdb_tuned_metadata)
        for encoding in column_encodings
    }
    assert pq.read_table(paths["pyarrow"]).equals(table)
    assert pq.read_table(paths["duckdb"]).equals(table)
    assert pq.read_table(paths["duckdb_tuned"]).equals(table)
    assert _normalize_polars_strings(pq.read_table(paths["polars"])).equals(table)
    assert _normalize_polars_strings(pq.read_table(paths["polars_normalized"])).equals(
        table
    )


def _assert_lint_results(paths: dict[str, Path]) -> None:
    pyarrow_report = lint(paths["pyarrow"])
    duckdb_report = lint(paths["duckdb"])
    polars_report = lint(paths["polars"])
    duckdb_tuned_report = lint(paths["duckdb_tuned"])
    polars_normalized_report = lint(paths["polars_normalized"])
    duckdb_rules = {
        diagnostic["rule_id"] for diagnostic in duckdb_report["diagnostics"]
    }

    assert not _actionable_rules(pyarrow_report)
    assert {"AL001", "AL006", "AL010"} <= duckdb_rules
    assert not _actionable_rules(polars_report)
    assert not _actionable_rules(duckdb_tuned_report)
    assert not _actionable_rules(polars_normalized_report)


def _assert_diff_results(paths: dict[str, Path]) -> None:
    duckdb_comparison = diff(paths["pyarrow"], paths["duckdb"])
    assert duckdb_comparison["schema"]["identical"] is True
    assert duckdb_comparison["statistics"]["row_count_changed"] is False
    assert duckdb_comparison["statistics"]["row_groups_changed"] is True
    assert duckdb_comparison["statistics"]["compression_changed"] is False
    assert duckdb_comparison["statistics"]["estimated_scan_cost_change_percent"] < 0

    polars_comparison = diff(paths["pyarrow"], paths["polars"])
    assert polars_comparison["schema"]["identical"] is False
    assert polars_comparison["schema"]["changed"] == [
        {
            "name": "category",
            "old": {
                "name": "category",
                "data_type": "Utf8",
                "nullable": True,
                "metadata": {},
            },
            "new": {
                "name": "category",
                "data_type": "LargeUtf8",
                "nullable": True,
                "metadata": {},
            },
        }
    ]
    assert polars_comparison["statistics"]["row_count_changed"] is False
    assert polars_comparison["statistics"]["row_groups_changed"] is True
    assert polars_comparison["statistics"]["compression_changed"] is True

    normalized_comparison = diff(paths["pyarrow"], paths["polars_normalized"])
    assert normalized_comparison["statistics"]["old_row_group_count"] == 1
    assert normalized_comparison["statistics"]["new_row_group_count"] == 1
    assert normalized_comparison["statistics"]["compression_changed"] is False


def _assert_case_study_document() -> None:
    case_study = (
        Path(__file__).parents[1]
        / "docs"
        / "src"
        / "case-studies"
        / "parquet-writers.md"
    )
    assert case_study.is_file()
    article = case_study.read_text()
    assert article.startswith("---\n")
    assert "jupytext:" in article.split("---", maxsplit=2)[1]
    assert "```{code-cell} ipython3" in article
    hero_image = case_study.parents[1] / "images" / "parquet-writers-hero.png"
    assert hero_image.is_file()
    assert "../images/parquet-writers-hero.png" in article
    assert "width: 85%" in article
    assert "## The short version" in article
    assert "## Reproduce the case study" in article
    assert article.index("## The short version") < article.index(
        "```{code-cell} ipython3"
    )
    assert "arrow-lint diff" in article
    assert "ArrowDiff" not in article
    assert "arrowdiff" not in article
    for rule_id in ("AL001", "AL006", "AL010"):
        assert rule_id in article


def _case_study_table() -> pa.Table:
    return pa.table(
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


def _row_group_rows(metadata: pq.FileMetaData) -> list[int]:
    return [
        metadata.row_group(index).num_rows for index in range(metadata.num_row_groups)
    ]


def _column_encodings(
    metadata: pq.FileMetaData,
    column_name: str,
) -> list[tuple[str, ...]]:
    return [
        next(
            metadata.row_group(group_index).column(column_index).encodings
            for column_index in range(metadata.num_columns)
            if metadata.row_group(group_index).column(column_index).path_in_schema
            == column_name
        )
        for group_index in range(metadata.num_row_groups)
    ]


def _all_column_encodings(
    metadata: pq.FileMetaData,
) -> list[tuple[str, ...]]:
    return [
        metadata.row_group(group_index).column(column_index).encodings
        for group_index in range(metadata.num_row_groups)
        for column_index in range(metadata.num_columns)
    ]


def _actionable_rules(report: dict[str, Any]) -> set[str]:
    return {
        diagnostic["rule_id"]
        for diagnostic in report["diagnostics"]
        if diagnostic["severity"] in {"warning", "error"}
    }


def _normalize_polars_strings(table: pa.Table) -> pa.Table:
    category_index = table.schema.get_field_index("category")
    return table.set_column(
        category_index,
        "category",
        table.column(category_index).cast(pa.string()),
    )
