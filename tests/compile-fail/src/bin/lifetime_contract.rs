#[cfg(feature = "thread_method_bounds")]
#[ffier::export]
pub fn retain<'a: 'static>(_bytes: &'a [u8]) {}

#[cfg(feature = "thread_slice_lifetime")]
#[ffier::export]
pub fn hold<'a>(_data: &'a [u32]) {}

#[cfg(feature = "thread_typed_receiver")]
pub struct TypedReceiver;
#[cfg(feature = "thread_typed_receiver")]
#[ffier::export]
impl TypedReceiver {
    pub fn bytes<'a>(self: &'a Self) -> &'a [u8] {
        &[]
    }
}

#[cfg(feature = "thread_inner_slice_lifetime")]
#[ffier::export]
pub fn hold_text<'a>(_data: &[&'a str]) {}

#[cfg(feature = "thread_raw_lifetime_bounds")]
#[ffier::export]
pub trait RawLifetime {
    #[ffier(index = 0, raw_handle)]
    fn retain<'a>(_data: &'a [u8])
    where
        'a: 'static,
        Self: Sized;
}

fn main() {}

#[cfg(feature = "thread_static_receiver")]
#[ffier::export]
pub trait StaticReceiver {
    #[ffier(index = 0)]
    fn reference(&'static self) -> &'static str;
}

#[cfg(feature = "thread_static_mut_receiver")]
#[ffier::export]
pub trait StaticMutReceiver {
    #[ffier(index = 0)]
    fn reference(&'static mut self) -> &'static str;
}
