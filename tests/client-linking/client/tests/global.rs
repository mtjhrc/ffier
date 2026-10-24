#![cfg(not(feature = "strong"))]
mod common;
use linking_client::{Symbol, answer, require, set_library_handle};

#[test]
fn first_probe_pins_the_default_process_namespace() {
    common::open_library(env!("FIXTURE_LIBRARY"), true);
    assert!(Symbol::FixtureAnswer.is_available());
    assert!(common::panic_message(answer).contains("call require() first"));
    let other = common::open_library(env!("FIXTURE_OTHER_LIBRARY"), false);
    assert!(unsafe { set_library_handle(Some(other), linking_client::LoadPolicy::Lazy) }.is_err());
    require(&[Symbol::FixtureAnswer]).unwrap();
    assert_eq!(answer(), 42);
}
