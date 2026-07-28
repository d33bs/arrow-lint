"""Tests for release artifact validation."""

import io
import tarfile
import zipfile
from pathlib import Path

import pytest

from tools.check_release_artifacts import (
    REQUIRED_SDIST_FILES,
    validate_required_sdist_files,
    validate_required_wheel_files,
    validate_sdist_license_files,
    validate_strict_zip,
    validate_wheel_platform,
    validate_wheel_python,
)
from tools.prepare_release_dist import prepare_release_dist


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


def test_release_artifacts_accept_matching_python_tag() -> None:
    validate_wheel_python(
        "arrow_lint-0.0.3-cp312-cp312-manylinux_2_28_x86_64.whl",
        "3.12",
    )


def test_release_artifacts_reject_mismatched_python_tag() -> None:
    with pytest.raises(SystemExit, match=r"does not match matrix Python 3\.12"):
        validate_wheel_python(
            "arrow_lint-0.0.3-cp311-cp311-manylinux_2_28_x86_64.whl",
            "3.12",
        )


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


def test_release_artifacts_reject_trailing_wheel_data(tmp_path: Path) -> None:
    wheel = tmp_path / "arrow_lint-0.0.3-cp311-cp311-any.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        archive.writestr("arrow_lint/__init__.py", "")
    with wheel.open("ab") as handle:
        handle.write(b"trailing data")

    with pytest.raises(SystemExit, match="trailing data"):
        validate_strict_zip(wheel)


def test_release_artifacts_reject_wheel_comment_data(tmp_path: Path) -> None:
    wheel = tmp_path / "arrow_lint-0.0.3-cp311-cp311-any.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        archive.comment = b"comment"
        archive.writestr("arrow_lint/__init__.py", "")

    with pytest.raises(SystemExit, match="ZIP comment data"):
        validate_strict_zip(wheel)


def test_release_artifacts_reject_prepended_wheel_data(tmp_path: Path) -> None:
    wheel = tmp_path / "arrow_lint-0.0.3-cp311-cp311-any.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        archive.writestr("arrow_lint/__init__.py", "")
    wheel.write_bytes(b"prepended data" + wheel.read_bytes())

    with pytest.raises(SystemExit, match="prepended data"):
        validate_strict_zip(wheel)


def test_release_artifacts_reject_duplicate_downloaded_filenames(
    tmp_path: Path,
) -> None:
    source = tmp_path / "dist-artifacts"
    first = source / "wheels-ubuntu"
    second = source / "wheels-macos"
    first.mkdir(parents=True)
    second.mkdir(parents=True)
    artifact_name = "arrow_lint-0.0.3-cp311-cp311-any.whl"
    (first / artifact_name).write_bytes(b"first")
    (second / artifact_name).write_bytes(b"second")

    with pytest.raises(SystemExit, match="duplicate release artifact filename"):
        prepare_release_dist(source, tmp_path / "dist")


def add_tar_text(archive: tarfile.TarFile, name: str, text: str = "content\n") -> None:
    payload = text.encode()
    info = tarfile.TarInfo(name)
    info.size = len(payload)
    archive.addfile(info, io.BytesIO(payload))
