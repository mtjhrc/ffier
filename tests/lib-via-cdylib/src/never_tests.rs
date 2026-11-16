use super::*;
use core::ffi::c_void;

struct Fails;

impl NeverCallback for Fails {
    fn run(&self) -> Result<ffier::Never, TestError> {
        Err(TestError::CustomMessage)
    }
}

#[test]
fn errors_cross_and_success_is_uninhabited() {
    let _: fn() -> ! = never_return;
    let _: fn(i32) -> ! = never_exit;

    init();
    let TestError::NumericError(data) = NeverRun::new().run(false).unwrap_err() else {
        panic!("unexpected error");
    };
    assert_eq!(data.field_0(), 73);
    assert!(matches!(
        run_never_callback(Fails),
        Err(TestError::CustomMessage)
    ));
}

unsafe extern "C" fn succeeds(_: *mut c_void, _: *mut *mut c_void) -> ffier::FfierResult {
    0
}

static SUCCEEDING: NeverCallbackVtable = NeverCallbackVtable {
    _num_slots: 1,
    _reserved1: 0,
    _reserved2: 0,
    drop: None,
    run: Some(succeeds),
};

/// A C callback that reports success, which a Rust implementation cannot.
fn succeeding_callback() -> *mut c_void {
    // The type tag of a handle the client built.
    let tag = unsafe {
        (*(Fails.__into_callback_handle().as_raw()
            as *const ffier::FfierHandle<ffier::VtableHandle>))
            .type_tag
    };
    unsafe {
        ft_vtable_handle_new(
            tag,
            &SUCCEEDING as *const NeverCallbackVtable as *const c_void,
            core::ptr::null(),
        )
    }
}

#[test]
fn child() {
    let Ok(case) = std::env::var("FFIER_NEVER_CHILD") else {
        return;
    };
    init();
    match case.as_str() {
        // Call the raw ABI so a generated client guard cannot mask a
        // missing abort in the native bridge.
        "return" => unsafe { ft_never_return() },
        "success" => unsafe {
            let mut err = core::ptr::null_mut();
            ft_never_run_run(ft_never_run_new(), true, &mut err);
        },
        "callback" => unsafe {
            let mut err = core::ptr::null_mut();
            ft_run_never_callback(succeeding_callback(), &mut err);
        },
        "exit" => never_exit(77),
        _ => panic!("unknown case: {case}"),
    }
    panic!("{case}: returned");
}

/// Contract violations that the library itself can commit: only while
/// `ffier::Never` is `()`.
#[rustversion::before(1.100)]
const NATIVE_CASES: &[&str] = &["return", "success"];
#[rustversion::since(1.100)]
const NATIVE_CASES: &[&str] = &[];

/// Breaking the contract aborts instead of returning to the caller.
#[test]
fn success_aborts() {
    for &case in NATIVE_CASES.iter().chain(&["callback", "exit"]) {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "never_tests::child"])
            .env("FFIER_NEVER_CHILD", case)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        if case == "exit" {
            assert_eq!(status.code(), Some(77));
            continue;
        }
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
