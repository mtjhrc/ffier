//! expected-error: #[ffier(returns_self)] on `add` needs a `&self` or `&mut self` receiver; a by-value `self -> Self` already replaces the receiver

pub struct Counter(i32);
#[ffier::export]
impl Counter {
    #[ffier(returns_self)]
    pub fn add(self, n: i32) -> Self {
        Counter(self.0 + n)
    }
}
ffier::library_definition!("ui", library_tag = 1, Counter = 1);
ffier::generate_bridge!(
    local = __ffier_ui_metadata,
    schema_output = "/dev/null"
);
fn main() {}
