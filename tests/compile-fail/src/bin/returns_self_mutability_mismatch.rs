//! expected-error: #[ffier(returns_self)] on `add` needs the return type `&mut Self` (or `Result<&mut Self, E>`), matching its receiver

pub struct Counter(i32);
#[ffier::export]
impl Counter {
    #[ffier(returns_self)]
    pub fn add(&mut self, n: i32) -> &Self {
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
