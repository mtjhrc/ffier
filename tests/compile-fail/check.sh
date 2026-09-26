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
missing_repr|ffier value structs require #[repr(C)]
private_field|ffier value-struct fields must be public
unsupported_field|unsupported type `String`
unregistered_nested|unsupported type `Inner`
value_slice|value-struct slices are not supported
optional_param|Option<ValueStruct> is supported only in return positions
optional_field|value-struct fields cannot be optional
nested_option|nested Option<Option<ValueStruct>> is not supported
packed|support only #[repr(C)]
aligned|support only #[repr(C)]
CASES
