use ffier_rt::{BorrowedCallback, CallbackHandle, OwnedCallback};

#[test]
fn callback_capabilities_follow_ownership_and_borrow_kind() {
    fn send<T: Send>() {}
    fn sync<T: Sync>() {}

    send::<CallbackHandle<'_, (), OwnedCallback, true, false>>();
    sync::<CallbackHandle<'_, (), OwnedCallback, false, true>>();
    send::<CallbackHandle<'_, (), OwnedCallback, true, true>>();
    sync::<CallbackHandle<'_, (), OwnedCallback, true, true>>();
    send::<CallbackHandle<'_, (), BorrowedCallback<false>, false, true>>();
    sync::<CallbackHandle<'_, (), BorrowedCallback<false>, false, true>>();
    send::<CallbackHandle<'_, (), BorrowedCallback<true>, true, false>>();
    sync::<CallbackHandle<'_, (), BorrowedCallback<true>, false, true>>();
}
