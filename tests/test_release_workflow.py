"""Tests for release workflow safety."""

import re
from pathlib import Path
from typing import Any

import yaml


def test_release_workflow_uses_unique_runner_matrix() -> None:
    workflow = load_release_workflow()
    matrix = workflow["jobs"]["build_wheels"]["strategy"]["matrix"]

    assert len(matrix["os"]) == len(set(matrix["os"]))
    assert len(matrix["python-version"]) == len(set(matrix["python-version"]))
    assert all(
        re.fullmatch(r"\d+\.\d+", python_version)
        for python_version in matrix["python-version"]
    )


def test_release_workflow_builds_with_matrix_python() -> None:
    workflow = load_release_workflow()
    steps = workflow["jobs"]["build_wheels"]["steps"]
    build_step = next(step for step in steps if step["name"] == "Build wheel")
    verify_step = next(
        step for step in steps if step["name"] == "Verify wheel interpreter tag"
    )

    assert '--python "${{ matrix.python-version }}"' in build_step["run"]
    assert "--interpreter python" in build_step["run"]
    assert '--expected-python "${{ matrix.python-version }}"' in verify_step["run"]


def test_release_helpers_do_not_install_local_project() -> None:
    workflow = load_release_workflow()
    publish_steps = workflow["jobs"]["publish_pypi"]["steps"]
    helper_steps = {
        step["name"]: step["run"]
        for step in publish_steps
        if step["name"]
        in {"Prepare package distributions", "Verify package distributions"}
    }

    assert "uv run" not in "\n".join(helper_steps.values())


def load_release_workflow() -> dict[str, Any]:
    return yaml.safe_load(Path(".github/workflows/publish-pypi.yml").read_text())
