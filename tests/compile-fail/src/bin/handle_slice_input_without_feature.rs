//! expected-error: ffier handle slices require the `unstable_unsafe_handle_slices` feature

pub struct Widget;

#[ffier::export]
pub fn consume(widgets: &[Widget]) {
    let _ = widgets;
}

fn main() {}
