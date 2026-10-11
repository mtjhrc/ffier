#!/usr/bin/env bash
set -euo pipefail

manifest="$(dirname "$0")/Cargo.toml"
target_dir="$(dirname "$0")/../../target"

while IFS='|' read -r case_name expected; do
    output="$(cargo check --manifest-path "$manifest" --target-dir "$target_dir" --bin "$case_name" 2>&1)" && {
        printf 'compile-fail/%s unexpectedly compiled\n' "$case_name" >&2
        exit 1
    }
    if ! grep -Fq -- "$expected" <<<"$output"; then
        printf 'compile-fail/%s did not produce expected diagnostic: %s\n' "$case_name" "$expected" >&2
        printf '%s\n' "$output" >&2
        exit 1
    fi
    printf 'compile-fail/%s ... ok\n' "$case_name"
done <<'CASES'
CASES
