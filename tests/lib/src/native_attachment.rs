// Rust-only callback trait with skipped methods.
//
// Fixture API for tests/consumer: a registered `no_vtable` callback trait whose
// methods are all `#[ffier(skip)]`, including a required `Box<Self>` receiver
// and a default body with a non-FFI-able return type. Native signatures and
// default bodies stay intact, and concrete dispatch still works through a
// different exported type's lifetime-parameterized method.

pub struct AttachContext {
    total: usize,
}

pub struct Requirements(Vec<usize>);

#[cfg_attr(feature = "ffi", ffier::export(no_vtable))]
pub trait AttachDevice<'a>: Send {
    #[cfg_attr(feature = "ffi", ffier(skip))]
    fn requirements(&self) -> Requirements {
        Requirements(vec![1, 2, 3])
    }

    #[cfg_attr(feature = "ffi", ffier(skip))]
    fn attach(self: Box<Self>, context: &mut AttachContext);
}

pub struct AttachItem<'a> {
    label: &'a str,
}

#[cfg_attr(feature = "ffi", ffier::export(send, invariant('a)))]
impl<'a> AttachItem<'a> {
    pub fn new(label: &'a str) -> Self {
        Self { label }
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl<'a> AttachDevice<'a> for AttachItem<'a> {
    #[cfg_attr(feature = "ffi", ffier(skip))]
    fn attach(self: Box<Self>, context: &mut AttachContext) {
        context.total += self.label.len();
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
pub fn attach_device<'a>(device: impl AttachDevice<'a> + 'a) -> usize {
    let mut context = AttachContext {
        total: device.requirements().0.iter().sum(),
    };
    Box::new(device).attach(&mut context);
    context.total
}
