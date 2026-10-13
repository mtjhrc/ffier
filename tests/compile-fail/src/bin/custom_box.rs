//! expected-error: FFI type name `Box` is reserved

pub mod custom {
    pub struct Box<T: ?Sized>(pub std::boxed::Box<T>);
}

#[ffier::export]
pub fn owned_text() -> std::result::Result<core::option::Option<custom::Box<str>>, u8> {
    Ok(Some(custom::Box("custom".into())))
}

ffier::library_definition!("ui", library_tag = 1, fn owned_text);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
