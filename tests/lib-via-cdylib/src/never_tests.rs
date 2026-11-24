use super::*;
use core::convert::Infallible;

struct Fails;

impl NeverCallback for Fails {
    fn run(&self) -> Result<Infallible, TestError> {
        Err(TestError::CustomMessage)
    }
}

#[test]
fn errors_cross_and_success_is_uninhabited() {
    let _: fn(&NeverRun, i32) -> ! = NeverRun::exit;
    let _: fn(i32) -> ! = never_exit;
    let _: fn(NeverRun) -> Result<Infallible, TestError> = NeverRun::run;

    init();
    let TestError::NumericError(data) = NeverRun::new().run().unwrap_err() else {
        panic!("unexpected error");
    };
    assert_eq!(data.field_0(), 73);
    assert!(matches!(
        run_never_callback(Fails),
        Err(TestError::CustomMessage)
    ));
}
