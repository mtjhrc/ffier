use super::*;

unsafe extern "C" fn invalid_callback_success(
    _: *mut core::ffi::c_void,
    _: *const ffier::FfierBytes,
    _: usize,
    _: *mut *mut core::ffi::c_void,
) -> ffier::FfierResult {
    0
}

static INVALID_VTABLE: NeverCallbackVtable = NeverCallbackVtable {
    _num_slots: 2,
    _reserved1: 0,
    _reserved2: 0,
    drop: None,
    fail: Some(invalid_callback_success),
    fail_default: None,
};

#[test]
fn child() {
    let Ok(case) = std::env::var("FFIER_NEVER_CHILD") else {
        return;
    };
    init();
    match case.as_str() {
        "direct-return" => never_return(),
        "result-success" => {
            let _ = NeverRun::new().run(true);
        }
        "process-exit" => never_exit(77),
        "callback-success" => unsafe {
            // Library tag 1 and NeverCallback type tag 51 from the fixture.
            let callback = ft_vtable_handle_new(
                (1 << 24) | 51,
                &INVALID_VTABLE as *const _ as *const core::ffi::c_void,
                core::ptr::null(),
            );
            let mut error = core::ptr::null_mut();
            ft_run_never_callback(callback, false, &mut error);
        },
        _ => panic!("unknown never-return child case: {case}"),
    }
    panic!("never-return child unexpectedly returned");
}

#[test]
fn success_exits_or_aborts_but_never_returns_to_the_client() {
    for case in [
        "direct-return",
        "result-success",
        "callback-success",
        "process-exit",
    ] {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "never_tests::child"])
            .env("FFIER_NEVER_CHILD", case)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        if case == "process-exit" {
            assert_eq!(status.code(), Some(77));
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                assert_eq!(status.signal(), Some(6), "{case}: {status}");
            }
            #[cfg(windows)]
            assert_eq!(
                status.code().unwrap() as u32,
                0xc0000409,
                "{case}: {status}"
            );
        }
    }
}
