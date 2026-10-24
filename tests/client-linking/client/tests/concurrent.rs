#![cfg(not(feature = "strong"))]
mod common;

#[test]
fn racing_setters_choose_one_source_for_all_symbols() {
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads = [env!("FIXTURE_LIBRARY"), env!("FIXTURE_OTHER_LIBRARY")].map(|path| {
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let handle = common::open_library(path, false);
            barrier.wait();
            unsafe {
                linking_client::set_library_handle(Some(handle), linking_client::LoadPolicy::Lazy)
            }
            .is_ok()
        })
    });
    let [first, second] = threads.map(|thread| thread.join().unwrap());
    assert_ne!(first, second);
    assert_eq!(linking_client::answer(), if first { 42 } else { 99 });
    assert_eq!(linking_client::other(), if first { 11 } else { 77 });
}
