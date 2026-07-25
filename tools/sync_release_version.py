"""Synchronize Cargo workspace version with a release tag."""

from __future__ import annotations

import argparse
import re
import subprocess
from pathlib import Path

VERSION_PATTERN = re.compile(r'^version = "\d+\.\d+\.\d+(?:[-+][^"]+)?"$', re.MULTILINE)
TAG_PATTERN = re.compile(r"^v?(?P<version>\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)$")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("tag", help="release tag, for example v0.0.3")
    parser.add_argument("--check", action="store_true", help="verify without editing")
    args = parser.parse_args()

    version = version_from_tag(args.tag)
    cargo_toml = Path("Cargo.toml")
    current_text = cargo_toml.read_text()
    next_text = VERSION_PATTERN.sub(f'version = "{version}"', current_text, count=1)
    if current_text == next_text and f'version = "{version}"' not in current_text:
        raise SystemExit("could not find workspace package version in Cargo.toml")

    if args.check:
        package_version = cargo_package_version("arrowlint-python")
        if package_version != version:
            raise_version_mismatch(args.tag, package_version)
        print(f"tag={version} package={package_version}")
        return 0

    cargo_toml.write_text(next_text)
    subprocess.run(["cargo", "generate-lockfile"], check=True)
    package_version = cargo_package_version("arrowlint-python")
    if package_version != version:
        raise_version_mismatch(args.tag, package_version)
    print(f"tag={version} package={package_version}")
    return 0


def version_from_tag(tag: str) -> str:
    match = TAG_PATTERN.match(tag)
    if not match:
        raise SystemExit(f"release tag must look like v1.2.3: {tag}")
    return match.group("version")


def cargo_package_version(package_name: str) -> str:
    output = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        text=True,
    )
    import json

    metadata = json.loads(output)
    for package in metadata["packages"]:
        if package["name"] == package_name:
            return str(package["version"])
    raise SystemExit(f"could not find Cargo package {package_name}")


def raise_version_mismatch(tag: str, package_version: str) -> None:
    raise SystemExit(
        f"release tag {tag} does not match package version {package_version}"
    )


if __name__ == "__main__":
    raise SystemExit(main())
