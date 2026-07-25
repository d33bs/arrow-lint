"""Validate release artifacts before upload."""

from __future__ import annotations

import argparse
import struct
import tarfile
import zipfile
from pathlib import Path

EOCD_SIGNATURE = b"PK\x05\x06"
EOCD_MIN_SIZE = 22
LOCAL_FILE_HEADER_SIGNATURE = b"PK\x03\x04"
SUPPORTED_PLATFORM_PREFIXES = ("any", "manylinux", "musllinux", "macosx", "win")
SUPPORTED_ARCHIVE_SUFFIXES = (".whl", ".tar.gz")
REQUIRED_SDIST_FILES = {
    "Cargo.lock",
    "Cargo.toml",
    "LICENSE",
    "README.md",
    "pyproject.toml",
    "crates/arrowlint-core/Cargo.toml",
    "crates/arrowlint-core/src/lib.rs",
    "crates/arrowlint-python/Cargo.toml",
    "crates/arrowlint-python/src/lib.rs",
    "src/arrow_lint/__init__.py",
    "src/arrow_lint/_native.pyi",
    "src/arrow_lint/api.py",
    "src/arrow_lint/cli.py",
    "src/arrow_lint/py.typed",
}
REQUIRED_WHEEL_SUFFIXES = {
    ".dist-info/METADATA",
    ".dist-info/RECORD",
    ".dist-info/WHEEL",
    ".dist-info/entry_points.txt",
    ".dist-info/licenses/LICENSE",
    "arrow_lint/__init__.py",
    "arrow_lint/_native.pyi",
    "arrow_lint/api.py",
    "arrow_lint/cli.py",
    "arrow_lint/py.typed",
}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("dist", type=Path)
    args = parser.parse_args()

    artifacts = sorted(path for path in args.dist.iterdir() if path.is_file())
    if not artifacts:
        raise SystemExit(f"no release artifacts found in {args.dist}")

    wheels = [path for path in artifacts if path.name.endswith(".whl")]
    sdists = [path for path in artifacts if path.name.endswith(".tar.gz")]
    if not wheels:
        raise SystemExit("release artifacts must include at least one wheel")
    if not sdists:
        raise SystemExit("release artifacts must include a source distribution")

    validate_artifacts(artifacts)

    for artifact in artifacts:
        print(artifact.name)
    return 0


def validate_artifacts(artifacts: list[Path]) -> None:
    for artifact in artifacts:
        if not artifact.name.endswith(SUPPORTED_ARCHIVE_SUFFIXES):
            raise SystemExit(f"unexpected release artifact: {artifact.name}")
        if artifact.name.endswith(".whl"):
            validate_wheel_platform(artifact.name)
            validate_wheel_license_files(artifact)
        if artifact.name.endswith(".tar.gz"):
            validate_sdist_license_files(artifact)


def validate_wheel_platform(filename: str) -> None:
    platform_tag = filename.removesuffix(".whl").split("-")[-1]
    for tag in platform_tag.split("."):
        if tag.startswith("linux_"):
            raise SystemExit(
                f"native Linux wheel tag is not accepted by PyPI: {filename}"
            )
        if not tag.startswith(SUPPORTED_PLATFORM_PREFIXES):
            raise SystemExit(f"unsupported wheel platform tag `{tag}` in {filename}")


def validate_sdist_license_files(path: Path) -> None:
    with tarfile.open(path) as archive:
        names = set(archive.getnames())
        metadata_name = find_member(names, "PKG-INFO")
        if metadata_name is None:
            raise SystemExit(f"source distribution is missing PKG-INFO: {path.name}")
        metadata = archive.extractfile(metadata_name)
        if metadata is None:
            raise SystemExit(
                f"source distribution has unreadable PKG-INFO: {path.name}"
            )
        metadata_text = metadata.read().decode()
        root = metadata_name.removesuffix("/PKG-INFO")
        for license_file in license_files_from_metadata(metadata_text):
            member_name = f"{root}/{license_file}"
            if member_name not in names:
                raise SystemExit(
                    f"{path.name} declares License-File: {license_file}, "
                    f"but {member_name} is missing"
                )
        validate_required_sdist_files(path.name, names, root)


