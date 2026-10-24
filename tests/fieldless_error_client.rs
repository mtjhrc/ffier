// Compile unchanged against native Rust and generated strong/dynlookup bindings.
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use api::{ErrorForwarder, Gadget, TestError};

struct ForwardingError {
    error: RefCell<Option<TestError>>,
    drops: Rc<Cell<usize>>,
}

impl ForwardingError {
    fn take_error(&self) -> TestError {
        self.error
            .borrow_mut()
            .take()
            .expect("callback called once")
    }
}

impl ErrorForwarder for ForwardingError {
    fn forward_value(&self) -> Result<i32, TestError> {
        Err(self.take_error())
    }

    fn forward_handle(&self) -> Result<Gadget, TestError> {
        Err(self.take_error())
    }

    fn forward_unit(&self) -> Result<(), TestError> {
        Err(self.take_error())
    }
}

impl Drop for ForwardingError {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn same_client_source_constructs_and_returns_fieldless_errors() {
    init();
    for method in 0..3 {
        for error in [TestError::CustomMessage, TestError::InvalidInput()] {
            let custom = matches!(error, TestError::CustomMessage);
            let drops = Rc::new(Cell::new(0));
            let callback = ForwardingError {
                error: RefCell::new(Some(error)),
                drops: drops.clone(),
            };
            let error = api::roundtrip_callback_error(callback, method).unwrap_err();
            if custom {
                assert!(matches!(error, TestError::CustomMessage));
                assert_eq!(error.to_string(), "custom error message");
            } else {
                assert!(matches!(error, TestError::InvalidInput()));
                assert_eq!(error.to_string(), "invalid input");
            }
            assert_eq!(drops.get(), 1);
        }
    }
}
