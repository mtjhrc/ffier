//! expected-error: #[ffier(returns_self)] on `duplicate` needs the return type `&Self` (or `Result<&Self, E>`), matching its receiver

pub struct Counter(i32);
#[ffier::export]
impl Counter {
    #[ffier(returns_self)]
    pub fn duplicate(&self) -> Self {
        Counter(self.0)
    }
}
ffier::library_definition!("ui", library_tag = 1, Counter = 1);
ffier::generate_bridge!(
    local = __ffier_ui_metadata,
    schema_output = "/dev/null"
);
fn main() {}
