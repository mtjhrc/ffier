use linking_client::{LoadError, Symbol, answer, require};

#[cfg(not(feature = "strong"))]
mod common;

#[cfg(not(feature = "strong"))]
#[test]
fn explicit_loading_and_optional_discovery_use_one_library() {
    use linking_client::set_library_handle;
    use std::error::Error;

    let handle = common::open_library(env!("FIXTURE_LIBRARY"), false);
    unsafe { set_library_handle(Some(handle), linking_client::LoadPolicy::Explicit) }.unwrap();
    assert!(Symbol::FixtureAnswer.is_available());
    assert!(!Symbol::FixtureMissing.is_available());
    // Probes fix the source but do not initialize function pointer caches.
    assert!(
        common::panic_message(answer)
            .contains("symbol `fixture_answer` not loaded; call require() first")
    );
    let error: LoadError = require(&[Symbol::FixtureMissing]).unwrap_err();
    assert!(error.to_string().contains("fixture_missing"));
    assert!(error.source().is_some());
    let _: fn() -> i32 = linking_client::missing;
    require(&[Symbol::FixtureAnswer]).unwrap();
    assert_eq!(answer(), 42);

    let other_library = common::open_library(env!("FIXTURE_OTHER_LIBRARY"), false);
    assert!(
        unsafe { set_library_handle(Some(other_library), linking_client::LoadPolicy::Lazy) }
            .is_err()
    );
    assert!(
        unsafe { set_library_handle(Some(handle), linking_client::LoadPolicy::Explicit) }.is_err()
    );
    // Y exports the optional symbol, but all probes and subsequent loads stay on X.
    assert!(!Symbol::FixtureMissing.is_available());
    assert!(require(&[Symbol::FixtureMissing]).is_err());
    require(&[Symbol::FixtureOther]).unwrap();
    assert_eq!(linking_client::other(), 11);
    require(&[Symbol::FixtureAnswer]).unwrap();
    assert_eq!(answer(), 42);
    common::assert_failed_callback_allocation_drops_value();
    common::assert_failed_calls_preserve_ownership();
}

#[cfg(feature = "strong")]
#[test]
fn strong_calls_and_metadata_need_no_loader() {
    assert_eq!(answer(), 42);
    let ignored_handle = Some(core::ptr::NonNull::dangling());
    unsafe { linking_client::set_library_handle(ignored_handle, linking_client::LoadPolicy::Lazy) }
        .unwrap();
    unsafe { linking_client::set_library_handle(None, linking_client::LoadPolicy::Explicit) }
        .unwrap();
    let result: Result<(), LoadError> = require(&[Symbol::FixtureMissing]);
    result.unwrap();
    require(&[Symbol::FixtureAnswer]).unwrap();
    assert!(Symbol::FixtureMissing.is_available());
    assert!(Symbol::FixtureAnswer.is_available());
    assert_eq!(Symbol::FixtureMissing.c_name(), "fixture_missing");
    assert_eq!(answer(), 42);
}

#[test]
fn conditional_vtable_methods_preserve_reserved_slot_layout() {
    use linking_client::{Worker, WorkerVtable};
    struct Callback;
    impl Worker for Callback {
        #[cfg(any(not(feature = "strong"), feature = "optional-entry"))]
        fn newer(&self) -> i32 {
            7
        }
    }
    let vtable = Callback::__ffier_vtable();
    assert_eq!(vtable._num_slots, 1);
    assert_eq!(
        core::mem::size_of::<WorkerVtable>(),
        core::mem::size_of::<ffier::VtableHeader>() + 2 * core::mem::size_of::<usize>()
    );
    #[cfg(feature = "strong")]
    assert!(vtable.__reserved_0.is_none());
    #[cfg(not(feature = "strong"))]
    {
        let callback = Callback;
        assert_eq!(
            unsafe { vtable.newer.unwrap()(&callback as *const Callback as *mut _) },
            7
        );
        let _: fn() -> linking_client::Widget = linking_client::Widget::new;
        let _: fn() -> linking_client::OptionalWidget = linking_client::OptionalWidget::new;
        assert_eq!(
            core::mem::size_of::<linking_client::OptionalWorkerVtable>(),
            core::mem::size_of::<WorkerVtable>()
        );
    }
}
