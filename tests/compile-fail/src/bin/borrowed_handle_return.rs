//! expected-error: ffier: `Owner::gadget` returns the borrowed handle `&Gadget`, which is not supported

pub struct Gadget;
#[ffier::export]
impl Gadget {}

pub struct Owner(Gadget);
#[ffier::export]
impl Owner {
    pub fn gadget(&self) -> &Gadget {
        &self.0
    }
}
ffier::library_definition!("ui", library_tag = 1, Gadget = 1, Owner = 2);
ffier::generate_bridge!(
    local = __ffier_ui_metadata,
    schema_output = "/dev/null"
);
fn main() {}
