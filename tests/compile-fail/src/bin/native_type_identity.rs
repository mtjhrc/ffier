//! expected-error: the trait `FfiType` is not implemented for `Counter`

//! A same-named root type must not receive the qualified type's FFI impls.
//!
//! The qualified export checks a thread-safe type, not root Counter's Rc.

/// Root type sharing the exported type's name. No exported impl of its own.
pub struct Counter(pub std::rc::Rc<u64>);

pub mod other {
    pub struct Counter(pub u64);
}

#[ffier::export(send, sync)]
impl other::Counter {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn get(&self) -> u64 {
        self.0
    }
}

ffier::library_definition!("identity_fail", library_tag = 9, Counter = 1);

ffier::generate_bridge!(
    local = __ffier_identity_fail_metadata,
    schema_output = "/dev/null"
);

// The root Counter is a distinct native type: registration must not have
// implemented FfiType for it. (With bare-name registration this compiled,
// silently binding FFI plumbing to the wrong type.)
fn probe() {
    let _ = <Counter as FfiType>::C_TYPE_NAME;
}

fn main() {}
