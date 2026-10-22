//! expected-error: the trait `FfiType` is not implemented for `Foo`

//! Forwarded metadata must not attach checked Send/Sync to root Foo's Rc.

pub struct Foo(pub std::rc::Rc<u64>);

pub mod other {
    pub struct Foo(pub u64);

    #[ffier::export(send, sync)]
    impl Foo {
        pub fn new(value: u64) -> Self {
            Self(value)
        }

        pub fn get(&self) -> u64 {
            self.0
        }
    }
}

// Forward checked type's metadata and helpers while root keeps unrelated Foo.
pub use other::{__ffier_meta_Foo, _ffier_foo};

ffier::library_definition!("identity_reexport_fail", library_tag = 9, Foo = 1);

ffier::generate_bridge!(
    local = __ffier_identity_reexport_fail_metadata,
    schema_output = "/dev/null"
);

// The root Foo must not have FFI impls from other::Foo's registration.
fn probe() {
    let _ = <Foo as FfiType>::C_TYPE_NAME;
}

fn main() {}
