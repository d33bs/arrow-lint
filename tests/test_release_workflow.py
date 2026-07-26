"""Tests for release workflow safety."""

from pathlib import Path

import yaml


def test_release_workflow_uses_unique_runner_matrix() -> None:
    workflow = yaml.safe_load(Path(".github/workflows/publish-pypi.yml").read_text())
    matrix = workflow["jobs"]["build_wheels"]["strategy"]["matrix"]

    assert matrix["os"] == [
        "ubuntu-24.04",
        "ubuntu-24.04-arm",
        "macos-15-intel",
        "windows-2022",
    ]
    assert len(matrix["os"]) == len(set(matrix["os"]))
    assert matrix["python-version"] == ["3.11", "3.12", "3.13"]
