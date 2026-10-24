#![cfg(not(feature = "strong"))]
mod common;
use linking_client::{Symbol, answer, set_library_handle};

#[test]
fn lazy_calls_load_from_the_configured_handle_and_report_missing_symbols() {
    let handle = common::open_library(env!("FIXTURE_LIBRARY"), false);
    unsafe { set_library_handle(Some(handle), linking_client::LoadPolicy::Lazy) }.unwrap();
    assert!(Symbol::FixtureAnswer.is_available());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| assert_eq!(answer(), 42));
        }
    });
    assert_eq!(linking_client::other(), 11);
    assert!(!Symbol::FixtureMissing.is_available());
    for _ in 0..2 {
        let message = common::panic_message(linking_client::missing);
        assert!(message.contains("failed to load symbol `fixture_missing`"));
        assert!(message.contains("fixture_missing"));
    }
    assert!(unsafe { set_library_handle(None, linking_client::LoadPolicy::Explicit) }.is_err());
    assert_eq!(answer(), 42);
    common::assert_failed_callback_allocation_drops_value();
    common::assert_failed_calls_preserve_ownership();
}
