"""Validate release artifacts before upload."""

from __future__ import annotations

import argparse
from pathlib import Path

SUPPORTED_PLATFORM_PREFIXES = ("any", "manylinux", "musllinux", "macosx", "win")
SUPPORTED_ARCHIVE_SUFFIXES = (".whl", ".tar.gz")


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


def validate_wheel_platform(filename: str) -> None:
    platform_tag = filename.removesuffix(".whl").split("-")[-1]
    for tag in platform_tag.split("."):
        if tag.startswith("linux_"):
            raise SystemExit(
                f"native Linux wheel tag is not accepted by PyPI: {filename}"
            )
        if not tag.startswith(SUPPORTED_PLATFORM_PREFIXES):
            raise SystemExit(f"unsupported wheel platform tag `{tag}` in {filename}")


if __name__ == "__main__":
    raise SystemExit(main())
