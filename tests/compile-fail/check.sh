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
check_failure opaque_lifetime_variance 'lifetime may not live long enough' via-cdylib
check_failure error_raw_import 'unsafe and requires unsafe block' via-cdylib

for case_name in callback_raw_transfer callback_raw_import; do
    check_failure "$case_name" 'unsafe and requires unsafe block' via-cdylib
done

for case_name in callback_owned_factory_borrow callback_mutable_factory_shared callback_factory_wrong_trait; do
    check_failure "$case_name" 'mismatched types' via-cdylib
done

# One fixture selects each independent guarantee through Cargo features.
for mode in native via-cdylib; do
    for case_name in thread_not_send thread_not_sync thread_not_send_sync; do
        check_failure thread_safety 'between threads safely' "$mode,$case_name"
    done
    for case_name in thread_not_static thread_worker_escape; do
        check_failure thread_safety 'does not live long enough' "$mode,$case_name"
    done
    check_failure thread_safety 'cannot be sent between threads safely' "$mode,thread_vtable_not_send"
    check_failure thread_safety 'cannot be shared between threads safely' "$mode,thread_vtable_not_sync"
    check_failure thread_safety 'cannot be shared between threads safely' "$mode,thread_send_vtable_not_sync"
    check_failure thread_safety 'cannot be sent between threads safely' "$mode,thread_sync_vtable_not_send"
done

for case_name in thread_handle_not_send thread_sync_handle_not_send thread_callback_not_send thread_sync_callback_not_send thread_shared_send_not_send thread_raw_vtable_not_send; do
    check_failure thread_safety 'cannot be sent between threads safely' "via-cdylib,$case_name"
done
for case_name in thread_handle_not_sync thread_send_handle_not_sync thread_callback_not_sync thread_send_callback_not_sync thread_raw_vtable_not_sync; do
    check_failure thread_safety 'cannot be shared between threads safely' "via-cdylib,$case_name"
done
check_failure thread_safety 'incompatible type for trait' via-cdylib,thread_factory_wrong_guarantees

while IFS='|' read -r case_name expected; do
    check_failure thread_contract "$expected" "$case_name"
done <<'CASES'
thread_false_send|cannot be sent between threads safely
thread_false_sync|cannot be shared between threads safely
thread_constrained_lifetime|ffier send/sync annotations require an inherent impl with only unconstrained lifetime parameters
thread_specialized_lifetime|ffier send/sync annotations require exporting the type for all its lifetime parameters
thread_stronger_bound|stronger use-site Send/Sync bounds for callback
thread_shared_send|requires Sync on Send trait
thread_supertrait|ffier thread-safe traits support only explicit Send/Sync supertraits
thread_where_bound|ffier exported trait where clauses are unsupported
thread_missing_annotation|requires #[ffier::export(send)] on its inherent impl
CASES

check_failure lifetime_contract 'ffier exported method lifetime bounds and where clauses are unsupported' thread_method_bounds
check_failure lifetime_contract 'ffier explicit receiver lifetimes are unsupported' thread_receiver_lifetime
check_failure lifetime_contract 'ffier typed self parameters are unsupported' thread_typed_receiver
check_failure lifetime_contract 'ffier explicit lifetimes on non-byte slice parameters are unsupported' thread_slice_lifetime
check_failure lifetime_contract 'ffier explicit lifetimes on non-byte slice parameters are unsupported' thread_inner_slice_lifetime
check_failure lifetime_contract 'ffier exported method lifetime bounds and where clauses are unsupported' thread_raw_lifetime_bounds
check_failure thread_contract 'requires a registered exported trait definition' thread_missing_definition
