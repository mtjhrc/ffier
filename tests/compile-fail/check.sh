#!/usr/bin/env bash
set -euo pipefail

manifest="$(dirname "$0")/Cargo.toml"
target_dir="$(dirname "$0")/../../target"

check_failure() {
    local case_name="$1" expected="$2" mode="${3:-}" output
    local features=()
    if [[ -n "$mode" ]]; then
        features=(--features "$mode")
    fi
    output="$(cargo check --manifest-path "$manifest" --target-dir "$target_dir" --bin "$case_name" "${features[@]}" 2>&1)" && {
        printf 'compile-fail/%s%s unexpectedly compiled\n' "$case_name" "${mode:+ ($mode)}" >&2
        exit 1
    }
    if ! grep -Fq -- "$expected" <<<"$output"; then
        printf 'compile-fail/%s did not produce expected diagnostic: %s\n' "$case_name" "$expected" >&2
        printf '%s\n' "$output" >&2
        exit 1
    fi
    printf 'compile-fail/%s%s ... ok\n' "$case_name" "${mode:+ ($mode)}"
}

while IFS='|' read -r case_name expected; do
    check_failure "$case_name" "$expected"
done <<'CASES'
missing_repr|ffier value structs require #[repr(C)]
private_field|ffier value-struct fields must be public
cfg_field|field-level cfg on ffier value structs is not supported yet
raw_field|ffier value-struct fields cannot use raw identifiers
unsupported_field|unsupported type `String`
unregistered_nested|unsupported type `Inner`
value_slice|value-struct slices are not supported
optional_param|Option<ValueStruct> is supported only in return positions
optional_field|value-struct fields cannot be optional
nested_option|nested Option<Option<ValueStruct>> is not supported
packed|support only #[repr(C)]
aligned|support only #[repr(C)]
callback_extra_bound|ffier impl Trait supports one trait bound and lifetime bounds
callback_higher_ranked|ffier impl Trait supports one trait bound and lifetime bounds
callback_reference_lifetime|callback handles are call-scoped
callback_trait_lifetime|ffier callback vtables with trait lifetime or type parameters are unsupported
callback_method_lifetime_bounds|ffier callback lifetime bounds on generic parameters and where clauses are unsupported
callback_shared_mutable|shared vtable dispatch for param
callback_borrowed_return|ffier borrowed callbacks with lifetime-bearing returns are unsupported
CASES

for mode in native via-cdylib; do
    for case_name in static_callback retained_callback_escape borrowed_callback_escape; do
        check_failure "$case_name" 'does not live long enough' "$mode"
    done
done

check_failure callback_handle_escape 'does not live long enough' via-cdylib

for case_name in callback_raw_transfer callback_raw_import; do
    check_failure "$case_name" 'unsafe and requires unsafe block' via-cdylib
done

for case_name in callback_owned_factory_borrow callback_mutable_factory_shared callback_factory_wrong_trait; do
    check_failure "$case_name" 'mismatched types' via-cdylib
done

check_failure opaque_lifetime_variance 'lifetime may not live long enough' via-cdylib
check_failure lifetime_contract 'ffier exported method lifetime bounds and where clauses are unsupported' thread_method_bounds
check_failure lifetime_contract 'ffier explicit receiver lifetimes are unsupported' thread_receiver_lifetime
check_failure lifetime_contract 'ffier typed self parameters are unsupported' thread_typed_receiver
check_failure lifetime_contract 'ffier explicit lifetimes on non-byte slice parameters are unsupported' thread_slice_lifetime
check_failure lifetime_contract 'ffier explicit lifetimes on non-byte slice parameters are unsupported' thread_inner_slice_lifetime
check_failure lifetime_contract 'ffier exported method lifetime bounds and where clauses are unsupported' thread_raw_lifetime_bounds
