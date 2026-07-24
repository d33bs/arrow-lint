"""Tests for the Python API."""

from arrow_lint.api import formats, rules


def test_rules_exposes_builtin_metadata() -> None:
    builtin_rules = rules()

    assert any(rule["id"] == "AL003" for rule in builtin_rules)


def test_formats_exposes_extension_targets() -> None:
    known_formats = formats()

    assert any(format_pack["name"] == "iceberg" for format_pack in known_formats)
    assert any(format_pack["name"] == "duckdb" for format_pack in known_formats)
