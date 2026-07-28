"""Tests for the Python API."""

import json

from pytest import MonkeyPatch

from arrow_lint import _native
from arrow_lint.api import diff, formats, rules


def test_rules_exposes_builtin_metadata() -> None:
    builtin_rules = rules()

    assert any(rule["id"] == "AL003" for rule in builtin_rules)


def test_formats_exposes_extension_targets() -> None:
    known_formats = formats()

    assert any(format_pack["name"] == "iceberg" for format_pack in known_formats)
    assert any(format_pack["name"] == "duckdb" for format_pack in known_formats)


def test_diff_exposes_structured_report(monkeypatch: MonkeyPatch) -> None:
    expected = {
        "comparison_basis": "metadata_and_statistics",
        "schema": {"identical": True},
    }
    monkeypatch.setattr(
        _native,
        "diff_paths_json",
        lambda old_path, new_path: json.dumps(
            {**expected, "old_path": old_path, "new_path": new_path}
        ),
    )

    report = diff("old.parquet", "new.parquet")

    assert report["comparison_basis"] == "metadata_and_statistics"
    assert report["schema"]["identical"] is True
    assert report["old_path"] == "old.parquet"
    assert report["new_path"] == "new.parquet"
