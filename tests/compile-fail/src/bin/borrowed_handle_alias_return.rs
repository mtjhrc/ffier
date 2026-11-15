//! expected-error: ffier: `Owner::gadget` returns a borrowed handle, which is not supported

pub struct Gadget;
#[ffier::export]
impl Gadget {}

pub type GadgetRef = &'static Gadget;

static GADGET: Gadget = Gadget;

pub struct Owner;
#[ffier::export]
impl Owner {
    pub fn gadget(&self) -> GadgetRef {
        &GADGET
    }
}
ffier::library_definition!("ui", library_tag = 1, Gadget = 1, Owner = 2);
ffier::generate_bridge!(
    local = __ffier_ui_metadata,
    schema_output = "/dev/null"
);
fn main() {}
