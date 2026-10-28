use ffier_gen_rust_client::generate;
use ffier_schema::Library;
use serde_json::{Value, json};

fn method(name: &str, receiver: &str, ret: Value, slot: Option<usize>) -> Value {
    let mut method = json!({
        "name": name, "ffi_name": format!("never_vmm_{name}"),
        "receiver": receiver, "params": [], "ret": ret
    });
    if let Some(index) = slot {
        method["ffi_name"] = json!(format!("never_runner_{name}"));
        method["trait_definition"] = json!({"index": index, "has_default": true});
    }
    method
}

fn bindings() -> String {
    let error_only = json!({"return_kind": "result_never", "err_type": "RunError"});
    let never = json!({"return_kind": "never"});
    let schema: Library = serde_json::from_value(json!({
        "prefix": "never",
        "type_registry": {
            "Vmm": {"kind": {"handle": {"c_name": "NeverVmm"}}, "type_tag": 1},
            "Runner": {"kind": {"trait": {"c_name": "NeverRunner"}}, "type_tag": 2}
        },
        "exported_types": [{
            "name": "Vmm", "destroy_ffi_name": "never_vmm_destroy", "is_builder_type": false,
            "methods": [
                method("run", "value", error_only.clone(), None),
                method("stop", "ref", never.clone(), None)
            ]
        }],
        "errors": [], "trait_impls": [],
        "traits": [{
            "name": "Runner", "destroy_ffi_name": "never_runner_destroy",
            "type_tag_constant": "NEVER_RUNNER_TYPE_TAG",
            "vtable_struct_c_name": "NeverRunnerVtable", "wrapper_c_name": "NeverVtableRunner",
            "vtable_struct_name": "RunnerVtable", "wrapper_name": "VtableRunner",
            "own_method_count": 2, "num_slots": 2,
            "methods": [
                method("run", "mut", error_only.clone(), Some(0)),
                method("stop", "ref", never.clone(), Some(1))
            ]
        }],
        "free_functions": [
            {"name": "run_guest", "ffi_name": "never_run_guest", "params": [], "ret": error_only},
            {"name": "stop_guest", "ffi_name": "never_stop_guest", "params": [], "ret": never}
        ]
    }))
    .unwrap();
    generate(&schema)
        .split_whitespace()
        .collect::<String>()
        .replace(",)", ")")
        .replace(",>", ">")
}

#[test]
fn error_only_bindings_have_an_uninhabited_success_type_and_no_success_buffer() {
    let source = bindings();
    for signature in [
        "pubfnrun(self)->Result<core::convert::Infallible,RunError>",
        "pubfnrun_guest()->Result<core::convert::Infallible,RunError>",
        "fnrun(&mutself)->Result<core::convert::Infallible,RunError>",
        "fnnever_run_guest(err_out:*mut*mutcore::ffi::c_void)->ffier::FfierResult",
        "pubrun:Option<unsafeextern\"C\"fn(*mutcore::ffi::c_void,*mut*mutcore::ffi::c_void)->ffier::FfierResult>",
    ] {
        assert!(source.contains(signature), "missing {signature}");
    }
    assert!(!source.contains("MaybeUninit"));
    assert!(!source.contains("unreachable_unchecked"));
    // Foreign success must terminate before any attempt to reconstruct an error.
    assert!(
        source
            .contains("if__r==0{std::process::abort()}Err(unsafe{RunError::from_ffi(__r,__err)})")
    );
    // Rust-implemented callbacks eliminate the impossible success and retain
    // both the status-only and owned-error output paths.
    assert!(source.contains("Ok(__never)=>match__never{}"));
    assert!(source.contains("iferr_out.is_null(){__e.ffi_result_of()}else{"));
    assert!(source.contains("let(__code,__handle)=__e.into_ffi();"));
}

#[test]
fn direct_never_bindings_keep_a_checked_void_abi() {
    let source = bindings();
    assert!(source.contains("pubfnstop_guest()->!"));
    assert!(source.contains("pubfnstop(&self)->!"));
    assert!(source.contains("fnnever_stop_guest();"));
    assert!(source.contains("unsafe{never_stop_guest()};std::process::abort()"));
    assert!(source.contains("pubstop:Option<unsafeextern\"C\"fn(*mutcore::ffi::c_void)>"));
    assert!(!source.contains("unsafeextern\"C\"fn(*mutcore::ffi::c_void)->!"));
}
