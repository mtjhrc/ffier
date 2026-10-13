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

#[cfg_attr(feature = "ffi", ffier::export(send))]
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

#[cfg(test)]
mod tests {
    #[cfg_attr(feature = "ffi", ffier::export(no_vtable))]
    trait NativeClone {
        #[cfg_attr(feature = "ffi", ffier(skip))]
        fn native_clone(&self) -> Self
        where
            Self: Sized;
    }

    impl NativeClone for usize {
        fn native_clone(&self) -> Self {
            *self
        }
    }

    #[test]
    fn skipped_sized_methods_keep_native_signatures() {
        assert_eq!(7usize.native_clone(), 7);
    }
}
