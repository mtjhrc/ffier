// This source is compiled unchanged against native Rust (with and without
// FFI annotations), strong generated bindings, and weak generated bindings.
use std::cell::Cell;
use std::rc::Rc;

use api::InputBackend as _;

struct FailingBackend(Rc<Cell<usize>>, Option<api::TestError>);

impl api::InputBackend for FailingBackend {
    fn current_ids(&self) -> Result<api::InputDeviceIds, api::TestError> {
        Err(api::TestError::CustomMessage)
    }

    fn peek_ids_result(&self) -> Result<&api::InputDeviceIds, api::TestError> {
        Err(api::TestError::CustomMessage)
    }

    fn edit_ids_result(&mut self) -> Result<&mut api::InputDeviceIds, api::TestError> {
        Err(api::TestError::CustomMessage)
    }

    fn next_event(&mut self, _fail: bool) -> Result<Option<api::InputEvent>, api::TestError> {
        Err(self
            .1
            .take()
            .unwrap_or_else(|| api::TestError::InvalidInput()))
    }

    fn fill_config<'a>(&self, _output: &'a mut [u8]) -> Result<&'a mut [u8], api::TestError> {
        Err(api::TestError::CustomMessage)
    }
}

impl Drop for FailingBackend {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn same_client_source_constructs_and_matches_fieldless_errors() {
    init();
    let drops = Rc::new(Cell::new(0));
    let mut backend = FailingBackend(drops.clone(), None);
    let error = backend.next_event(true).unwrap_err();
    assert!(matches!(error, api::TestError::InvalidInput()));
    assert_eq!(error.to_string(), "invalid input");
    let error = backend.current_ids().unwrap_err();
    assert!(matches!(error, api::TestError::CustomMessage));
    assert_eq!(error.to_string(), "custom error message");
    let error = api::roundtrip_input_backend_error(backend).unwrap_err();
    assert!(matches!(error, api::TestError::InvalidInput()));
    assert_eq!(drops.get(), 1);
}

