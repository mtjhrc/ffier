#!/usr/bin/env python3

from pathlib import Path
import subprocess
import sys


def extract_expected_error(fixture: Path) -> str:
    prefix = "//! expected-error:"
    diagnostics = [
        line.removeprefix(prefix).strip()
        for line in fixture.read_text().splitlines()
        if line.startswith(prefix)
    ]
    if len(diagnostics) != 1 or not diagnostics[0]:
        raise ValueError(f"{fixture.name}: require one non-empty {prefix} comment")
    return diagnostics[0]


def main() -> int:
    fixture_dir = Path(__file__).resolve().parent
    for fixture in sorted((fixture_dir / "src" / "bin").glob("*.rs")):
        case_name = fixture.stem
        try:
            expected = extract_expected_error(fixture)
        except ValueError as error:
            print(error, file=sys.stderr)
            return 1
        result = subprocess.run(
            [
                "cargo",
                "check",
                "--manifest-path",
                str(fixture_dir / "Cargo.toml"),
                "--target-dir",
                str(fixture_dir.parent.parent / "target"),
                "--bin",
                case_name,
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        if result.returncode == 0:
            print(f"compile-fail/{case_name} unexpectedly compiled", file=sys.stderr)
            return 1
        if expected not in result.stdout:
            print(
                f"compile-fail/{case_name} did not produce expected diagnostic: {expected}",
                file=sys.stderr,
            )
            print(result.stdout, file=sys.stderr, end="")
            return 1
        print(f"compile-fail/{case_name} ... ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
