//! expected-error: FFI type name `PushStr` is already registered

#[ffier::export]
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum PushStr {
    Value = 1,
}

ffier::library_definition!(
    "ui",
    library_tag = 1,
    enum PushStr,
    trait ffier::builtins::PushStr = 1,
);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
