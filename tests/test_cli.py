"""Tests for the CLI module."""

from pytest import CaptureFixture

from arrow_lint.cli import main


def test_rules_cli_lists_builtin_rules(capsys: CaptureFixture[str]) -> None:
    result = main(["rules"])

    captured = capsys.readouterr()
    assert result == 0
    assert "AL001" in captured.out
    assert "tiny-row-groups" in captured.out
