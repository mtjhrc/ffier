#!/usr/bin/env python3
"""Render diffs of ffier-generated test artifacts between a base ref and HEAD.

Generated artifacts (client bindings, compat bindings, C header) are no
longer committed; they are derived at build time from the test libraries'
JSON schemas. This script regenerates them for a base ref and for the
current working tree, then renders a unified diff for each artifact as
GitHub-flavored markdown.

Output goes to stdout. When GITHUB_STEP_SUMMARY is set (GitHub Actions),
the markdown is also appended to the job summary.

Usage:
    python3 render-codegen-diff.py [base-ref]

The base ref defaults to origin/main. The script builds the test libraries
and generators in both checkouts, so it takes a few minutes to run.
"""

import argparse
import difflib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Maximum diff lines rendered per artifact; larger diffs are truncated.
MAX_DIFF_LINES = 500

# (title, schema file, kind) — kind is "rust-client", "rust-client-v1",
# or "c-header".
ARTIFACTS = [
    ("lib-via-cdylib client bindings", "ffier-ft.json", "rust-client"),
    ("foreign-lib client bindings", "ffier-fl.json", "rust-client"),
    ("compat v1 client bindings", "ffier-ft.json", "rust-client-v1"),
    ("C header", "ffier-ft.json", "c-header"),
]


def run(cmd, cwd=None):
    subprocess.run(
        cmd, cwd=cwd, check=True, stdout=subprocess.DEVNULL, stderr=None
    )


def build_artifacts(repo: Path, out_dir: Path) -> None:
    """Build test libs + generators in `repo`, then generate all artifacts."""
    run(["cargo", "rustc", "-p", "ffier-test-foreign-lib", "--crate-type", "cdylib"], cwd=repo)
    run(["cargo", "rustc", "-p", "ffier-test-lib", "--crate-type", "cdylib"], cwd=repo)
    run(["cargo", "build", "-p", "ffier-gen-rust-client", "-p", "ffier-gen-c-header"], cwd=repo)

    gen_rust = repo / "target" / "debug" / "ffier-gen-rust-client"
    gen_header = repo / "target" / "debug" / "ffier-gen-c-header"

    for title, schema_name, kind in ARTIFACTS:
        schema = repo / "target" / schema_name
        if not schema.exists():
            sys.exit(f"error: {schema} missing after build")
        if kind in ("rust-client", "rust-client-v1"):
            data = json.loads(schema.read_text())
            if kind == "rust-client-v1":
                remove_fruit_label(data)
            # Temp files live inside the per-run tempdir so nothing leaks on
            # subprocess failure. rustfmt reads the extension to guess the
            # edition, hence the explicit suffixes.
            v1_schema = out_dir / "input-schema.json"
            v1_schema.write_text(json.dumps(data))
            src = subprocess.run(
                [str(gen_rust), str(v1_schema)],
                check=True,
                capture_output=True,
                text=True,
            ).stdout
            raw = out_dir / "input-raw.rs"
            raw.write_text(src)
            fmt = subprocess.run(
                ["rustfmt", "--emit", "stdout", str(raw)],
                check=True,
                capture_output=True,
                text=True,
            ).stdout
            # rustfmt prefixes output with "<filename>:" on its first line;
            # strip it so the temp path never reaches the diff.
            prefix = str(raw) + ":\n"
            if fmt.startswith(prefix):
                fmt = fmt[len(prefix):]
            (out_dir / slug(title)).write_text(fmt)
        else:
            src = subprocess.run(
                [str(gen_header), str(schema), "FFIER_FT_H"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout
            (out_dir / slug(title)).write_text(src)


def remove_fruit_label(schema: dict) -> None:
    """Mirror ffier_test_codegen::remove_fruit_label."""
    for trait in schema.get("traits", []):
        if trait["name"] == "Fruit":
            trait["methods"] = [m for m in trait["methods"] if m["name"] != "label"]
            trait["own_method_count"] = len(trait["methods"])


def slug(title: str) -> str:
    return title.replace(" ", "_")


def render_diffs(base_dir: Path, head_dir: Path) -> str:
    parts = ["## Generated code diff", ""]
    parts += [
        "Generated test artifacts are derived at build time, not committed.",
        "This is what the generated output change looks like.",
        "",
    ]
    any_change = False
    for title, _, _ in ARTIFACTS:
        name = slug(title)
        base = (base_dir / name).read_text().splitlines(keepends=True)
        head = (head_dir / name).read_text().splitlines(keepends=True)
        diff = list(
            difflib.unified_diff(
                base, head, fromfile=f"a/{name}", tofile=f"b/{name}", n=3
            )
        )
        if not diff:
            continue
        any_change = True
        changed = sum(1 for line in diff if line.startswith(("+", "-")) and not line.startswith(("+++", "---")))
        parts.append(f"<details><summary><code>{title}</code> — {changed} lines changed</summary>")
        parts.append("")
        parts.append("```diff")
        shown = diff[:MAX_DIFF_LINES]
        parts.extend(line.rstrip("\n") for line in shown)
        if len(diff) > MAX_DIFF_LINES:
            parts.append(f"... diff truncated after {MAX_DIFF_LINES} lines ({len(diff)} total)")
        parts.append("```")
        parts.append("</details>")
        parts.append("")
    if not any_change:
        parts.append("No changes in generated output.")
    return "\n".join(parts) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("base", nargs="?", default="origin/main", help="base ref (default origin/main)")
    args = parser.parse_args()

    if args.base.startswith("origin/"):
        branch = args.base[len("origin/"):]
        run(["git", "fetch", "origin", branch], cwd=REPO_ROOT)

    with tempfile.TemporaryDirectory(prefix="ffier-codegen-diff-") as tmp:
        tmp = Path(tmp)
        base_checkout = tmp / "base"
        run(["git", "worktree", "add", "--detach", str(base_checkout), args.base], cwd=REPO_ROOT)
        try:
            base_dir = tmp / "base-artifacts"
            head_dir = tmp / "head-artifacts"
            base_dir.mkdir()
            head_dir.mkdir()
            build_artifacts(base_checkout, base_dir)
            build_artifacts(REPO_ROOT, head_dir)
            markdown = render_diffs(base_dir, head_dir)
        finally:
            run(["git", "worktree", "remove", "--force", str(base_checkout)], cwd=REPO_ROOT)

    print(markdown)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(markdown)


if __name__ == "__main__":
    main()
