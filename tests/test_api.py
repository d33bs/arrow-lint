"""Tests for the Python API."""

import json

from pytest import MonkeyPatch

from arrow_lint import _native
from arrow_lint.api import diff, formats, lint, render, rules


def test_rules_exposes_builtin_metadata() -> None:
    builtin_rules = rules()
    rules_by_id = {rule["id"]: rule for rule in builtin_rules}

    assert rules_by_id["AL003"]["default_severity"] == "error"
    assert set(rules_by_id).issuperset(
        {
            "AL009",
            "AL010",
            "AL011",
            "AL012",
            "AL013",
            "AL014",
            "AL015",
            "AL101",
            "AL102",
            "AL103",
            "AL104",
            "AL105",
            "AL106",
            "AL107",
        }
    )
    assert rules_by_id["AL012"]["default_severity"] == "error"
    assert rules_by_id["AL014"]["default_severity"] == "warning"
    assert rules_by_id["AL107"]["default_severity"] == "info"


def test_formats_exposes_extension_targets() -> None:
    known_formats = formats()

    assert any(format_pack["name"] == "iceberg" for format_pack in known_formats)
    assert any(format_pack["name"] == "duckdb" for format_pack in known_formats)
    assert (
        next(
            format_pack["status"]
            for format_pack in known_formats
            if format_pack["name"] == "iceberg"
        )
        == "built-in-metadata"
    )


def test_lint_forwards_rule_selection(monkeypatch: MonkeyPatch) -> None:
    calls: list[tuple[list[str], str | None, list[str] | None, list[str] | None]] = []

    def lint_paths_json(
        paths: list[str],
        config_path: str | None,
        only: list[str] | None,
        disabled: list[str] | None,
    ) -> str:
        calls.append((paths, config_path, only, disabled))
        return json.dumps({"diagnostics": []})

    monkeypatch.setattr(_native, "lint_paths_json", lint_paths_json)

    report = lint(
        "dataset.parquet",
        config=".arrowlint.yaml",
        only=["AL011"],
        disabled=["AL001"],
    )

    assert report == {"diagnostics": []}
    assert calls == [
        (
            ["dataset.parquet"],
            ".arrowlint.yaml",
            ["AL011"],
            ["AL001"],
        )
    ]


def test_render_forwards_rule_selection(monkeypatch: MonkeyPatch) -> None:
    calls: list[
        tuple[
            list[str],
            str | None,
            str,
            list[str] | None,
            list[str] | None,
        ]
    ] = []

    def render_lint(
        paths: list[str],
        config_path: str | None,
        output: str,
        only: list[str] | None,
        disabled: list[str] | None,
    ) -> str:
        calls.append((paths, config_path, output, only, disabled))
        return "No diagnostics.\n"

    monkeypatch.setattr(_native, "render_lint", render_lint)

    output = render(
        "dataset.parquet",
        only=["AL011"],
        disabled=["AL001"],
    )

    assert output == "No diagnostics.\n"
    assert calls == [
        (
            ["dataset.parquet"],
            None,
            "text",
            ["AL011"],
            ["AL001"],
        )
    ]


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