def validate_wheel_license_files(path: Path) -> None:
    validate_strict_zip(path)
    with zipfile.ZipFile(path) as archive:
        names = set(archive.namelist())
        metadata_name = find_member(names, ".dist-info/METADATA")
        if metadata_name is None:
            raise SystemExit(f"wheel is missing METADATA: {path.name}")
        metadata_text = archive.read(metadata_name).decode()
        dist_info = metadata_name.removesuffix("/METADATA")
        for license_file in license_files_from_metadata(metadata_text):
            wheel_paths = {
                license_file,
                f"{dist_info}/{license_file}",
                f"{dist_info}/licenses/{license_file}",
            }
            if names.isdisjoint(wheel_paths):
                raise SystemExit(
                    f"{path.name} declares License-File: {license_file}, "
                    "but the file is missing"
                )
        validate_required_wheel_files(path.name, names)


def validate_strict_zip(path: Path) -> None:
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        duplicates = sorted({name for name in names if names.count(name) > 1})
        if duplicates:
            raise SystemExit(
                f"{path.name} has duplicate ZIP members: {', '.join(duplicates)}"
            )
        invalid_member = archive.testzip()
        if invalid_member is not None:
            raise SystemExit(
                f"{path.name} has invalid ZIP member data: {invalid_member}"
            )

    data = path.read_bytes()
    if not data.startswith(LOCAL_FILE_HEADER_SIGNATURE):
        raise SystemExit(f"{path.name} has prepended data before ZIP archive")
    eocd_offset = data.rfind(EOCD_SIGNATURE)
    if eocd_offset < 0:
        raise SystemExit(f"{path.name} is missing ZIP end-of-central-directory")
    if len(data) < eocd_offset + EOCD_MIN_SIZE:
        raise SystemExit(f"{path.name} has truncated ZIP end-of-central-directory")
    central_directory_size = struct.unpack_from("<I", data, eocd_offset + 12)[0]
    central_directory_offset = struct.unpack_from("<I", data, eocd_offset + 16)[0]
    central_directory_end = central_directory_offset + central_directory_size
    if central_directory_end != eocd_offset:
        raise SystemExit(f"{path.name} has malformed ZIP central directory")
    comment_length = struct.unpack_from("<H", data, eocd_offset + 20)[0]
    if comment_length:
        raise SystemExit(f"{path.name} has ZIP comment data")
    expected_end = eocd_offset + EOCD_MIN_SIZE
    if expected_end != len(data):
        raise SystemExit(f"{path.name} has trailing data after ZIP archive")


def find_member(names: set[str], suffix: str) -> str | None:
    for name in names:
        if name.endswith(suffix):
            return name
    return None


def validate_required_sdist_files(
    artifact_name: str, names: set[str], root: str
) -> None:
    missing = [
        required
        for required in sorted(REQUIRED_SDIST_FILES)
        if f"{root}/{required}" not in names
    ]
    if missing:
        raise SystemExit(
            f"{artifact_name} is missing required source files: {', '.join(missing)}"
        )


def validate_required_wheel_files(artifact_name: str, names: set[str]) -> None:
    missing = [
        required
        for required in sorted(REQUIRED_WHEEL_SUFFIXES)
        if find_member(names, required) is None
    ]
    native_extension = any(
        name.startswith("arrow_lint/_native.") and name.endswith((".so", ".pyd"))
        for name in names
    )
    if not native_extension:
        missing.append("arrow_lint/_native.{so,pyd}")
    if missing:
        raise SystemExit(
            f"{artifact_name} is missing required wheel files: {', '.join(missing)}"
        )


def license_files_from_metadata(metadata: str) -> list[str]:
    prefix = "License-File:"
    return [
        line.removeprefix(prefix).strip()
        for line in metadata.splitlines()
        if line.startswith(prefix)
    ]


if __name__ == "__main__":
    raise SystemExit(main())
