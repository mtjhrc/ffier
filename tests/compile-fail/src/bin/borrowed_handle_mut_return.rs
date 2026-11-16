//! expected-error: ffier: `Owner::gadget_mut` returns the borrowed handle `&mut Gadget`, which is not supported

pub struct Gadget;
#[ffier::export]
impl Gadget {}

pub struct Owner(Gadget);
#[ffier::export]
impl Owner {
    pub fn gadget_mut(&mut self) -> &mut Gadget {
        &mut self.0
    }
}
ffier::library_definition!("ui", library_tag = 1, Gadget = 1, Owner = 2);
ffier::generate_bridge!(
    local = __ffier_ui_metadata,
    schema_output = "/dev/null"
);
fn main() {}
