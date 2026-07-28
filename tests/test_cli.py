"""Tests for the CLI module."""

import json

from pytest import CaptureFixture, MonkeyPatch

from arrow_lint.cli import diff_main, main


def test_rules_cli_lists_builtin_rules(capsys: CaptureFixture[str]) -> None:
    result = main(["rules"])

    captured = capsys.readouterr()
    assert result == 0
    assert "AL001" in captured.out
    assert "tiny-row-groups" in captured.out


def test_diff_cli_renders_text(
    monkeypatch: MonkeyPatch, capsys: CaptureFixture[str]
) -> None:
    monkeypatch.setattr(
        "arrow_lint.cli.render_diff",
        lambda old, new, output: f"ArrowDiff\n{old} → {new}\n",
    )

    result = main(["diff", "old.parquet", "new.parquet"])

    captured = capsys.readouterr()
    assert result == 0
    assert "ArrowDiff" in captured.out
    assert "old.parquet → new.parquet" in captured.out


def test_arrowdiff_json_and_exit_code(
    monkeypatch: MonkeyPatch, capsys: CaptureFixture[str]
) -> None:
    report = {
        "has_changes": True,
        "schema": {"identical": True},
        "columns": {"changed": ["phenotype_score"]},
        "metadata": {"added": [], "removed": [], "changed": []},
        "statistics": {
            "file_count_changed": False,
            "row_count_changed": False,
            "row_groups_changed": False,
            "compression_changed": False,
            "scan_cost_changed": False,
        },
    }
    monkeypatch.setattr("arrow_lint.cli.diff", lambda old, new: report)

    result = diff_main(
        ["old.parquet", "new.parquet", "--output", "json", "--exit-code"]
    )

    captured = capsys.readouterr()
    assert result == 1
    assert json.loads(captured.out)["columns"]["changed"] == ["phenotype_score"]
