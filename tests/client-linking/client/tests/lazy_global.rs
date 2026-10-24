#![cfg(not(feature = "strong"))]
mod common;
use linking_client::{answer, set_library_handle};

#[test]
fn lazy_calls_can_use_the_process_namespace() {
    common::open_library(env!("FIXTURE_LIBRARY"), true);
    unsafe { set_library_handle(None, linking_client::LoadPolicy::Lazy) }.unwrap();
    assert_eq!(answer(), 42);
    assert_eq!(linking_client::other(), 11);
}
