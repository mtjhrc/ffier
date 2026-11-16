//! expected-error: ffier: `Counter::add` returns the borrowed handle `&mut Counter`, which is not supported; return an owned handle or a value instead; if it returns its receiver for chaining, mark it #[ffier(returns_self)]

pub struct Counter(i32);
#[ffier::export]
impl Counter {
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
