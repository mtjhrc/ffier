//! expected-error: ffier: `Counter::add` returns a borrowed handle, which is not supported; return an owned handle or a value instead; add #[ffier(returns_self)] to discard a matching receiver reference

pub struct Counter(i32);
#[ffier::export]
impl Counter {
    #[cfg(all())]
    pub fn add(&mut self, n: i32) -> &mut Self {
        self.0 += n;
        self
    }
}
ffier::library_definition!("ui", library_tag = 1, Counter = 1);
ffier::generate_bridge!(
    local = __ffier_ui_metadata,
    schema_output = "/dev/null"
);
fn main() {}
