"""Tests for release artifact validation."""

from pathlib import Path

import pytest

from tools.check_release_artifacts import validate_artifacts


def test_release_artifacts_accept_pypi_platform_wheels() -> None:
    validate_artifacts(
        [
            Path("arrow_lint-0.0.3.tar.gz"),
            Path("arrow_lint-0.0.3-cp311-cp311-manylinux_2_28_x86_64.whl"),
            Path("arrow_lint-0.0.3-cp311-cp311-manylinux2014_x86_64.whl"),
            Path("arrow_lint-0.0.3-cp311-cp311-musllinux_1_2_x86_64.whl"),
            Path("arrow_lint-0.0.3-cp311-cp311-macosx_11_0_arm64.whl"),
            Path("arrow_lint-0.0.3-cp311-cp311-win_amd64.whl"),
        ]
    )


def test_release_artifacts_reject_native_linux_wheel() -> None:
    with pytest.raises(SystemExit, match="native Linux wheel tag"):
        validate_artifacts(
            [
                Path("arrow_lint-0.0.3.tar.gz"),
                Path("arrow_lint-0.0.3-cp311-cp311-linux_x86_64.whl"),
            ]
        )


def test_release_artifacts_reject_unknown_platform_wheel() -> None:
    with pytest.raises(SystemExit, match="unsupported wheel platform tag"):
        validate_artifacts(
            [
                Path("arrow_lint-0.0.3.tar.gz"),
                Path("arrow_lint-0.0.3-cp311-cp311-solaris_11_x86_64.whl"),
            ]
        )
