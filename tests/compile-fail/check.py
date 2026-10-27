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


def extract_feature_modes(fixture: Path) -> list[str]:
    """Feature list declared with `//! features: a b`; empty means no features."""
    prefix = "//! features:"
    modes = [
        line.removeprefix(prefix).strip().split()
        for line in fixture.read_text().splitlines()
        if line.startswith(prefix)
    ]
    if len(modes) > 1:
        raise ValueError(f"{fixture.name}: at most one {prefix} comment is allowed")
    return modes[0] if modes else []


def main() -> int:
    fixture_dir = Path(__file__).resolve().parent
    fixtures = sorted((fixture_dir / "src" / "bin").glob("*.rs"))
    if not fixtures:
        print("compile-fail: no fixtures found", file=sys.stderr)
        return 1
    for fixture in fixtures:
        case_name = fixture.stem
        try:
            expected = extract_expected_error(fixture)
            modes = extract_feature_modes(fixture)
        except ValueError as error:
            print(error, file=sys.stderr)
            return 1
        for mode in modes or [None]:
            command = [
                "cargo",
                "check",
                "--manifest-path",
                str(fixture_dir / "Cargo.toml"),
                "--target-dir",
                str(fixture_dir.parent.parent / "target"),
                "--bin",
                case_name,
            ]
            if mode is not None:
                command += ["--features", mode]
            result = subprocess.run(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
            )
            if result.returncode == 0:
                label = f"{case_name} ({mode})" if mode else case_name
                print(f"compile-fail/{label} unexpectedly compiled", file=sys.stderr)
                return 1
            if expected not in result.stdout:
                label = f"{case_name} ({mode})" if mode else case_name
                print(
                    f"compile-fail/{label} did not produce expected diagnostic: {expected}",
                    file=sys.stderr,
                )
                print(result.stdout, file=sys.stderr, end="")
                return 1
            if mode:
                print(f"compile-fail/{case_name} ({mode}) ... ok")
            else:
                print(f"compile-fail/{case_name} ... ok")

    return 0


if __name__ == "__main__":
    sys.exit(main())
