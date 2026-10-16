//! expected-error: FFI type name `FfierResult` is already registered

#[ffier::export]
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum FfierResult {
    Value = 1,
}

ffier::library_definition!("ui", library_tag = 1, enum FfierResult);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
