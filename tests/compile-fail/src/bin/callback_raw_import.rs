//! features: via-cdylib
//! expected-error: unsafe and requires unsafe block

#[path = "../callback.rs"]
mod callback;
use callback::{BorrowedFruit, api};
use ffier::ClientHandle;

type Tok = api::__FfierLocalToken;

fn main() {
    let local = 1;
    let retained = api::RetainedFruit::new_dynamic(BorrowedFruit(&local));
    let raw = ClientHandle::<Tok>::into_raw(retained);
    let _forged: api::RetainedFruit<'static> =
        <api::RetainedFruit<'static> as ClientHandle<Tok>>::from_raw(raw);
}
