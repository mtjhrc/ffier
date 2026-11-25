//! expected-error: ffier: `find` returns the borrowed handle `&'static Gadget`, which is not supported

#[derive(Debug, ffier::FfiError)]
#[non_exhaustive]
pub enum UiError {
    #[ffier(code = 1)]
    Missing(),
}

impl std::fmt::Display for UiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("missing")
    }
}

impl std::error::Error for UiError {}

pub struct Gadget;
#[ffier::export]
impl Gadget {}

static GADGET: Gadget = Gadget;

#[ffier::export]
pub fn find() -> Result<&'static Gadget, UiError> {
    Ok(&GADGET)
}
ffier::library_definition!("ui", library_tag = 1, UiError = 1, Gadget = 2, fn find);
ffier::generate_bridge!(
    local = __ffier_ui_metadata,
    schema_output = "/dev/null"
);
fn main() {}
