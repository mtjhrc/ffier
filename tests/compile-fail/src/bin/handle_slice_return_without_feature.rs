//! expected-error: ffier handle slices require the `unstable_unsafe_handle_slices` feature

pub struct Widget;

#[ffier::export]
pub trait Widgets {
    #[ffier(index = 0)]
    fn items(&self) -> &[Widget];
}

fn main() {}
