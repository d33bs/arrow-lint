"""Tests for release artifact validation."""

import io
import tarfile
from pathlib import Path

import pytest

from tools.check_release_artifacts import (
    REQUIRED_SDIST_FILES,
    validate_required_sdist_files,
    validate_required_wheel_files,
    validate_sdist_license_files,
    validate_wheel_platform,
)


def test_release_artifacts_accept_pypi_platform_wheels() -> None:
    validate_wheel_platform("arrow_lint-0.0.3-cp311-cp311-manylinux_2_28_x86_64.whl")
    validate_wheel_platform("arrow_lint-0.0.3-cp311-cp311-manylinux2014_x86_64.whl")
    validate_wheel_platform("arrow_lint-0.0.3-cp311-cp311-musllinux_1_2_x86_64.whl")
    validate_wheel_platform("arrow_lint-0.0.3-cp311-cp311-macosx_11_0_arm64.whl")
    validate_wheel_platform("arrow_lint-0.0.3-cp311-cp311-win_amd64.whl")


def test_release_artifacts_reject_native_linux_wheel() -> None:
    with pytest.raises(SystemExit, match="native Linux wheel tag"):
        validate_wheel_platform("arrow_lint-0.0.3-cp311-cp311-linux_x86_64.whl")


def test_release_artifacts_reject_unknown_platform_wheel() -> None:
    with pytest.raises(SystemExit, match="unsupported wheel platform tag"):
        validate_wheel_platform("arrow_lint-0.0.3-cp311-cp311-solaris_11_x86_64.whl")


def test_release_artifacts_reject_missing_sdist_license_file(tmp_path: Path) -> None:
    sdist = tmp_path / "arrow_lint-0.0.3.tar.gz"
    pkg_info = tmp_path / "PKG-INFO"
    pkg_info.write_text("Metadata-Version: 2.4\nLicense-File: LICENSE\n")
    with tarfile.open(sdist, "w:gz") as archive:
        archive.add(pkg_info, arcname="arrow_lint-0.0.3/PKG-INFO")

    with pytest.raises(SystemExit, match="declares License-File: LICENSE"):
        validate_sdist_license_files(sdist)


def test_release_artifacts_accept_sdist_license_file(tmp_path: Path) -> None:
    sdist = tmp_path / "arrow_lint-0.0.3.tar.gz"
    pkg_info = tmp_path / "PKG-INFO"
    pkg_info.write_text("Metadata-Version: 2.4\nLicense-File: LICENSE\n")
    with tarfile.open(sdist, "w:gz") as archive:
        archive.add(pkg_info, arcname="arrow_lint-0.0.3/PKG-INFO")
        for required_file in REQUIRED_SDIST_FILES:
            add_tar_text(archive, f"arrow_lint-0.0.3/{required_file}")

    validate_sdist_license_files(sdist)


def test_release_artifacts_reject_missing_required_sdist_files() -> None:
    with pytest.raises(SystemExit, match="missing required source files"):
        validate_required_sdist_files(
            "arrow_lint-0.0.3.tar.gz", set(), "arrow_lint-0.0.3"
        )


def test_release_artifacts_reject_missing_required_wheel_files() -> None:
    with pytest.raises(SystemExit, match="missing required wheel files"):
        validate_required_wheel_files("arrow_lint-0.0.3-cp311-cp311-any.whl", set())


def add_tar_text(archive: tarfile.TarFile, name: str, text: str = "content\n") -> None:
    payload = text.encode()
    info = tarfile.TarInfo(name)
    info.size = len(payload)
    archive.addfile(info, io.BytesIO(payload))
